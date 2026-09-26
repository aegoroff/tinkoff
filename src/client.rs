use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Utc};
use color_eyre::eyre::{self, WrapErr};
use iso_currency::Currency;
use itertools::Itertools;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex};
use t_invest_sdk::{
    TInvestInterceptor,
    api::{
        Account, AccountStatus, AccountType, Bond as ApiBond, CandleInterval,
        Currency as ApiCurrency, Dividend, Etf as ApiEtf, FindInstrumentRequest,
        Future as ApiFuture, GetAccountsRequest, GetAccountsResponse, GetAssetFundamentalsRequest,
        GetBondEventsRequest, GetCandlesRequest, GetDividendsRequest, GetForecastRequest,
        GetForecastResponse, GetOperationsByCursorRequest, GetOperationsByCursorResponse,
        HistoricCandle, Instrument as ApiInstrument, InstrumentIdType, InstrumentRequest,
        InstrumentShort, InstrumentStatus, InstrumentType, InstrumentsRequest, OperationItem,
        OperationState, OperationType, PortfolioPosition, PortfolioRequest, Quotation,
        Recommendation as ApiRecommendation, Share as ApiShare,
        get_asset_fundamentals_response::StatisticResponse, get_bond_events_request::EventType,
        get_bond_events_response::BondEvent as ApiBondEvent,
        instruments_service_client::InstrumentsServiceClient,
        market_data_service_client::MarketDataServiceClient,
        operations_service_client::OperationsServiceClient, portfolio_request::CurrencyRequest,
        users_service_client::UsersServiceClient,
    },
};
use tokio::sync::{OnceCell, Semaphore};
use tokio::task::JoinSet;
use tokio::time::{Duration, Instant, sleep};
use tonic::Code;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

/// Prod T-Invest API endpoint (official `tbank.ru` host).
const INVEST_API_ENDPOINT: &str = "https://invest-public-api.tbank.ru:443/";

/// Russian Trusted Root CA PEM (required for T-Bank TLS).
const RUSSIAN_TRUSTED_CAS: &[u8] = include_bytes!("../certs/russian_trusted_cas.pem");

use crate::{
    account_type_name,
    domain::{
        BondInfo, CouponCalendar, CouponPayment, CouponProfit, DividendCalendar, DividendPayment,
        DividendProfit, Figi, Instrument, LoadedPaper, Money, NoneProfit, Paper, Portfolio,
        Position, Ticker, Totals,
        analytics::{Analytics, Forecast, Fundamentals, Recommendation, ShareAnalytics},
        bond::{BondEvent, BondEventKind, bond_info, mark_amortizations},
        calendar::{Calendar, CalendarKind, CombinedCalendar},
        fx::{FxCandidate, FxInstrument, build_fx_map, quote_to_rub_rate},
        history::{History, HistoryItem},
        income::{DividendRecord, IncomeForecast, IncomeItem, IncomeKind, expected_dividends},
        tax::{TaxOperation, TaxOperationKind, TaxReport},
        xirr::CashFlow,
    },
    progress::Progress,
    to_currency, to_datetime_utc, to_decimal, to_metric, to_money, to_optional_datetime_utc,
};

/// How many days back to look for an FX candle; covers the New Year holidays.
const FX_LOOKBACK_DAYS: i64 = 14;

/// How far ahead bond events are requested; covers the longest bond maturities.
const BOND_EVENTS_HORIZON_DAYS: i64 = 50 * 365;

/// Passive income forecast horizon in days.
const INCOME_FORECAST_DAYS: u32 = 365;

/// Most assets accepted by a single fundamentals request.
const MAX_FUNDAMENTALS_ASSETS: usize = 100;

/// Operations requested per page, the maximum the API allows.
const OPERATIONS_PAGE_SIZE: i32 = 1000;

/// Maximum number of concurrent API requests when loading portfolio positions or calendars.
pub const MAX_CONCURRENT_REQUESTS: usize = 10;

/// Daily close rates to RUB by date.
type DailyRates = BTreeMap<NaiveDate, Decimal>;

/// Lazily loaded daily rates of one currency for one calendar year.
type YearRates = Arc<OnceCell<DailyRates>>;

/// FX instrument map + per-request cache of daily rates.
pub struct FxBook {
    instruments: HashMap<Currency, FxInstrument>,
    /// Rates loaded once per currency and calendar year; concurrent requests share one load.
    daily: Mutex<HashMap<(Currency, i32), YearRates>>,
}

/// How the account to work with is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountSelector {
    /// The only open account of this type.
    Type(AccountType),
    /// The account with this ID, whatever its type and status.
    Id(String),
}

/// Picks the account matching `selector`.
///
/// # Errors
///
/// Returns an error listing available accounts if no account matches, or if several
/// open accounts have the requested type and the choice would be ambiguous.
pub fn select_account<'a>(
    accounts: &'a [Account],
    selector: &AccountSelector,
) -> color_eyre::Result<&'a Account> {
    let matching: Vec<&Account> = match selector {
        AccountSelector::Id(id) => accounts.iter().filter(|a| &a.id == id).collect(),
        AccountSelector::Type(account_type) => accounts
            .iter()
            .filter(|a| a.r#type() == *account_type && a.status() == AccountStatus::Open)
            .collect(),
    };
    let wanted = match selector {
        AccountSelector::Id(id) => format!("account with ID {id}"),
        AccountSelector::Type(t) => format!("open {} account", account_type_name(*t)),
    };
    match matching.as_slice() {
        [account] => Ok(account),
        [] => Err(eyre::eyre!(
            "No {wanted}; available accounts: {}",
            describe_accounts(accounts.iter())
        )),
        several => Err(eyre::eyre!(
            "Several accounts match: {}; choose one with --account-id",
            describe_accounts(several.iter().copied())
        )),
    }
}

/// Formats accounts as `ID (name, type)` for error messages.
fn describe_accounts<'a>(accounts: impl Iterator<Item = &'a Account>) -> String {
    let described = accounts
        .map(|a| format!("{} ({}, {})", a.id, a.name, account_type_name(a.r#type())))
        .join(", ");
    if described.is_empty() {
        "none".to_string()
    } else {
        described
    }
}

#[derive(Default)]
pub struct AccountPortfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

#[derive(Clone)]
pub struct TinkoffInvestment {
    interceptor: TInvestInterceptor,
    /// Shared TLS channel; connected once, then cloned (cheap handle).
    channel: Arc<OnceCell<Channel>>,
}

enum OperationInfluence {
    /// Anything that affects to dividends or coupons value.<br/>
    /// Including negative values like dividend tax etc. to calculate pure income<br/>
    /// without taxes.
    PureIncome,
    /// Comissions and other losses
    Fees,
    Unspecified,
}

#[must_use]
fn to_influence(op: OperationType) -> OperationInfluence {
    match op {
        t_invest_sdk::api::OperationType::DividendTax
        | t_invest_sdk::api::OperationType::DividendTaxProgressive
        | t_invest_sdk::api::OperationType::BondTax
        | t_invest_sdk::api::OperationType::BondTaxProgressive
        | t_invest_sdk::api::OperationType::Coupon
        | t_invest_sdk::api::OperationType::BenefitTax
        | t_invest_sdk::api::OperationType::BenefitTaxProgressive
        | t_invest_sdk::api::OperationType::Overnight
        | t_invest_sdk::api::OperationType::Tax
        | t_invest_sdk::api::OperationType::Dividend => OperationInfluence::PureIncome,
        t_invest_sdk::api::OperationType::ServiceFee
        | t_invest_sdk::api::OperationType::MarginFee
        | t_invest_sdk::api::OperationType::BrokerFee
        | t_invest_sdk::api::OperationType::SuccessFee
        | t_invest_sdk::api::OperationType::TrackMfee
        | t_invest_sdk::api::OperationType::TrackPfee
        | t_invest_sdk::api::OperationType::CashFee
        | t_invest_sdk::api::OperationType::OutFee
        | t_invest_sdk::api::OperationType::OutStampDuty
        | t_invest_sdk::api::OperationType::AdviceFee
        | t_invest_sdk::api::OperationType::OutputPenalty => OperationInfluence::Fees,
        _ => OperationInfluence::Unspecified,
    }
}

/// Operation types that affect taxes: income and taxes on it, taxes and their corrections.
const TAX_OPERATION_TYPES: [OperationType; 20] = [
    OperationType::Dividend,
    OperationType::DivExt,
    OperationType::Coupon,
    OperationType::DividendTax,
    OperationType::DividendTaxProgressive,
    OperationType::BondTax,
    OperationType::BondTaxProgressive,
    OperationType::TaxCorrectionCoupon,
    OperationType::Tax,
    OperationType::TaxProgressive,
    OperationType::TaxCorrection,
    OperationType::TaxCorrectionProgressive,
    OperationType::BenefitTax,
    OperationType::BenefitTaxProgressive,
    OperationType::TaxRepo,
    OperationType::TaxRepoProgressive,
    OperationType::TaxRepoHold,
    OperationType::TaxRepoHoldProgressive,
    OperationType::TaxRepoRefund,
    OperationType::TaxRepoRefundProgressive,
];

/// Tax kind of an operation; `None` for operations that do not affect taxes.
fn tax_operation_kind(op: OperationType) -> Option<TaxOperationKind> {
    match op {
        OperationType::Dividend | OperationType::DivExt => Some(TaxOperationKind::Dividend),
        OperationType::Coupon => Some(TaxOperationKind::Coupon),
        OperationType::DividendTax | OperationType::DividendTaxProgressive => {
            Some(TaxOperationKind::DividendTax)
        }
        OperationType::BondTax
        | OperationType::BondTaxProgressive
        | OperationType::TaxCorrectionCoupon => Some(TaxOperationKind::CouponTax),
        OperationType::Tax
        | OperationType::TaxProgressive
        | OperationType::TaxCorrection
        | OperationType::TaxCorrectionProgressive
        | OperationType::BenefitTax
        | OperationType::BenefitTaxProgressive
        | OperationType::TaxRepo
        | OperationType::TaxRepoProgressive
        | OperationType::TaxRepoHold
        | OperationType::TaxRepoHoldProgressive
        | OperationType::TaxRepoRefund
        | OperationType::TaxRepoRefundProgressive => Some(TaxOperationKind::OtherTax),
        _ => None,
    }
}

impl TryFrom<&PortfolioPosition> for Position {
    type Error = color_eyre::eyre::Error;

    fn try_from(value: &PortfolioPosition) -> Result<Self, Self::Error> {
        let currency =
            to_currency(&value.current_price).ok_or(eyre::eyre!("Failed to get currency"))?;

        let average_buy_price = to_money(value.average_position_price.as_ref())
            .ok_or(eyre::eyre!("Failed to get average position price"))?;

        let quantity = to_decimal(value.quantity.as_ref());

        let current_instrument_price = to_money(value.current_price.as_ref())
            .ok_or(eyre::eyre!("Failed to get current price"))?;

        let accrued_interest = to_money(value.current_nkd.as_ref())
            .filter(|nkd| !nkd.value.is_zero())
            .unwrap_or_else(|| Money::zero(current_instrument_price.currency));

        let daily_yield = to_money(value.daily_yield.as_ref())
            .unwrap_or_else(|| Money::zero(current_instrument_price.currency));

        Ok(Self {
            currency,
            average_buy_price,
            current_instrument_price,
            accrued_interest,
            quantity,
            daily_yield,
            blocked: value.blocked,
            blocked_lots: to_decimal(value.blocked_lots.as_ref()),
        })
    }
}

/// Maximum number of attempts made by [`with_retry`].
const MAX_ATTEMPTS: u32 = 5;

/// Upper bound for a server-requested delay, protects from waiting forever on a bogus value.
const MAX_SERVER_DELAY: Duration = Duration::from_secs(60);

/// gRPC metadata key with seconds left until the API rate limit window resets.
const RATE_LIMIT_RESET_HEADER: &str = "x-ratelimit-reset";

/// Shortest rate limit wait worth telling the user about.
const NOTICEABLE_DELAY: Duration = Duration::from_secs(1);

/// End of the last announced rate limit wait, shared by all concurrent requests
/// so that hitting the limit is announced once rather than by every waiting task.
static RATE_LIMIT_WAIT_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// Executes a future with exponential backoff retry logic.
///
/// Only transient failures (see [`is_transient`]) are retried, up to [`MAX_ATTEMPTS`] times
/// with delays 100ms, 200ms, 400ms, 800ms. When the API reports an exhausted rate limit,
/// the delay is extended up to the limit reset time. Other errors are returned immediately.
async fn with_retry<T, F, Fut>(f: F) -> color_eyre::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = color_eyre::Result<T>>,
{
    let mut delay = Duration::from_millis(100);
    let mut attempt = 1;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if !is_transient(&e) => return Err(e),
            Err(e) if attempt == MAX_ATTEMPTS => {
                return Err(e.wrap_err(format!("Operation failed after {MAX_ATTEMPTS} attempts")));
            }
            Err(e) => {
                let wait = match server_retry_delay(&e) {
                    Some(server_delay) => {
                        announce_rate_limit_wait(server_delay);
                        server_delay.max(delay)
                    }
                    None => delay,
                };
                sleep(wait).await;
                delay *= 2;
                attempt += 1;
            }
        }
    }
}

/// Prints a notice that requests are paused until the API rate limit resets.
fn announce_rate_limit_wait(wait: Duration) {
    let until = Instant::now() + wait;
    let Ok(mut last) = RATE_LIMIT_WAIT_UNTIL.lock() else {
        return;
    };
    if should_announce_wait(*last, until, wait) {
        *last = Some(until);
        eprintln!(
            "API rate limit reached, waiting {}s for it to reset...",
            wait.as_secs()
        );
    }
}

/// Whether a wait ending at `until` is long enough and not already covered by
/// the previously announced one ending at `last`.
fn should_announce_wait(last: Option<Instant>, until: Instant, wait: Duration) -> bool {
    wait >= NOTICEABLE_DELAY && last.is_none_or(|last| until > last + NOTICEABLE_DELAY)
}

/// Returns the gRPC status carried by the error chain, if any.
fn grpc_status(e: &eyre::Report) -> Option<&tonic::Status> {
    e.chain().find_map(|c| c.downcast_ref::<tonic::Status>())
}

/// Whether a failed request may succeed when repeated.
///
/// Transport failures and statuses like `Unavailable` are transient, while statuses like
/// `Unauthenticated` or `InvalidArgument` and errors of our own (e.g. a missing
/// instrument in a response) are not.
fn is_transient(e: &eyre::Report) -> bool {
    match grpc_status(e) {
        Some(s) => matches!(
            s.code(),
            Code::Unavailable
                | Code::ResourceExhausted
                | Code::DeadlineExceeded
                | Code::Aborted
                | Code::Internal
                | Code::Unknown
        ),
        None => e
            .chain()
            .any(|c| c.downcast_ref::<tonic::transport::Error>().is_some()),
    }
}

/// Delay requested by the API before the next request when the rate limit is exhausted.
fn server_retry_delay(e: &eyre::Report) -> Option<Duration> {
    let status = grpc_status(e).filter(|s| s.code() == Code::ResourceExhausted)?;
    let seconds = status
        .metadata()
        .get(RATE_LIMIT_RESET_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds).min(MAX_SERVER_DELAY))
}

impl TinkoffInvestment {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            interceptor: TInvestInterceptor { token },
            channel: Arc::new(OnceCell::new()),
        }
    }

    /// Returns a gRPC service client on the shared TLS channel.
    async fn client<C, F>(&self, make: F) -> color_eyre::Result<C>
    where
        F: FnOnce(Channel, TInvestInterceptor) -> C,
    {
        let channel = self.create_channel().await?;
        Ok(make(channel, self.interceptor.clone()))
    }

    /// Returns a shared TLS gRPC channel (connects on first use).
    async fn create_channel(&self) -> color_eyre::Result<Channel> {
        self.channel
            .get_or_try_init(|| async {
                let tls = ClientTlsConfig::new()
                    .ca_certificate(Certificate::from_pem(RUSSIAN_TRUSTED_CAS))
                    .domain_name("invest-public-api.tbank.ru");

                Channel::from_static(INVEST_API_ENDPOINT)
                    .tls_config(tls)
                    .map_err(|e| eyre::eyre!("TLS config failed: {e:?}"))?
                    .connect()
                    .await
                    .wrap_err("Failed to create channel")
            })
            .await
            .cloned()
    }

    /// Loads the portfolio and instruments of its positions.
    ///
    /// # Errors
    ///
    /// Returns an error if the portfolio request fails after retries.
    pub async fn get_portfolio_and_instruments(
        &self,
        selector: &AccountSelector,
    ) -> color_eyre::Result<(AccountPortfolio, HashMap<String, Instrument>)> {
        let portfolio = self.get_portfolio_until_done(selector).await?;
        let instruments = self
            .get_instruments_for_positions(&portfolio.positions)
            .await;
        Ok((portfolio, instruments))
    }

    /// Looks up instruments of the given positions by FIGI concurrently.
    ///
    /// Instruments that could not be loaded are absent from the result;
    /// callers fall back to FIGI for their name and ticker.
    pub async fn get_instruments_for_positions(
        &self,
        positions: &[PortfolioPosition],
    ) -> HashMap<String, Instrument> {
        self.parallel_for_positions(positions, None, |client, position| async move {
            let instrument = with_retry(|| {
                client.get_instrument(position.figi.clone(), &position.instrument_type)
            })
            .await;
            (position.figi, instrument)
        })
        .await
        .into_iter()
        .filter_map(|(figi, instrument)| Some((figi, instrument.ok()?)))
        .collect()
    }

    /// Looks up an instrument by FIGI. Bonds and currencies are requested by their own
    /// methods, as only these return the nominal currency the position is exposed to;
    /// shares, ETFs and futures too, as only these return the sector.
    async fn get_instrument(
        &self,
        figi: String,
        instrument_type: &str,
    ) -> color_eyre::Result<Instrument> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let request = InstrumentRequest {
            id_type: InstrumentIdType::Figi as i32,
            class_code: None,
            id: figi.clone(),
        };
        let instrument = match instrument_type {
            "bond" => instruments
                .bond_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_bond)),
            "currency" => instruments
                .currency_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_currency)),
            "share" => instruments
                .share_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_share)),
            "etf" => instruments
                .etf_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_etf)),
            "futures" => instruments
                .future_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_future)),
            _ => instruments
                .get_instrument_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_instrument)),
        }
        .wrap_err_with(|| format!("Failed to get instrument {figi}"))?;
        instrument.ok_or_else(|| eyre::eyre!("Instrument {figi} not found"))
    }

    /// Loads analyst forecasts and fundamentals of the given share positions.
    ///
    /// Shares without a forecast or fundamentals are kept with `None`; failed requests
    /// are returned alongside so the caller can tell the user that the output is incomplete.
    pub async fn get_share_analytics(
        &self,
        positions: &[PortfolioPosition],
        instruments: &HashMap<String, Instrument>,
    ) -> (Analytics, Vec<eyre::Report>) {
        let asset_uids = positions
            .iter()
            .filter_map(|p| instruments.get(&p.figi)?.asset_uid.clone())
            .unique()
            .collect_vec();
        let currencies: Arc<HashMap<String, Currency>> = Arc::new(
            instruments
                .iter()
                .filter_map(|(figi, i)| Some((figi.clone(), i.currency?)))
                .collect(),
        );
        let (forecasts, fundamentals) = tokio::join!(
            self.parallel_for_positions(positions, None, move |client, position| {
                let currency = currencies.get(&position.figi).copied();
                async move {
                    let forecast = with_retry(|| {
                        client.get_forecast(position.instrument_uid.clone(), currency)
                    })
                    .await;
                    (position.figi, forecast)
                }
            }),
            self.get_fundamentals_until_done(&asset_uids),
        );

        let mut failures = Vec::new();
        let fundamentals = fundamentals.unwrap_or_else(|e| {
            failures.push(e);
            HashMap::new()
        });
        let mut forecasts_by_figi = HashMap::new();
        for (figi, forecast) in forecasts {
            match forecast {
                Ok(forecast) => {
                    forecasts_by_figi.insert(figi, forecast);
                }
                Err(e) => {
                    let ticker = instrument_or_figi(instruments, &figi).ticker;
                    failures.push(e.wrap_err(format!("Forecast for {ticker} not loaded")));
                }
            }
        }

        let shares = positions
            .iter()
            .map(|p| {
                let instrument = instrument_or_figi(instruments, &p.figi);
                ShareAnalytics {
                    forecast: forecasts_by_figi.get(&p.figi).cloned().flatten(),
                    fundamentals: instrument
                        .asset_uid
                        .as_ref()
                        .and_then(|uid| fundamentals.get(uid))
                        .copied(),
                    name: instrument.name,
                    ticker: instrument.ticker,
                }
            })
            .collect();
        (Analytics::new(shares), failures)
    }

    /// Consensus forecast of an instrument; `None` when investment houses give none.
    /// `currency` is used when the API does not specify the forecast currency.
    async fn get_forecast(
        &self,
        instrument_uid: String,
        currency: Option<Currency>,
    ) -> color_eyre::Result<Option<Forecast>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        match instruments
            .get_forecast_by(GetForecastRequest {
                instrument_id: instrument_uid,
            })
            .await
        {
            Ok(response) => Ok(to_forecast(&response.into_inner(), currency)),
            Err(status) if status.code() == Code::NotFound => Ok(None),
            Err(status) => Err(status).wrap_err("Failed to get forecast"),
        }
    }

    /// Fundamentals by asset UID, requested in batches of [`MAX_FUNDAMENTALS_ASSETS`].
    async fn get_fundamentals_until_done(
        &self,
        asset_uids: &[String],
    ) -> color_eyre::Result<HashMap<String, Fundamentals>> {
        let mut result = HashMap::new();
        for chunk in asset_uids.chunks(MAX_FUNDAMENTALS_ASSETS) {
            let items = with_retry(|| self.get_fundamentals(chunk.to_vec())).await?;
            result.extend(items);
        }
        Ok(result)
    }

    async fn get_fundamentals(
        &self,
        asset_uids: Vec<String>,
    ) -> color_eyre::Result<Vec<(String, Fundamentals)>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let response = instruments
            .get_asset_fundamentals(GetAssetFundamentalsRequest { assets: asset_uids })
            .await
            .wrap_err("Failed to get fundamentals")?;
        Ok(response
            .into_inner()
            .fundamentals
            .iter()
            .map(|s| (s.asset_uid.clone(), to_fundamentals(s)))
            .collect())
    }

    /// Fetches data for each position in parallel with retries,
    /// limiting concurrent requests with a semaphore.
    ///
    /// Returns pairs of (position, fetch result); task panics are logged to stderr.
    async fn fetch_parallel<T, F, Fut>(
        &self,
        positions: &[PortfolioPosition],
        fetch: F,
    ) -> Vec<(PortfolioPosition, color_eyre::Result<Vec<T>>)>
    where
        T: Send + 'static,
        F: Fn(Self, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = color_eyre::Result<Vec<T>>> + Send,
    {
        let fetch = Arc::new(fetch);
        self.parallel_for_positions(positions, None, {
            move |client, position| {
                let fetch = Arc::clone(&fetch);
                async move {
                    let items = with_retry(|| fetch(client.clone(), position.figi.clone())).await;
                    (position, items)
                }
            }
        })
        .await
    }

    /// Runs `task` for each position concurrently, limited by [`MAX_CONCURRENT_REQUESTS`].
    ///
    /// Task panics are logged to stderr; failed permit acquisition skips the position.
    /// When `progress` is set, it is incremented once per completed task.
    async fn parallel_for_positions<T, F, Fut>(
        &self,
        positions: &[PortfolioPosition],
        progress: Option<Arc<dyn Progress>>,
        task: F,
    ) -> Vec<T>
    where
        T: Send + 'static,
        F: Fn(TinkoffInvestment, PortfolioPosition) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
        let task = Arc::new(task);
        let mut set = JoinSet::new();

        for position in positions {
            let client = self.clone();
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to acquire semaphore: {e}");
                    continue;
                }
            };
            let position = position.clone();
            let task = Arc::clone(&task);
            let progress = progress.clone();

            set.spawn(async move {
                let _permit = permit;
                let result = task(client, position).await;
                if let Some(p) = &progress {
                    p.progress();
                }
                result
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(item) => results.push(item),
                Err(e) => eprintln!("Task panicked or cancelled: {e}"),
            }
        }
        results
    }

    /// Builds a [`Portfolio`] by loading papers for each position in parallel.
    ///
    /// Position money fields and operation totals are converted to RUB via FX rates.
    /// Positions that failed to load are not included into the portfolio; their errors
    /// are returned alongside so the caller can tell the user that totals are incomplete.
    /// Bond details are loaded only with `output_papers`, since only paper cards show them.
    pub async fn build_portfolio(
        &self,
        instruments: Arc<HashMap<String, Instrument>>,
        positions: &[PortfolioPosition],
        account_id: &str,
        output_papers: bool,
        progress: Option<Arc<dyn Progress>>,
    ) -> (Portfolio, Vec<eyre::Report>) {
        let account_id = account_id.to_string();
        let fx = match self.load_fx_book().await {
            Ok(fx) => fx,
            Err(e) => {
                eprintln!("Failed to load FX book: {e:?}");
                Arc::new(FxBook {
                    instruments: HashMap::new(),
                    daily: Mutex::new(HashMap::new()),
                })
            }
        };

        let papers = self
            .parallel_for_positions(positions, progress.clone(), {
                let account_id = account_id.clone();
                let fx = fx.clone();
                move |client, position| {
                    let instruments = instruments.clone();
                    let account_id = account_id.clone();
                    let fx = fx.clone();
                    async move {
                        client
                            .paper_for_position(
                                &instruments,
                                &account_id,
                                &position,
                                &fx,
                                output_papers,
                            )
                            .await
                    }
                }
            })
            .await;

        if let Some(p) = &progress {
            p.finish();
        }

        let mut portfolio = Portfolio::new(output_papers);
        let mut failures = Vec::new();
        for paper in papers {
            match paper {
                Ok(paper) => portfolio.add_loaded_paper(paper),
                Err(e) => failures.push(e),
            }
        }
        (portfolio, failures)
    }

    /// Loads a position as a paper; bond details (events, YTM) only when `with_bond_details`,
    /// as they are shown in paper cards only and cost a request per bond.
    async fn paper_for_position(
        &self,
        instruments: &HashMap<String, Instrument>,
        account_id: &str,
        position: &PortfolioPosition,
        fx: &FxBook,
        with_bond_details: bool,
    ) -> color_eyre::Result<LoadedPaper> {
        let skip = |e| {
            skipped(
                e,
                &instrument_or_figi(instruments, &position.figi),
                &position.figi,
            )
        };
        // Checked before any request, so unsupported positions cost no API calls.
        let tag: fn(Paper<NoneProfit>) -> LoadedPaper = match position.instrument_type.as_str() {
            "bond" => |p| LoadedPaper::Bond(p.with_profit(CouponProfit)),
            "share" => |p| LoadedPaper::Share(p.with_profit(DividendProfit)),
            "etf" => |p| LoadedPaper::Etf(p.with_profit(DividendProfit)),
            "currency" => LoadedPaper::Currency,
            "futures" => LoadedPaper::Future,
            other => {
                return Err(skip(eyre::eyre!("Unsupported instrument type '{other}'")));
            }
        };
        let mut paper = self
            .create_paper_from_position(instruments, account_id.to_string(), position, fx)
            .await
            .map_err(skip)?;
        if with_bond_details && position.instrument_type == "bond" {
            // Bond details are informational: the position stays in totals without them.
            paper.bond = self
                .load_bond_info(&position.figi, &paper.position, fx)
                .await
                .ok();
        }
        Ok(tag(paper))
    }

    async fn get_portfolio(&self, account_id: &str) -> color_eyre::Result<AccountPortfolio> {
        let mut operations = self
            .client(OperationsServiceClient::with_interceptor)
            .await?;

        // Money values in RUB, converted by the broker at the current rate.
        let portfolio = operations
            .get_portfolio(PortfolioRequest {
                account_id: account_id.to_string(),
                currency: Some(CurrencyRequest::Rub as i32),
            })
            .await
            .wrap_err("Failed to get portfolio")?;
        Ok(AccountPortfolio {
            account_id: account_id.to_string(),
            positions: portfolio.into_inner().positions,
        })
    }

    /// Get accounts response from API.
    ///
    /// # Errors
    ///
    /// This function will return an error if accounts cannot be retrieved.
    async fn get_accounts_response(&self) -> color_eyre::Result<GetAccountsResponse> {
        let mut users = self.client(UsersServiceClient::with_interceptor).await?;
        let accounts = users
            .get_accounts(GetAccountsRequest {
                status: Some(AccountStatus::All as i32),
            })
            .await
            .wrap_err("Failed to get accounts")?;
        Ok(accounts.into_inner())
    }

    /// Lists all accounts of the user.
    ///
    /// # Errors
    ///
    /// Returns an error if accounts cannot be retrieved after retries.
    pub async fn get_accounts(&self) -> color_eyre::Result<Vec<Account>> {
        let response = with_retry(|| self.get_accounts_response()).await?;
        Ok(response.accounts)
    }

    /// Gets the account chosen by `selector`.
    ///
    /// # Errors
    ///
    /// Returns an error if accounts cannot be retrieved or the selector does not
    /// match exactly one account (see [`select_account`]).
    pub async fn get_account(&self, selector: &AccountSelector) -> color_eyre::Result<Account> {
        let accounts = self.get_accounts().await?;
        select_account(&accounts, selector).cloned()
    }

    /// Search instruments by ticker.
    ///
    /// # Errors
    ///
    /// This function will return an error if instruments cannot be get from remote server.
    pub async fn find_instruments_by_ticker(
        &self,
        ticker: String,
    ) -> color_eyre::Result<Vec<InstrumentShort>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let response = instruments
            .find_instrument(FindInstrumentRequest {
                instrument_kind: Some(InstrumentType::Unspecified.into()),
                query: ticker.clone(),
                api_trade_available_flag: Some(false),
            })
            .await
            .wrap_err_with(|| format!("Failed to find instruments by ticker {ticker}"))?;
        Ok(response.into_inner().instruments)
    }

    /// Search instruments by ticker with retry logic.
    ///
    /// # Errors
    ///
    /// This function will return an error if instruments cannot be found after multiple retries.
    pub async fn find_instruments_by_ticker_until_done(
        &self,
        ticker: &str,
    ) -> color_eyre::Result<Vec<InstrumentShort>> {
        with_retry(|| self.find_instruments_by_ticker(ticker.to_string())).await
    }

    /// Get portfolio until done with retry logic.
    ///
    /// # Errors
    ///
    /// This function will return an error if portfolio cannot be retrieved after multiple retries.
    pub async fn get_portfolio_until_done(
        &self,
        selector: &AccountSelector,
    ) -> color_eyre::Result<AccountPortfolio> {
        let account = self.get_account(selector).await?;
        with_retry(|| self.get_portfolio(&account.id)).await
    }

    async fn get_operations_page(
        &self,
        request: GetOperationsByCursorRequest,
    ) -> color_eyre::Result<GetOperationsByCursorResponse> {
        let mut operations = self
            .client(OperationsServiceClient::with_interceptor)
            .await?;
        let response = operations
            .get_operations_by_cursor(request)
            .await
            .wrap_err("Failed to get operations")?;
        Ok(response.into_inner())
    }

    /// Gets all executed operations matching `request` page by page, retrying each page.
    async fn get_all_operations(
        &self,
        request: GetOperationsByCursorRequest,
    ) -> color_eyre::Result<Vec<OperationItem>> {
        let request = GetOperationsByCursorRequest {
            limit: Some(OPERATIONS_PAGE_SIZE),
            state: Some(OperationState::Executed as i32),
            without_trades: Some(true),
            ..request
        };
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let page = with_retry(|| {
                self.get_operations_page(GetOperationsByCursorRequest {
                    cursor: cursor.clone(),
                    ..request.clone()
                })
            })
            .await?;
            items.extend(page.items);
            if !page.has_next || page.next_cursor.is_empty() {
                return Ok(items);
            }
            cursor = Some(page.next_cursor);
        }
    }

    /// Income and taxes of the account by year, converted to RUB at operation dates.
    ///
    /// # Errors
    ///
    /// Returns an error if the account, its operations or FX rates cannot be loaded.
    pub async fn get_tax_report(
        &self,
        selector: &AccountSelector,
    ) -> color_eyre::Result<TaxReport> {
        let account = self.get_account(selector).await?;
        let operations = self
            .get_all_operations(GetOperationsByCursorRequest {
                account_id: account.id,
                operation_types: TAX_OPERATION_TYPES.map(|t| t as i32).to_vec(),
                ..Default::default()
            })
            .await?;
        let fx = self.load_fx_book().await?;

        let mut items = Vec::new();
        for op in operations.iter().unique_by(|op| &op.id) {
            let kind = OperationType::try_from(op.r#type)
                .ok()
                .and_then(tax_operation_kind);
            let (Some(kind), Some(payment)) = (kind, to_money(op.payment.as_ref())) else {
                continue;
            };
            let date = to_datetime_utc(op.date.as_ref());
            let amount = self.money_to_rub(&fx, payment, date).await?;
            items.push(TaxOperation { date, kind, amount });
        }
        Ok(TaxReport::new(&items))
    }

    /// Gets all executed operations of an instrument page by page, retrying each page.
    ///
    /// # Errors
    ///
    /// This function will return an error if a page cannot be retrieved after multiple retries.
    pub async fn get_operations_until_done(
        &self,
        account_id: String,
        figi: String,
    ) -> color_eyre::Result<Vec<OperationItem>> {
        self.get_all_operations(GetOperationsByCursorRequest {
            account_id,
            instrument_id: Some(figi),
            ..Default::default()
        })
        .await
    }

    /// Creates a paper from a portfolio position with prices and operation totals in RUB.
    ///
    /// The paper has no additional profit kind; tag it with [`Paper::with_profit`].
    ///
    /// # Errors
    ///
    /// Returns an error if position prices are missing, FX conversion fails,
    /// or operations cannot be loaded.
    pub async fn create_paper_from_position(
        &self,
        instruments: &HashMap<String, Instrument>,
        account_id: String,
        portfolio_position: &PortfolioPosition,
        fx: &FxBook,
    ) -> color_eyre::Result<Paper<NoneProfit>> {
        // Portfolio is requested in RUB; the position keeps the currency it is exposed to.
        let mut position = Position::try_from(portfolio_position)?;
        ensure_rub(&position)?;

        let instrument = instrument_or_figi(instruments, &portfolio_position.figi);
        if let Some(currency) = instrument.currency {
            position.currency = currency;
        }

        let executed_ops = self
            .get_operations_until_done(account_id, portfolio_position.figi.clone())
            .await?;

        let totals = self.reduce(fx, &executed_ops).await?;

        Ok(Paper {
            name: instrument.name,
            ticker: instrument.ticker,
            figi: Figi::new(portfolio_position.figi.clone()),
            position,
            totals,
            profit: NoneProfit,
            bond: None,
            sector: instrument.sector,
        })
    }

    /// Accumulates fees and pure-income payments already denominated in RUB.
    #[must_use]
    pub fn accumulate_totals_rub(
        pure_income: impl IntoIterator<Item = Money>,
        fees: impl IntoIterator<Item = Money>,
    ) -> Totals {
        let mut additional_profit = Money::zero(Currency::RUB);
        let mut fee_total = Money::zero(Currency::RUB);
        for payment in pure_income {
            additional_profit += payment;
        }
        for payment in fees {
            fee_total += payment;
        }
        Totals {
            additional_profit,
            fees: fee_total,
            cash_flows: Vec::new(),
        }
    }

    async fn reduce(
        &self,
        fx: &FxBook,
        operations: &[OperationItem],
    ) -> color_eyre::Result<Totals> {
        let mut income = Vec::new();
        let mut fees = Vec::new();
        let mut cash_flows = Vec::new();
        for op in operations {
            let Some(payment) = crate::to_money(op.payment.as_ref()) else {
                continue;
            };
            let at = to_datetime_utc(op.date.as_ref());
            if let Some(accrued_interest) = trade_accrued_interest(op) {
                income.push(self.money_to_rub(fx, accrued_interest, at).await?);
            }
            let payment = self.money_to_rub(fx, payment, at).await?;
            if !payment.value.is_zero() {
                cash_flows.push(CashFlow {
                    date: at,
                    amount: payment.value,
                });
            }
            match to_influence(op.r#type()) {
                OperationInfluence::PureIncome => income.push(payment),
                OperationInfluence::Fees => fees.push(payment),
                OperationInfluence::Unspecified => {}
            }
        }
        Ok(Totals {
            cash_flows,
            ..Self::accumulate_totals_rub(income, fees)
        })
    }

    /// Fetches payments of the given kind for portfolio positions due within `days` from today.
    ///
    /// Positions whose payments failed to load are not included into the calendar;
    /// their errors are returned alongside so the caller can report an incomplete result.
    ///
    /// # Errors
    ///
    /// Returns an error if FX rates cannot be loaded or converted.
    pub async fn get_calendar(
        &self,
        portfolio: &AccountPortfolio,
        instruments: &HashMap<String, Instrument>,
        kind: CalendarKind,
        days: u32,
    ) -> color_eyre::Result<(Calendar, Vec<eyre::Report>)> {
        let period = CalendarPeriod::days_ahead(Utc::now(), days);
        let fx = self.load_fx_book().await?;
        match kind {
            CalendarKind::Dividends => {
                let (calendar, failures) = self
                    .get_dividend_calendar(portfolio, instruments, &period, &fx)
                    .await?;
                Ok((Calendar::Dividends(calendar), failures))
            }
            CalendarKind::Coupons => {
                let (calendar, failures) = self
                    .get_coupon_calendar(portfolio, instruments, &period, &fx)
                    .await?;
                Ok((Calendar::Coupons(calendar), failures))
            }
            CalendarKind::Combined => {
                let (dividends, mut failures) = self
                    .get_dividend_calendar(portfolio, instruments, &period, &fx)
                    .await?;
                let (coupons, coupon_failures) = self
                    .get_coupon_calendar(portfolio, instruments, &period, &fx)
                    .await?;
                failures.extend(coupon_failures);
                Ok((
                    Calendar::Combined(CombinedCalendar::merge(dividends, coupons)),
                    failures,
                ))
            }
        }
    }

    /// Builds instrument history with all money fields converted to RUB.
    ///
    /// # Errors
    ///
    /// Returns an error if FX conversion fails.
    pub async fn history_in_rub(
        &self,
        operations: &[OperationItem],
        instrument: &InstrumentShort,
    ) -> color_eyre::Result<Option<History>> {
        let fx = self.load_fx_book().await?;
        let mut items: Vec<HistoryItem> = Vec::new();
        for op in operations.iter().unique_by(|op| &op.id) {
            let mut item = HistoryItem::from(op);
            let at = item.datetime;
            item.payment = self.money_to_rub(&fx, item.payment, at).await?;
            item.price = self.money_to_rub(&fx, item.price, at).await?;
            items.push(item);
        }
        items.sort_by(|a, b| Ord::cmp(&a.datetime, &b.datetime));
        let Some(first) = items.first() else {
            return Ok(None);
        };
        Ok(Some(History {
            name: instrument.name.clone(),
            ticker: instrument.ticker.clone(),
            figi: instrument.figi.clone(),
            currency: first.payment.currency,
            items,
        }))
    }

    /// Loads currency instruments for FX conversion.
    ///
    /// # Errors
    ///
    /// Returns an error if the Currencies catalog cannot be fetched.
    pub async fn load_fx_book(&self) -> color_eyre::Result<Arc<FxBook>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let response = instruments
            .currencies(InstrumentsRequest {
                instrument_status: Some(InstrumentStatus::All as i32),
                instrument_exchange: None,
            })
            .await
            .wrap_err("Failed to fetch currencies")?;

        let candidates = response
            .into_inner()
            .instruments
            .into_iter()
            .filter_map(|c| {
                let currency = Currency::from_code(&c.iso_currency_name.to_ascii_uppercase())?;
                let nominal = c.nominal.as_ref().map_or(Decimal::ONE, money_value_amount);
                Some(FxCandidate {
                    currency,
                    instrument_id: if c.uid.is_empty() { c.figi } else { c.uid },
                    lot: c.lot,
                    nominal,
                    settlement_is_rub: c.currency.eq_ignore_ascii_case("rub"),
                })
            });

        Ok(Arc::new(FxBook {
            instruments: build_fx_map(candidates),
            daily: Mutex::new(HashMap::new()),
        }))
    }

    /// Converts `money` to RUB at the daily rate of the `at` date.
    async fn money_to_rub(
        &self,
        fx: &FxBook,
        money: Money,
        at: DateTime<Utc>,
    ) -> color_eyre::Result<Money> {
        if money.currency == Currency::RUB {
            return Ok(money);
        }
        let rate = self.rate_to_rub(fx, money.currency, at).await?;
        Ok(money.to_currency(rate, Currency::RUB))
    }

    async fn rate_to_rub(
        &self,
        fx: &FxBook,
        currency: Currency,
        at: DateTime<Utc>,
    ) -> color_eyre::Result<Decimal> {
        if currency == Currency::RUB {
            return Ok(Decimal::ONE);
        }
        let instr = fx
            .instruments
            .get(&currency)
            .ok_or_else(|| eyre::eyre!("No FX instrument for currency {currency:?}"))?;
        self.hist_rate(fx, currency, instr, at).await
    }

    async fn hist_rate(
        &self,
        fx: &FxBook,
        currency: Currency,
        instr: &FxInstrument,
        at: DateTime<Utc>,
    ) -> color_eyre::Result<Decimal> {
        let date = at.date_naive();
        // Weekends and holidays have no candle: take the closest earlier trading day.
        for offset in 0..=FX_LOOKBACK_DAYS {
            let day = date
                .checked_sub_signed(ChronoDuration::days(offset))
                .unwrap_or(date);
            if let Some(rate) = self.daily_rate(fx, currency, instr, day).await? {
                return Ok(rate);
            }
        }
        Err(eyre::eyre!("No FX candle for {currency:?} around {date}"))
    }

    /// Rate of `day` from the daily candles of its year, loading them on first use.
    async fn daily_rate(
        &self,
        fx: &FxBook,
        currency: Currency,
        instr: &FxInstrument,
        day: NaiveDate,
    ) -> color_eyre::Result<Option<Decimal>> {
        let year = day.year();
        let cell = {
            let mut cells = fx
                .daily
                .lock()
                .map_err(|e| eyre::eyre!("FX cache is poisoned: {e}"))?;
            Arc::clone(cells.entry((currency, year)).or_default())
        };
        let rates = cell
            .get_or_try_init(|| with_retry(|| self.get_year_rates(currency, instr, year)))
            .await?;
        Ok(rates.get(&day).copied())
    }

    async fn get_year_rates(
        &self,
        currency: Currency,
        instr: &FxInstrument,
        year: i32,
    ) -> color_eyre::Result<DailyRates> {
        let (from, to) = year_bounds(year).ok_or_else(|| eyre::eyre!("Invalid year {year}"))?;
        let mut market = self
            .client(MarketDataServiceClient::with_interceptor)
            .await?;
        let response = market
            .get_candles(GetCandlesRequest {
                from: Some(prost_types::Timestamp {
                    seconds: from.timestamp(),
                    nanos: 0,
                }),
                to: Some(prost_types::Timestamp {
                    seconds: to.timestamp(),
                    nanos: 0,
                }),
                interval: CandleInterval::Day as i32,
                instrument_id: Some(instr.instrument_id.clone()),
                ..Default::default()
            })
            .await
            .wrap_err_with(|| format!("GetCandles failed for {currency:?} in {year}"))?;
        Ok(daily_rates(&response.into_inner().candles, instr))
    }

    /// Internal method for fetching dividend calendar with optional date filtering.
    async fn get_dividend_calendar(
        &self,
        portfolio: &AccountPortfolio,
        instruments: &HashMap<String, Instrument>,
        period: &CalendarPeriod,
        fx: &FxBook,
    ) -> color_eyre::Result<(DividendCalendar, Vec<eyre::Report>)> {
        let pairs = self.fetch_portfolio_dividends(portfolio).await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (position, dividends) in pairs {
            let instrument = instrument_or_figi(instruments, &position.figi);
            let dividends = match dividends {
                Ok(dividends) => dividends,
                Err(e) => {
                    failures.push(skipped(e, &instrument, &position.figi));
                    continue;
                }
            };
            for dividend in dividends {
                let dividend_per_share = dividend_per_share(&dividend);
                let payment_date = to_optional_datetime_utc(dividend.payment_date.as_ref());
                let record_date = to_optional_datetime_utc(dividend.record_date.as_ref());
                // Without any date a dividend cannot be placed into the calendar.
                let Some(calendar_date) = dividend_date(&dividend) else {
                    continue;
                };

                if !period.contains(calendar_date) {
                    continue;
                }

                // Future payments are converted at today's rate.
                let dividend_per_share = self
                    .money_to_rub(fx, dividend_per_share, period.from)
                    .await?;

                let quantity = to_decimal(position.quantity.as_ref());
                upcoming.push(DividendPayment {
                    figi: Figi::new(position.figi.clone()),
                    ticker: Ticker::new(instrument.ticker.as_str().to_string()),
                    name: instrument.name.clone(),
                    currency: Currency::RUB,
                    dividend_per_share,
                    total_dividend: dividend_per_share * quantity,
                    quantity,
                    ex_dividend_date: record_date.unwrap_or(calendar_date),
                    payment_date,
                    dividend_type: dividend.dividend_type,
                });
            }
        }

        upcoming.sort_by_key(|a| a.ex_dividend_date);
        Ok((DividendCalendar { upcoming }, failures))
    }

    /// All dividends, past and declared, of portfolio shares and ETFs.
    async fn fetch_portfolio_dividends(
        &self,
        portfolio: &AccountPortfolio,
    ) -> Vec<(PortfolioPosition, color_eyre::Result<Vec<Dividend>>)> {
        // The API rejects dividend requests for anything but shares and ETFs.
        let dividend_positions: Vec<PortfolioPosition> = portfolio
            .positions
            .iter()
            .filter(|p| p.instrument_type == "share" || p.instrument_type == "etf")
            .cloned()
            .collect();

        self.fetch_parallel(&dividend_positions, |client, figi| async move {
            client.get_dividends_for_figi(figi).await
        })
        .await
    }

    /// Passive income expected within a year: coupons, declared dividends and dividends
    /// estimated by the last year ones, with the current portfolio value for its yield.
    ///
    /// Amounts are before taxes; foreign currency payments are converted at today's rate.
    ///
    /// # Errors
    ///
    /// Returns an error if FX rates cannot be loaded or a payment cannot be converted to RUB.
    /// Positions that failed to load are returned alongside the forecast.
    pub async fn get_income_forecast(
        &self,
        portfolio: &AccountPortfolio,
        instruments: &HashMap<String, Instrument>,
    ) -> color_eyre::Result<(IncomeForecast, Vec<eyre::Report>)> {
        let period = CalendarPeriod::days_ahead(Utc::now(), INCOME_FORECAST_DAYS);
        let fx = self.load_fx_book().await?;

        let (coupons, mut failures) = self
            .get_coupon_calendar(portfolio, instruments, &period, &fx)
            .await?;
        let mut items: Vec<IncomeItem> = coupons
            .upcoming
            .iter()
            .filter(|c| c.kind == BondEventKind::Coupon)
            .map(|c| IncomeItem {
                date: c.coupon_date,
                kind: IncomeKind::Coupon,
                amount: c.total_coupon,
            })
            .collect();

        for (position, dividends) in self.fetch_portfolio_dividends(portfolio).await {
            let dividends = match dividends {
                Ok(dividends) => dividends,
                Err(e) => {
                    let instrument = instrument_or_figi(instruments, &position.figi);
                    failures.push(skipped(e, &instrument, &position.figi));
                    continue;
                }
            };
            let mut records = Vec::new();
            for dividend in &dividends {
                let Some(date) = dividend_date(dividend) else {
                    continue;
                };
                // Past and future payments are all converted at today's rate.
                let per_share = self
                    .money_to_rub(&fx, dividend_per_share(dividend), period.from)
                    .await?;
                records.push(DividendRecord { date, per_share });
            }
            let quantity = to_decimal(position.quantity.as_ref());
            items.extend(expected_dividends(
                &records,
                quantity,
                period.from,
                period.until,
            ));
        }

        let mut portfolio_value = Money::zero(Currency::RUB);
        for position in &portfolio.positions {
            let value = Position::try_from(position).and_then(|p| {
                ensure_rub(&p)?;
                Ok(p.current())
            });
            match value {
                Ok(value) => portfolio_value += value,
                Err(e) => {
                    let instrument = instrument_or_figi(instruments, &position.figi);
                    failures.push(skipped(e, &instrument, &position.figi));
                }
            }
        }

        let forecast = IncomeForecast::new(period.from, period.until, &items, portfolio_value);
        Ok((forecast, failures))
    }

    async fn get_dividends_for_figi(&self, figi: String) -> color_eyre::Result<Vec<Dividend>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let response = instruments
            .get_dividends(GetDividendsRequest {
                instrument_id: figi,
                from: None,
                to: None,
                ..Default::default()
            })
            .await
            .wrap_err("Failed to get dividends")?;
        Ok(response.into_inner().dividends)
    }

    /// Internal method for fetching coupon calendar with optional date filtering.
    async fn get_coupon_calendar(
        &self,
        portfolio: &AccountPortfolio,
        instruments: &HashMap<String, Instrument>,
        period: &CalendarPeriod,
        fx: &FxBook,
    ) -> color_eyre::Result<(CouponCalendar, Vec<eyre::Report>)> {
        let bond_positions: Vec<PortfolioPosition> = portfolio
            .positions
            .iter()
            .filter(|p| p.instrument_type == "bond")
            .cloned()
            .collect();

        let pairs = self
            .fetch_parallel(&bond_positions, |client, figi| async move {
                client.get_bond_events(figi).await
            })
            .await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (position, events) in pairs {
            let instrument = instrument_or_figi(instruments, &position.figi);
            let events = match events {
                Ok(events) => events,
                Err(e) => {
                    failures.push(skipped(e, &instrument, &position.figi));
                    continue;
                }
            };
            for event in events.into_iter().filter(|e| e.kind.is_payment()) {
                let coupon_value = event.per_bond;
                let coupon_date = event.date;

                if !period.contains(coupon_date) {
                    continue;
                }

                let coupon_value = self.money_to_rub(fx, coupon_value, period.from).await?;

                let quantity = to_decimal(position.quantity.as_ref());
                upcoming.push(CouponPayment {
                    figi: Figi::new(position.figi.clone()),
                    ticker: Ticker::new(instrument.ticker.as_str().to_string()),
                    name: instrument.name.clone(),
                    currency: Currency::RUB,
                    coupon_per_bond: coupon_value,
                    total_coupon: coupon_value * quantity,
                    quantity,
                    coupon_date,
                    kind: event.kind,
                });
            }
        }

        upcoming.sort_by_key(|a| a.coupon_date);
        Ok((CouponCalendar { upcoming }, failures))
    }

    /// Bond events from today on; redemptions but the last one are marked as amortization.
    async fn get_bond_events(&self, figi: String) -> color_eyre::Result<Vec<BondEvent>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let today = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map(|d| d.and_utc())
            .ok_or_else(|| eyre::eyre!("Invalid current date"))?;
        let until = today + ChronoDuration::days(BOND_EVENTS_HORIZON_DAYS);
        let response = instruments
            .get_bond_events(GetBondEventsRequest {
                from: Some(prost_types::Timestamp {
                    seconds: today.timestamp(),
                    nanos: 0,
                }),
                to: Some(prost_types::Timestamp {
                    seconds: until.timestamp(),
                    nanos: 0,
                }),
                instrument_id: figi,
                r#type: EventType::Unspecified as i32,
            })
            .await
            .wrap_err("Failed to get bond events")?;
        let mut events: Vec<BondEvent> = response
            .into_inner()
            .events
            .iter()
            .filter_map(to_bond_event)
            .collect();
        mark_amortizations(&mut events);
        Ok(events)
    }

    /// Maturity, next offer and YTM of a bond at its current dirty price.
    async fn load_bond_info(
        &self,
        figi: &str,
        position: &Position,
        fx: &FxBook,
    ) -> color_eyre::Result<BondInfo> {
        let now = Utc::now();
        let mut events = with_retry(|| self.get_bond_events(figi.to_string())).await?;
        for event in &mut events {
            event.per_bond = self.money_to_rub(fx, event.per_bond, now).await?;
        }
        let dirty_price = position.current_instrument_price.value + position.accrued_interest.value;
        Ok(bond_info(&events, dirty_price, now))
    }
}

/// Checks that position money values are in RUB as requested from the portfolio API,
/// so a mismatch is reported instead of mixing currencies in totals.
fn ensure_rub(position: &Position) -> color_eyre::Result<()> {
    for money in [
        position.average_buy_price,
        position.current_instrument_price,
        position.accrued_interest,
        position.daily_yield,
    ] {
        if money.currency != Currency::RUB {
            return Err(eyre::eyre!(
                "Expected position prices in RUB, got {}",
                money.currency.code()
            ));
        }
    }
    Ok(())
}

/// Accrued coupon interest (NKD) paid on a bond buy (negative) or received on a sell (positive).
///
/// A bond trade payment includes NKD besides `price × quantity`, so adding it to coupons
/// gives the net coupon income.
fn trade_accrued_interest(op: &OperationItem) -> Option<Money> {
    if op.instrument_type != "bond" {
        return None;
    }
    let direction = match op.r#type() {
        OperationType::Buy | OperationType::BuyCard | OperationType::BuyMargin => {
            Decimal::NEGATIVE_ONE
        }
        OperationType::Sell | OperationType::SellCard | OperationType::SellMargin => Decimal::ONE,
        _ => return None,
    };
    let accrued = to_money(op.accrued_int.as_ref())?;
    (!accrued.value.is_zero()).then(|| accrued * direction)
}

/// Start of `year` and start of the next one in UTC.
fn year_bounds(year: i32) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = |y: i32| {
        NaiveDate::from_ymd_opt(y, 1, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|d| d.and_utc())
    };
    Some((start(year)?, start(year.checked_add(1)?)?))
}

/// Maps daily candles to RUB rates by candle date; the last candle of a date wins.
fn daily_rates(candles: &[HistoricCandle], instr: &FxInstrument) -> DailyRates {
    candles
        .iter()
        .filter_map(|c| {
            let date = to_optional_datetime_utc(c.time.as_ref())?.date_naive();
            let close = c.close.as_ref()?;
            let rate = quote_to_rub_rate(to_decimal(Some(close)), instr.lot, instr.nominal);
            Some((date, rate))
        })
        .collect()
}

/// Dates a calendar covers: from `from`'s day to `until`'s day inclusive.
struct CalendarPeriod {
    from: DateTime<Utc>,
    until: DateTime<Utc>,
}

impl CalendarPeriod {
    fn days_ahead(from: DateTime<Utc>, days: u32) -> Self {
        Self {
            from,
            until: from + ChronoDuration::days(i64::from(days)),
        }
    }

    /// Whether a payment on `date` falls into the period; payments due today are included.
    fn contains(&self, date: DateTime<Utc>) -> bool {
        let day = date.date_naive();
        day >= self.from.date_naive() && day <= self.until.date_naive()
    }
}

/// Dividend per share before taxes; zero when not known yet.
fn dividend_per_share(dividend: &Dividend) -> Money {
    to_money(dividend.dividend_net.as_ref()).unwrap_or_else(|| Money::zero(Currency::RUB))
}

/// Payment date of a dividend, or its record date when the payment date is unknown.
fn dividend_date(dividend: &Dividend) -> Option<DateTime<Utc>> {
    to_optional_datetime_utc(dividend.payment_date.as_ref())
        .or_else(|| to_optional_datetime_utc(dividend.record_date.as_ref()))
}

/// Adds to `e` which position was skipped because of it.
fn skipped(e: eyre::Report, instrument: &Instrument, figi: &str) -> eyre::Report {
    e.wrap_err(format!("Position {} ({figi}) skipped", instrument.ticker))
}

fn to_recommendation(recommendation: ApiRecommendation) -> Option<Recommendation> {
    match recommendation {
        ApiRecommendation::Buy => Some(Recommendation::Buy),
        ApiRecommendation::Hold => Some(Recommendation::Hold),
        ApiRecommendation::Sell => Some(Recommendation::Sell),
        ApiRecommendation::Unspecified => None,
    }
}

/// Consensus forecast; `None` without a consensus target price or a known currency.
///
/// The API may leave the consensus currency empty, then the currency of investment house
/// forecasts is used and `fallback` as the last resort.
fn to_forecast(response: &GetForecastResponse, fallback: Option<Currency>) -> Option<Forecast> {
    let consensus = response.consensus.as_ref()?;
    let currency = std::iter::once(&consensus.currency)
        .chain(response.targets.iter().map(|t| &t.currency))
        .find_map(|code| Currency::from_code(&code.to_ascii_uppercase()))
        .or(fallback)?;
    let money = |q: Option<&Quotation>| Money::from_value(to_decimal(q), currency);
    let forecast = Forecast {
        recommendation: to_recommendation(consensus.recommendation()),
        current_price: money(consensus.current_price.as_ref()),
        target_price: money(consensus.consensus.as_ref()),
        min_target: money(consensus.min_target.as_ref()),
        max_target: money(consensus.max_target.as_ref()),
        analysts: response.targets.len(),
    };
    Some(forecast).filter(|f| !f.target_price.value.is_zero())
}

fn to_fundamentals(statistic: &StatisticResponse) -> Fundamentals {
    Fundamentals {
        pe: to_metric(statistic.pe_ratio_ttm),
        pb: to_metric(statistic.price_to_book_ttm),
        ev_to_ebitda: to_metric(statistic.ev_to_ebitda_mrq),
        net_debt_to_ebitda: to_metric(statistic.net_debt_to_ebitda),
        roe: to_metric(statistic.roe),
        dividend_yield: to_metric(statistic.dividend_yield_daily_ttm),
        beta: to_metric(statistic.beta),
    }
}

/// Returns the instrument for `figi` or, when it failed to load, one named after the FIGI,
/// so a position is never dropped only because its name is unknown.
fn instrument_or_figi(instruments: &HashMap<String, Instrument>, figi: &str) -> Instrument {
    instruments
        .get(figi)
        .cloned()
        .unwrap_or_else(|| Instrument {
            name: figi.to_string(),
            ticker: Ticker::new(figi),
            currency: None,
            asset_uid: None,
            sector: None,
        })
}

/// Instrument exposed to the `exposure` currency, or to the `trading` one when it is unknown.
fn instrument(
    name: String,
    ticker: String,
    asset_uid: String,
    exposure: &str,
    trading: &str,
) -> Instrument {
    let parse = |code: &str| Currency::from_code(&code.to_ascii_uppercase());
    Instrument {
        currency: parse(exposure).or_else(|| parse(trading)),
        name,
        ticker: Ticker::new(ticker),
        asset_uid: Some(asset_uid).filter(|uid| !uid.is_empty()),
        sector: None,
    }
}

/// Sector code normalized to lower case; `None` when the API returned none.
fn sector(code: &str) -> Option<String> {
    Some(code.trim().to_lowercase()).filter(|s| !s.is_empty())
}

fn from_api_instrument(i: ApiInstrument) -> Instrument {
    instrument(i.name, i.ticker, i.asset_uid, &i.currency, &i.currency)
}

fn from_api_share(s: ApiShare) -> Instrument {
    Instrument {
        sector: sector(&s.sector),
        ..instrument(s.name, s.ticker, s.asset_uid, &s.currency, &s.currency)
    }
}

fn from_api_etf(e: ApiEtf) -> Instrument {
    Instrument {
        sector: sector(&e.sector),
        ..instrument(e.name, e.ticker, e.asset_uid, &e.currency, &e.currency)
    }
}

/// Futures have no asset of their own, only a basic one.
fn from_api_future(f: ApiFuture) -> Instrument {
    Instrument {
        sector: sector(&f.sector),
        ..instrument(f.name, f.ticker, String::new(), &f.currency, &f.currency)
    }
}

/// Bond exposed to its nominal currency, e.g. USD for a replacement bond traded in RUB.
fn from_api_bond(b: ApiBond) -> Instrument {
    let nominal = b.nominal.map(|n| n.currency).unwrap_or_default();
    Instrument {
        sector: sector(&b.sector),
        ..instrument(b.name, b.ticker, b.asset_uid, &nominal, &b.currency)
    }
}

/// Currency exposed to itself, not to RUB it is traded in.
fn from_api_currency(c: ApiCurrency) -> Instrument {
    instrument(
        c.name,
        c.ticker,
        c.asset_uid,
        &c.iso_currency_name,
        &c.currency,
    )
}

fn money_value_amount(mv: &t_invest_sdk::api::MoneyValue) -> Decimal {
    Decimal::from(mv.units) + Decimal::from(mv.nano) / dec!(1_000_000_000)
}

/// Converts an API bond event; conversions and events without a date are skipped.
fn to_bond_event(event: &ApiBondEvent) -> Option<BondEvent> {
    let kind = match event.event_type {
        t if t == EventType::Cpn as i32 => BondEventKind::Coupon,
        t if t == EventType::Call as i32 => BondEventKind::Offer,
        t if t == EventType::Mty as i32 => BondEventKind::Maturity,
        _ => return None,
    };
    let date = to_optional_datetime_utc(event.pay_date.as_ref())
        .or_else(|| to_optional_datetime_utc(event.event_date.as_ref()))?;
    let per_bond =
        to_money(event.pay_one_bond.as_ref()).unwrap_or_else(|| Money::zero(Currency::RUB));
    Some(BondEvent {
        kind,
        date,
        per_bond,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use std::sync::atomic::{AtomicU32, Ordering};
    use t_invest_sdk::api::{
        MoneyValue,
        get_forecast_response::{ConsensusItem, TargetItem},
    };

    #[test]
    fn invest_tls_config_accepts_embedded_cas() {
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(RUSSIAN_TRUSTED_CAS))
            .domain_name("invest-public-api.tbank.ru");

        Channel::from_static(INVEST_API_ENDPOINT)
            .tls_config(tls)
            .expect("TLS config with embedded Russian Trusted CAs should be valid");
    }

    #[test]
    fn accumulate_totals_rub_sums_converted_payments() {
        let totals = TinkoffInvestment::accumulate_totals_rub(
            [
                Money::from_value(dec!(100), Currency::RUB),
                Money::from_value(dec!(50), Currency::RUB),
            ],
            [Money::from_value(dec!(-10), Currency::RUB)],
        );
        assert_eq!(totals.additional_profit.value, dec!(150));
        assert_eq!(totals.fees.value, dec!(-10));
        assert_eq!(totals.additional_profit.currency, Currency::RUB);
    }

    fn status_error(code: Code) -> eyre::Report {
        Err::<(), _>(tonic::Status::new(code, "test"))
            .wrap_err("Failed to call API")
            .unwrap_err()
    }

    fn rate_limit_error(reset: &str) -> eyre::Report {
        let mut metadata = tonic::metadata::MetadataMap::new();
        if let Ok(value) = reset.parse() {
            metadata.insert(RATE_LIMIT_RESET_HEADER, value);
        }
        Err::<(), _>(tonic::Status::with_metadata(
            Code::ResourceExhausted,
            "limit",
            metadata,
        ))
        .wrap_err("Failed to call API")
        .unwrap_err()
    }

    #[rstest]
    #[case(Code::Unavailable, true)]
    #[case(Code::ResourceExhausted, true)]
    #[case(Code::DeadlineExceeded, true)]
    #[case(Code::Internal, true)]
    #[case(Code::Unauthenticated, false)]
    #[case(Code::PermissionDenied, false)]
    #[case(Code::InvalidArgument, false)]
    #[case(Code::NotFound, false)]
    fn is_transient_by_status_code(#[case] code: Code, #[case] expected: bool) {
        // Arrange
        let error = status_error(code);

        // Act
        let transient = is_transient(&error);

        // Assert
        assert_eq!(transient, expected);
    }

    #[test]
    fn is_transient_on_transport_error() {
        // Arrange
        let error = match tonic::transport::Endpoint::from_shared("not a uri") {
            Ok(_) => eyre::eyre!("an invalid URI is accepted"),
            Err(e) => eyre::Report::new(e).wrap_err("Failed to create channel"),
        };

        // Act
        let transient = is_transient(&error);

        // Assert
        assert!(transient);
    }

    #[test]
    fn is_transient_not_on_own_error() {
        // Arrange
        let error = eyre::eyre!("Instrument FIGI not found");

        // Act
        let transient = is_transient(&error);

        // Assert
        assert!(!transient);
    }

    #[tokio::test]
    async fn with_retry_stops_on_own_error() {
        // Arrange
        let calls = AtomicU32::new(0);

        // Act
        let result: color_eyre::Result<()> = with_retry(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(eyre::eyre!("Instrument FIGI not found"))
        })
        .await;

        // Assert
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[rstest]
    #[case("3", Some(Duration::from_secs(3)))]
    #[case(" 7 ", Some(Duration::from_secs(7)))]
    #[case("3600", Some(MAX_SERVER_DELAY))]
    #[case("soon", None)]
    #[case("", None)]
    fn server_retry_delay_from_rate_limit(#[case] reset: &str, #[case] expected: Option<Duration>) {
        // Arrange
        let error = rate_limit_error(reset);

        // Act
        let delay = server_retry_delay(&error);

        // Assert
        assert_eq!(delay, expected);
    }

    #[test]
    fn server_retry_delay_ignores_other_statuses() {
        // Arrange
        let error = status_error(Code::Unavailable);

        // Act
        let delay = server_retry_delay(&error);

        // Assert
        assert_eq!(delay, None);
    }

    #[tokio::test]
    async fn with_retry_stops_on_permanent_error() {
        // Arrange
        let calls = AtomicU32::new(0);

        // Act
        let result: color_eyre::Result<()> = with_retry(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(status_error(Code::Unauthenticated))
        })
        .await;

        // Assert
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn with_retry_repeats_transient_error() {
        // Arrange
        let calls = AtomicU32::new(0);

        // Act
        let result = with_retry(|| async {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(status_error(Code::Unavailable))
            } else {
                Ok(42)
            }
        })
        .await;

        // Assert
        assert_eq!(result.ok(), Some(42));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn position_accrued_interest_from_nkd() {
        // Arrange
        let position = PortfolioPosition {
            quantity: Some(Quotation { units: 2, nano: 0 }),
            average_position_price: Some(rub(1000)),
            current_price: Some(rub(990)),
            current_nkd: Some(rub(15)),
            ..Default::default()
        };

        // Act
        let position = Position::try_from(&position).unwrap();

        // Assert
        assert_eq!(position.accrued_interest.value, dec!(15));
        assert_eq!(position.accrued_interest.currency, Currency::RUB);
    }

    #[test]
    fn position_keeps_daily_yield_and_blocked_lots() {
        // Arrange
        let position = PortfolioPosition {
            quantity: Some(Quotation { units: 2, nano: 0 }),
            average_position_price: Some(rub(1000)),
            current_price: Some(rub(990)),
            daily_yield: Some(rub(-20)),
            blocked: true,
            blocked_lots: Some(Quotation { units: 1, nano: 0 }),
            ..Default::default()
        };

        // Act
        let position = Position::try_from(&position).unwrap();

        // Assert
        assert_eq!(position.daily_yield.value, dec!(-20));
        assert!(position.blocked);
        assert_eq!(position.blocked_lots, dec!(1));
    }

    #[test]
    fn position_daily_yield_zero_when_missing() {
        // Arrange
        let position = PortfolioPosition {
            quantity: Some(Quotation { units: 2, nano: 0 }),
            average_position_price: Some(rub(1000)),
            current_price: Some(rub(990)),
            ..Default::default()
        };

        // Act
        let position = Position::try_from(&position).unwrap();

        // Assert
        assert!(position.daily_yield.value.is_zero());
        assert!(!position.blocked);
        assert!(position.blocked_lots.is_zero());
    }

    #[test]
    fn position_accrued_interest_zero_when_nkd_missing() {
        // Arrange
        let position = PortfolioPosition {
            quantity: Some(Quotation { units: 2, nano: 0 }),
            average_position_price: Some(rub(1000)),
            current_price: Some(rub(990)),
            current_nkd: None,
            ..Default::default()
        };

        // Act
        let position = Position::try_from(&position).unwrap();

        // Assert
        assert!(position.accrued_interest.value.is_zero());
    }

    #[tokio::test]
    async fn paper_for_position_reports_unsupported_type() {
        // Arrange
        let client = TinkoffInvestment::new(String::new());
        let fx = FxBook {
            instruments: HashMap::new(),
            daily: Mutex::new(HashMap::new()),
        };
        let instruments = HashMap::from([(
            "OPT1".to_string(),
            Instrument {
                name: "Option".to_string(),
                ticker: Ticker::new("OPTX"),
                currency: Some(Currency::RUB),
                asset_uid: None,
                sector: None,
            },
        )]);
        let position = PortfolioPosition {
            figi: "OPT1".to_string(),
            instrument_type: "option".to_string(),
            ..Default::default()
        };

        // Act
        let result = client
            .paper_for_position(&instruments, "account", &position, &fx, true)
            .await;

        // Assert
        let message = format!("{:#}", result.err().unwrap());
        assert!(message.contains("OPTX (OPT1) skipped"), "{message}");
        assert!(
            message.contains("Unsupported instrument type 'option'"),
            "{message}"
        );
    }

    #[test]
    fn instrument_or_figi_returns_known_instrument() {
        // Arrange
        let instruments = HashMap::from([(
            "FIGI1".to_string(),
            Instrument {
                name: "Sber".to_string(),
                ticker: Ticker::new("SBER"),
                currency: Some(Currency::RUB),
                asset_uid: None,
                sector: None,
            },
        )]);

        // Act
        let instrument = instrument_or_figi(&instruments, "FIGI1");

        // Assert
        assert_eq!(instrument.name, "Sber");
        assert_eq!(instrument.ticker.as_str(), "SBER");
    }

    #[test]
    fn instrument_or_figi_falls_back_to_figi() {
        // Arrange
        let instruments = HashMap::new();

        // Act
        let instrument = instrument_or_figi(&instruments, "FIGI2");

        // Assert
        assert_eq!(instrument.name, "FIGI2");
        assert_eq!(instrument.ticker.as_str(), "FIGI2");
    }

    fn positions(figis: &[&str]) -> Vec<PortfolioPosition> {
        figis
            .iter()
            .map(|figi| PortfolioPosition {
                figi: (*figi).to_string(),
                ..Default::default()
            })
            .collect()
    }

    #[tokio::test]
    async fn fetch_parallel_keeps_failed_positions_as_errors() {
        // Arrange
        let client = TinkoffInvestment::new(String::new());
        let positions = positions(&["OK", "BAD"]);

        // Act
        let results = client
            .fetch_parallel(&positions, |_, figi| async move {
                if figi == "BAD" {
                    Err(status_error(Code::InvalidArgument))
                } else {
                    Ok(vec![figi])
                }
            })
            .await;

        // Assert
        let results: HashMap<_, _> = results
            .into_iter()
            .map(|(position, items)| (position.figi, items))
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results["OK"].as_ref().ok(), Some(&vec!["OK".to_string()]));
        assert!(results["BAD"].is_err());
    }

    #[tokio::test]
    async fn fetch_parallel_retries_rate_limited_requests() {
        // Arrange
        let client = TinkoffInvestment::new(String::new());
        let positions = positions(&["FIGI"]);
        let calls = Arc::new(AtomicU32::new(0));

        // Act
        let results = client
            .fetch_parallel(&positions, {
                let calls = Arc::clone(&calls);
                move |_, figi| {
                    let calls = Arc::clone(&calls);
                    async move {
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            Err(rate_limit_error("0"))
                        } else {
                            Ok(vec![figi])
                        }
                    }
                }
            })
            .await;

        // Assert
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(results[0].1.is_ok());
    }

    #[test]
    fn skipped_names_position() {
        // Arrange
        let instrument = Instrument {
            name: "Sber".to_string(),
            ticker: Ticker::new("SBER"),
            currency: Some(Currency::RUB),
            asset_uid: None,
            sector: None,
        };

        // Act
        let error = skipped(eyre::eyre!("boom"), &instrument, "FIGI1");

        // Assert
        assert_eq!(format!("{error:#}"), "Position SBER (FIGI1) skipped: boom");
    }

    #[rstest]
    #[case::first_long_wait(None, 42, 42, true)]
    #[case::short_wait(None, 0, 0, false)]
    #[case::same_window(Some(42), 42, 42, false)]
    #[case::within_a_second(Some(42), 43, 43, false)]
    #[case::next_window(Some(42), 100, 58, true)]
    fn should_announce_wait_once_per_window(
        #[case] last_after: Option<u64>,
        #[case] until_after: u64,
        #[case] wait: u64,
        #[case] expected: bool,
    ) {
        // Arrange
        let now = Instant::now();
        let last = last_after.map(|secs| now + Duration::from_secs(secs));
        let until = now + Duration::from_secs(until_after);

        // Act
        let announce = should_announce_wait(last, until, Duration::from_secs(wait));

        // Assert
        assert_eq!(announce, expected);
    }

    fn position_in(currency: Currency) -> Position {
        Position {
            currency,
            average_buy_price: Money::from_value(dec!(100), currency),
            current_instrument_price: Money::from_value(dec!(110), currency),
            accrued_interest: Money::zero(currency),
            quantity: dec!(1),
            daily_yield: Money::zero(currency),
            blocked: false,
            blocked_lots: dec!(0),
        }
    }

    #[test]
    fn ensure_rub_accepts_rub_prices() {
        // Arrange
        let position = position_in(Currency::RUB);

        // Act
        let result = ensure_rub(&position);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn ensure_rub_rejects_foreign_prices() {
        // Arrange
        let position = position_in(Currency::USD);

        // Act
        let result = ensure_rub(&position);

        // Assert
        let message = format!("{:#}", result.err().unwrap());
        assert_eq!(message, "Expected position prices in RUB, got USD");
    }

    fn consensus(currency: &str, target: i64) -> GetForecastResponse {
        GetForecastResponse {
            targets: vec![],
            consensus: Some(ConsensusItem {
                recommendation: ApiRecommendation::Buy as i32,
                currency: currency.to_string(),
                current_price: Some(Quotation {
                    units: 100,
                    nano: 0,
                }),
                consensus: Some(Quotation {
                    units: target,
                    nano: 0,
                }),
                min_target: Some(Quotation { units: 90, nano: 0 }),
                max_target: Some(Quotation {
                    units: 150,
                    nano: 0,
                }),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn to_forecast_maps_consensus() {
        // Arrange
        let response = consensus("usd", 120);

        // Act
        let forecast = to_forecast(&response, Some(Currency::RUB)).unwrap();

        // Assert
        assert_eq!(forecast.recommendation, Some(Recommendation::Buy));
        assert_eq!(
            forecast.current_price,
            Money::from_value(dec!(100), Currency::USD)
        );
        assert_eq!(forecast.target_price.value, dec!(120));
        assert_eq!(forecast.min_target.value, dec!(90));
        assert_eq!(forecast.max_target.value, dec!(150));
        assert_eq!(forecast.analysts, 0);
    }

    #[test]
    fn to_forecast_takes_currency_from_targets() {
        // Arrange
        let mut response = consensus("", 120);
        response.targets = vec![TargetItem {
            currency: "eur".to_string(),
            ..Default::default()
        }];

        // Act
        let forecast = to_forecast(&response, Some(Currency::RUB)).unwrap();

        // Assert
        assert_eq!(forecast.target_price.currency, Currency::EUR);
        assert_eq!(forecast.analysts, 1);
    }

    #[rstest]
    #[case::fallback(Some(Currency::RUB), Some(Currency::RUB))]
    #[case::unknown(None, None)]
    fn to_forecast_uses_fallback_currency(
        #[case] fallback: Option<Currency>,
        #[case] expected: Option<Currency>,
    ) {
        // Arrange
        let response = consensus("", 120);

        // Act
        let forecast = to_forecast(&response, fallback);

        // Assert
        assert_eq!(forecast.map(|f| f.target_price.currency), expected);
    }

    #[rstest]
    #[case::no_consensus(GetForecastResponse::default())]
    #[case::zero_target(consensus("rub", 0))]
    fn to_forecast_without_target(#[case] response: GetForecastResponse) {
        // Arrange

        // Act
        let forecast = to_forecast(&response, Some(Currency::RUB));

        // Assert
        assert_eq!(forecast, None);
    }

    #[test]
    fn to_fundamentals_maps_known_metrics() {
        // Arrange
        let statistic = StatisticResponse {
            pe_ratio_ttm: 3.194,
            price_to_book_ttm: 0.7,
            roe: 23.44,
            dividend_yield_daily_ttm: 13.61,
            beta: 0.55,
            ..Default::default()
        };

        // Act
        let fundamentals = to_fundamentals(&statistic);

        // Assert
        assert_eq!(
            fundamentals,
            Fundamentals {
                pe: Some(dec!(3.19)),
                pb: Some(dec!(0.7)),
                ev_to_ebitda: None,
                net_debt_to_ebitda: None,
                roe: Some(dec!(23.44)),
                dividend_yield: Some(dec!(13.61)),
                beta: Some(dec!(0.55)),
            }
        );
    }

    #[rstest]
    #[case(ApiRecommendation::Buy, Some(Recommendation::Buy))]
    #[case(ApiRecommendation::Hold, Some(Recommendation::Hold))]
    #[case(ApiRecommendation::Sell, Some(Recommendation::Sell))]
    #[case(ApiRecommendation::Unspecified, None)]
    fn to_recommendation_cases(
        #[case] recommendation: ApiRecommendation,
        #[case] expected: Option<Recommendation>,
    ) {
        // Arrange

        // Act
        let result = to_recommendation(recommendation);

        // Assert
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::replacement_bond("usd", Currency::USD)]
    #[case::rub_bond("rub", Currency::RUB)]
    #[case::unknown_nominal("", Currency::RUB)]
    fn bond_instrument_is_exposed_to_nominal_currency(
        #[case] nominal: &str,
        #[case] expected: Currency,
    ) {
        // Arrange
        let bond = ApiBond {
            name: "Bond".to_string(),
            ticker: "BND".to_string(),
            currency: "rub".to_string(),
            nominal: Some(t_invest_sdk::api::MoneyValue {
                units: 1000,
                nano: 0,
                currency: nominal.to_string(),
            }),
            ..Default::default()
        };

        // Act
        let instrument = from_api_bond(bond);

        // Assert
        assert_eq!(instrument.currency, Some(expected));
    }

    #[test]
    fn currency_instrument_is_exposed_to_its_currency() {
        // Arrange
        let currency = ApiCurrency {
            name: "Доллар США".to_string(),
            ticker: "USD000UTSTOM".to_string(),
            currency: "rub".to_string(),
            iso_currency_name: "usd".to_string(),
            asset_uid: "uid".to_string(),
            ..Default::default()
        };

        // Act
        let instrument = from_api_currency(currency);

        // Assert
        assert_eq!(instrument.currency, Some(Currency::USD));
        assert_eq!(instrument.ticker, Ticker::new("USD000UTSTOM"));
        assert_eq!(instrument.asset_uid.as_deref(), Some("uid"));
    }

    #[test]
    fn share_instrument_is_exposed_to_trading_currency() {
        // Arrange
        let share = ApiInstrument {
            name: "Share".to_string(),
            ticker: "SHR".to_string(),
            currency: "usd".to_string(),
            ..Default::default()
        };

        // Act
        let instrument = from_api_instrument(share);

        // Assert
        assert_eq!(instrument.currency, Some(Currency::USD));
        assert_eq!(instrument.asset_uid, None);
    }

    #[test]
    fn share_instrument_has_sector() {
        // Arrange
        let share = ApiShare {
            name: "Sber".to_string(),
            ticker: "SBER".to_string(),
            currency: "rub".to_string(),
            sector: "financial".to_string(),
            ..Default::default()
        };

        // Act
        let instrument = from_api_share(share);

        // Assert
        assert_eq!(instrument.sector.as_deref(), Some("financial"));
        assert_eq!(instrument.currency, Some(Currency::RUB));
    }

    #[test]
    fn bond_instrument_has_sector() {
        // Arrange
        let bond = ApiBond {
            name: "OFZ".to_string(),
            ticker: "SU26238RMFS4".to_string(),
            currency: "rub".to_string(),
            sector: "government".to_string(),
            ..Default::default()
        };

        // Act
        let instrument = from_api_bond(bond);

        // Assert
        assert_eq!(instrument.sector.as_deref(), Some("government"));
    }

    #[rstest]
    #[case::lower("energy", Some("energy"))]
    #[case::mixed_case_and_spaces(" Health_Care ", Some("health_care"))]
    #[case::empty("", None)]
    #[case::blank("  ", None)]
    fn sector_is_normalized(#[case] code: &str, #[case] expected: Option<&str>) {
        // Act
        let result = sector(code);

        // Assert
        assert_eq!(result.as_deref(), expected);
    }

    #[test]
    fn instrument_or_figi_fallback_has_no_currency() {
        // Arrange
        let instruments = HashMap::new();

        // Act
        let instrument = instrument_or_figi(&instruments, "FIGI3");

        // Assert
        assert_eq!(instrument.currency, None);
    }

    fn account(id: &str, account_type: AccountType, status: AccountStatus) -> Account {
        Account {
            id: id.to_string(),
            name: format!("Account {id}"),
            r#type: account_type as i32,
            status: status as i32,
            ..Default::default()
        }
    }

    fn accounts() -> Vec<Account> {
        vec![
            account("B1", AccountType::Tinkoff, AccountStatus::Open),
            account("B2", AccountType::Tinkoff, AccountStatus::Open),
            account("I1", AccountType::TinkoffIis, AccountStatus::Open),
            account("I0", AccountType::TinkoffIis, AccountStatus::Closed),
        ]
    }

    #[rstest]
    #[case::only_open_of_type(AccountSelector::Type(AccountType::TinkoffIis), "I1")]
    #[case::by_id(AccountSelector::Id("B2".to_string()), "B2")]
    #[case::closed_by_id(AccountSelector::Id("I0".to_string()), "I0")]
    fn select_account_picks_single_match(
        #[case] selector: AccountSelector,
        #[case] expected_id: &str,
    ) {
        // Arrange
        let accounts = accounts();

        // Act
        let account = select_account(&accounts, &selector).unwrap();

        // Assert
        assert_eq!(account.id, expected_id);
    }

    #[rstest]
    #[case::several_of_type(
        AccountSelector::Type(AccountType::Tinkoff),
        "Several accounts match: B1 (Account B1, tinkoff), B2 (Account B2, tinkoff); \
         choose one with --account-id"
    )]
    #[case::no_open_of_type(
        AccountSelector::Type(AccountType::InvestBox),
        "No open invest-box account; available accounts: B1 (Account B1, tinkoff), \
         B2 (Account B2, tinkoff), I1 (Account I1, iis), I0 (Account I0, iis)"
    )]
    #[case::unknown_id(
        AccountSelector::Id("X".to_string()),
        "No account with ID X; available accounts: B1 (Account B1, tinkoff), \
         B2 (Account B2, tinkoff), I1 (Account I1, iis), I0 (Account I0, iis)"
    )]
    fn select_account_rejects_ambiguous_or_missing(
        #[case] selector: AccountSelector,
        #[case] expected: &str,
    ) {
        // Arrange
        let accounts = accounts();

        // Act
        let error = select_account(&accounts, &selector).unwrap_err();

        // Assert
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn select_account_without_accounts() {
        // Arrange
        let accounts: Vec<Account> = vec![];

        // Act
        let error =
            select_account(&accounts, &AccountSelector::Type(AccountType::Tinkoff)).unwrap_err();

        // Assert
        assert_eq!(
            error.to_string(),
            "No open tinkoff account; available accounts: none"
        );
    }

    #[rstest]
    #[case::yesterday("2026-09-25T23:59:59Z", false)]
    #[case::earlier_today("2026-09-26T00:00:00Z", true)]
    #[case::later_today("2026-09-26T23:00:00Z", true)]
    #[case::tomorrow("2026-09-27T00:00:00Z", true)]
    #[case::last_day("2026-10-06T23:00:00Z", true)]
    #[case::after_last_day("2026-10-07T00:00:00Z", false)]
    fn calendar_period_contains_whole_days(#[case] date: &str, #[case] expected: bool) {
        // Arrange
        let from: DateTime<Utc> = "2026-09-26T12:00:00Z".parse().unwrap();
        let period = CalendarPeriod::days_ahead(from, 10);
        let date: DateTime<Utc> = date.parse().unwrap();

        // Act
        let contains = period.contains(date);

        // Assert
        assert_eq!(contains, expected);
    }

    fn trade(
        instrument_type: &str,
        operation_type: OperationType,
        accrued_int: Option<i64>,
    ) -> OperationItem {
        OperationItem {
            instrument_type: instrument_type.to_string(),
            r#type: operation_type as i32,
            accrued_int: accrued_int.map(rub),
            ..Default::default()
        }
    }

    #[rstest]
    #[case::buy_pays_nkd(OperationType::Buy, Some(19), Some(dec!(-19)))]
    #[case::buy_card_pays_nkd(OperationType::BuyCard, Some(19), Some(dec!(-19)))]
    #[case::sell_receives_nkd(OperationType::Sell, Some(19), Some(dec!(19)))]
    #[case::buy_without_nkd(OperationType::Buy, None, None)]
    #[case::zero_nkd(OperationType::Buy, Some(0), None)]
    #[case::coupon_is_not_a_trade(OperationType::Coupon, Some(19), None)]
    fn trade_accrued_interest_of_bond(
        #[case] operation_type: OperationType,
        #[case] accrued_int: Option<i64>,
        #[case] expected: Option<Decimal>,
    ) {
        // Arrange
        let op = trade("bond", operation_type, accrued_int);

        // Act
        let accrued = trade_accrued_interest(&op);

        // Assert
        assert_eq!(accrued.map(|m| m.value), expected);
    }

    #[test]
    fn trade_accrued_interest_ignores_shares() {
        // Arrange
        let op = trade("share", OperationType::Buy, Some(19));

        // Act
        let accrued = trade_accrued_interest(&op);

        // Assert
        assert!(accrued.is_none());
    }

    #[test]
    fn year_bounds_cover_whole_year() {
        // Arrange
        let year = 2024;

        // Act
        let (from, to) = year_bounds(year).unwrap();

        // Assert
        assert_eq!(from.to_rfc3339(), "2024-01-01T00:00:00+00:00");
        assert_eq!(to.to_rfc3339(), "2025-01-01T00:00:00+00:00");
    }

    fn candle(time: &str, close_units: i64) -> HistoricCandle {
        let time: DateTime<Utc> = time.parse().unwrap();
        HistoricCandle {
            time: Some(prost_types::Timestamp {
                seconds: time.timestamp(),
                nanos: 0,
            }),
            close: Some(Quotation {
                units: close_units,
                nano: 0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn daily_rates_keyed_by_candle_date() {
        // Arrange
        let instr = FxInstrument {
            instrument_id: "USD".to_string(),
            lot: 1,
            nominal: dec!(1),
        };
        let candles = [
            candle("2026-09-24T07:00:00Z", 90),
            candle("2026-09-25T07:00:00Z", 91),
            HistoricCandle::default(),
        ];

        // Act
        let rates = daily_rates(&candles, &instr);

        // Assert
        let expected: DailyRates = [
            (NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(), dec!(90)),
            (NaiveDate::from_ymd_opt(2026, 9, 25).unwrap(), dec!(91)),
        ]
        .into();
        assert_eq!(rates, expected);
    }

    #[test]
    fn daily_rates_apply_lot_and_nominal() {
        // Arrange
        let instr = FxInstrument {
            instrument_id: "KZT".to_string(),
            lot: 1,
            nominal: dec!(100),
        };
        let candles = [candle("2026-09-25T07:00:00Z", 18)];

        // Act
        let rates = daily_rates(&candles, &instr);

        // Assert
        assert_eq!(
            rates.get(&NaiveDate::from_ymd_opt(2026, 9, 25).unwrap()),
            Some(&dec!(0.18))
        );
    }

    fn api_event(event_type: EventType, pay_date: Option<i64>, event_date: i64) -> ApiBondEvent {
        let ts = |seconds| prost_types::Timestamp { seconds, nanos: 0 };
        ApiBondEvent {
            event_type: event_type as i32,
            pay_date: pay_date.map(ts),
            event_date: Some(ts(event_date)),
            pay_one_bond: Some(rub(30)),
            ..Default::default()
        }
    }

    #[rstest]
    #[case::coupon(EventType::Cpn, Some(BondEventKind::Coupon))]
    #[case::offer(EventType::Call, Some(BondEventKind::Offer))]
    #[case::maturity(EventType::Mty, Some(BondEventKind::Maturity))]
    #[case::conversion(EventType::Conv, None)]
    fn to_bond_event_maps_kind(
        #[case] event_type: EventType,
        #[case] expected: Option<BondEventKind>,
    ) {
        // Arrange
        let event = api_event(event_type, Some(1_800_000_000), 1_799_900_000);

        // Act
        let converted = to_bond_event(&event);

        // Assert
        assert_eq!(converted.map(|e| e.kind), expected);
    }

    #[test]
    fn to_bond_event_prefers_pay_date() {
        // Arrange
        let event = api_event(EventType::Cpn, Some(1_800_000_000), 1_799_900_000);

        // Act
        let converted = to_bond_event(&event).unwrap();

        // Assert
        assert_eq!(converted.date.timestamp(), 1_800_000_000);
        assert_eq!(converted.per_bond.value, dec!(30));
    }

    #[test]
    fn to_bond_event_falls_back_to_event_date() {
        // Arrange
        let event = api_event(EventType::Mty, None, 1_799_900_000);

        // Act
        let converted = to_bond_event(&event).unwrap();

        // Assert
        assert_eq!(converted.date.timestamp(), 1_799_900_000);
    }

    fn rub(units: i64) -> MoneyValue {
        MoneyValue {
            currency: "rub".to_string(),
            units,
            nano: 0,
        }
    }

    #[test]
    fn money_to_rub_via_rate_matches_spot_math() {
        let usd = Money::from_value(dec!(2), Currency::USD);
        let rub = usd.to_currency(quote_to_rub_rate(dec!(90), 1, dec!(1)), Currency::RUB);
        assert_eq!(rub.value, dec!(180));
        assert_eq!(rub.currency, Currency::RUB);
    }
}
