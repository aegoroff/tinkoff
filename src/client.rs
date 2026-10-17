use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use color_eyre::eyre::{self, WrapErr};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use t_invest_sdk::{
    TInvestInterceptor,
    api::{
        Account, AccountStatus, Bond as ApiBond, CandleInterval, Currency as ApiCurrency, Dividend,
        Etf as ApiEtf, FindInstrumentRequest, Future as ApiFuture, GetAccountsRequest,
        GetAccountsResponse, GetAssetFundamentalsRequest, GetBondEventsRequest, GetCandlesRequest,
        GetDividendsRequest, GetForecastRequest, GetForecastResponse, GetOperationsByCursorRequest,
        GetOperationsByCursorResponse, HistoricCandle, IndicativeResponse, IndicativesRequest,
        Instrument as ApiInstrument, InstrumentIdType, InstrumentRequest, InstrumentShort,
        InstrumentStatus, InstrumentType, InstrumentsRequest, OperationItem, OperationState,
        PortfolioPosition, PortfolioRequest, Quotation, Recommendation as ApiRecommendation,
        Share as ApiShare, get_asset_fundamentals_response::StatisticResponse,
        get_bond_events_request::EventType, get_bond_events_response::BondEvent as ApiBondEvent,
        instruments_service_client::InstrumentsServiceClient,
        market_data_service_client::MarketDataServiceClient,
        operations_service_client::OperationsServiceClient, portfolio_request::CurrencyRequest,
        users_service_client::UsersServiceClient,
    },
};
use tokio::sync::OnceCell;
use tokio::time::{Duration, Instant, sleep};
use tonic::Code;
use tonic::transport::{Certificate, Channel, ClientTlsConfig};

/// Prod T-Invest API endpoint (official `tbank.ru` host).
const INVEST_API_ENDPOINT: &str = "https://invest-public-api.tbank.ru:443/";

/// Russian Trusted Root CA PEM (required for T-Bank TLS).
const RUSSIAN_TRUSTED_CAS: &[u8] = include_bytes!("../certs/russian_trusted_cas.pem");

use crate::{
    api::{InvestApi, OperationsFilter},
    domain::{
        Instrument, InstrumentKind, Money, Ticker,
        analytics::{Forecast, Fundamentals, HouseForecast, Recommendation},
        bond::{BondEvent, BondEventKind, mark_amortizations},
        fx::{DailyCloses, DailyPrices, FxCandidate},
        moscow_date,
    },
    to_decimal, to_metric, to_money, to_optional_datetime_utc,
};

/// How far ahead bond events are requested; covers the longest bond maturities.
const BOND_EVENTS_HORIZON_DAYS: i64 = 50 * 365;

/// Most assets accepted by a single fundamentals request.
const MAX_FUNDAMENTALS_ASSETS: usize = 100;

/// Operations requested per page, the maximum the API allows.
const OPERATIONS_PAGE_SIZE: i32 = 1000;

#[derive(Clone)]
pub struct TinkoffInvestment {
    interceptor: TInvestInterceptor,
    /// Shared TLS channel; connected once, then cloned (cheap handle).
    channel: Arc<OnceCell<Channel>>,
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

    /// Looks up an instrument by FIGI. Bonds and currencies are requested by their own
    /// methods, as only these return the nominal currency the position is exposed to;
    /// shares, ETFs and futures too, as only these return the sector.
    async fn get_instrument(
        &self,
        figi: String,
        kind: Option<InstrumentKind>,
    ) -> color_eyre::Result<Instrument> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let request = InstrumentRequest {
            id_type: InstrumentIdType::Figi as i32,
            class_code: None,
            id: figi.clone(),
        };
        let instrument = match kind {
            Some(InstrumentKind::Bond) => instruments
                .bond_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_bond)),
            Some(InstrumentKind::Currency) => instruments
                .currency_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_currency)),
            Some(InstrumentKind::Share) => instruments
                .share_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_share)),
            Some(InstrumentKind::Etf) => instruments
                .etf_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_etf)),
            Some(InstrumentKind::Future) => instruments
                .future_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_future)),
            None => instruments
                .get_instrument_by(request)
                .await
                .map(|r| r.into_inner().instrument.map(from_api_instrument)),
        }
        .wrap_err_with(|| format!("Failed to get instrument {figi}"))?;
        instrument.ok_or_else(|| eyre::eyre!("Instrument {figi} not found"))
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

    async fn get_portfolio_positions(
        &self,
        account_id: &str,
    ) -> color_eyre::Result<Vec<PortfolioPosition>> {
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
        Ok(portfolio.into_inner().positions)
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

    /// Daily candles of an instrument within a calendar year.
    async fn get_year_candles(
        &self,
        instrument_id: &str,
        year: i32,
    ) -> color_eyre::Result<Vec<HistoricCandle>> {
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
                instrument_id: Some(instrument_id.to_string()),
                ..Default::default()
            })
            .await
            .wrap_err("Failed to get candles")?;
        Ok(response.into_inner().candles)
    }

    async fn get_indicatives(&self) -> color_eyre::Result<Vec<IndicativeResponse>> {
        let mut instruments = self
            .client(InstrumentsServiceClient::with_interceptor)
            .await?;
        let response = instruments
            .indicatives(IndicativesRequest {})
            .await
            .wrap_err("Failed to get indices")?;
        Ok(response.into_inner().instruments)
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

    /// Currency instruments from the Currencies catalog.
    async fn get_fx_candidates(&self) -> color_eyre::Result<Vec<FxCandidate>> {
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

        Ok(response
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
            })
            .collect())
    }
}

impl InvestApi for TinkoffInvestment {
    async fn accounts(&self) -> color_eyre::Result<Vec<Account>> {
        let response = with_retry(|| self.get_accounts_response()).await?;
        Ok(response.accounts)
    }

    async fn portfolio_positions(
        &self,
        account_id: &str,
    ) -> color_eyre::Result<Vec<PortfolioPosition>> {
        with_retry(|| self.get_portfolio_positions(account_id)).await
    }

    async fn instrument(
        &self,
        figi: &str,
        kind: Option<InstrumentKind>,
    ) -> color_eyre::Result<Instrument> {
        with_retry(|| self.get_instrument(figi.to_string(), kind)).await
    }

    async fn find_instruments(&self, query: &str) -> color_eyre::Result<Vec<InstrumentShort>> {
        with_retry(|| self.find_instruments_by_ticker(query.to_string())).await
    }

    async fn operations(
        &self,
        account_id: &str,
        filter: OperationsFilter<'_>,
    ) -> color_eyre::Result<Vec<OperationItem>> {
        let (instrument_id, operation_types) = match filter {
            OperationsFilter::Instrument(figi) => (Some(figi.to_string()), Vec::new()),
            OperationsFilter::Types(types) => (None, types.iter().map(|t| *t as i32).collect()),
        };
        self.get_all_operations(GetOperationsByCursorRequest {
            account_id: account_id.to_string(),
            instrument_id,
            operation_types,
            ..Default::default()
        })
        .await
    }

    async fn forecast(
        &self,
        instrument_uid: &str,
        currency: Option<Currency>,
    ) -> color_eyre::Result<Option<Forecast>> {
        with_retry(|| self.get_forecast(instrument_uid.to_string(), currency)).await
    }

    /// Requested in batches of [`MAX_FUNDAMENTALS_ASSETS`].
    async fn fundamentals(
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

    async fn dividends(&self, figi: &str) -> color_eyre::Result<Vec<Dividend>> {
        with_retry(|| self.get_dividends_for_figi(figi.to_string())).await
    }

    async fn bond_events(&self, figi: &str) -> color_eyre::Result<Vec<BondEvent>> {
        with_retry(|| self.get_bond_events(figi.to_string())).await
    }

    async fn fx_candidates(&self) -> color_eyre::Result<Vec<FxCandidate>> {
        with_retry(|| self.get_fx_candidates()).await
    }

    async fn indices(&self) -> color_eyre::Result<Vec<IndicativeResponse>> {
        with_retry(|| self.get_indicatives()).await
    }
}

impl DailyCloses for TinkoffInvestment {
    async fn daily_closes(
        &self,
        instrument_id: &str,
        year: i32,
    ) -> color_eyre::Result<DailyPrices> {
        let candles = with_retry(|| self.get_year_candles(instrument_id, year)).await?;
        Ok(daily_closes(&candles))
    }
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

/// Maps daily candles to closes by Moscow date of a candle; the last candle of a date wins.
/// Candles without a time or a close are skipped.
fn daily_closes(candles: &[HistoricCandle]) -> DailyPrices {
    candles
        .iter()
        .filter_map(|c| {
            let date = moscow_date(to_optional_datetime_utc(c.time.as_ref())?);
            Some((date, to_decimal(Some(c.close.as_ref()?))))
        })
        .collect()
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
    let mut targets: Vec<HouseForecast> = response
        .targets
        .iter()
        .map(|t| HouseForecast {
            company: t.company.clone(),
            recommendation: to_recommendation(t.recommendation()),
            date: to_optional_datetime_utc(t.recommendation_date.as_ref()),
            target_price: money(t.target_price.as_ref()),
        })
        .collect();
    targets.sort_by_key(|t| std::cmp::Reverse(t.date));
    let forecast = Forecast {
        recommendation: to_recommendation(consensus.recommendation()),
        current_price: money(consensus.current_price.as_ref()),
        target_price: money(consensus.consensus.as_ref()),
        min_target: money(consensus.min_target.as_ref()),
        max_target: money(consensus.max_target.as_ref()),
        targets,
    };
    Some(forecast).filter(|f| !f.target_price.value.is_zero())
}

fn to_fundamentals(statistic: &StatisticResponse) -> Fundamentals {
    let currency = Currency::from_code(&statistic.currency.to_ascii_uppercase());
    // Amounts of an unknown currency are dropped rather than shown in a wrong one.
    let money = |value: f64| Some(Money::from_value(to_metric(value)?, currency?));
    Fundamentals {
        pe: to_metric(statistic.pe_ratio_ttm),
        pb: to_metric(statistic.price_to_book_ttm),
        ev_to_ebitda: to_metric(statistic.ev_to_ebitda_mrq),
        net_debt_to_ebitda: to_metric(statistic.net_debt_to_ebitda),
        roe: to_metric(statistic.roe),
        dividend_yield: to_metric(statistic.dividend_yield_daily_ttm),
        beta: to_metric(statistic.beta),
        market_cap: money(statistic.market_capitalization),
        enterprise_value: money(statistic.total_enterprise_value_mrq),
        ps: to_metric(statistic.price_to_sales_ttm),
        price_to_fcf: to_metric(statistic.price_to_free_cash_flow_ttm),
        revenue: money(statistic.revenue_ttm),
        ebitda: money(statistic.ebitda_ttm),
        net_income: money(statistic.net_income_ttm),
        free_cash_flow: money(statistic.free_cash_flow_ttm),
        eps: money(statistic.eps_ttm),
        revenue_growth: to_metric(statistic.one_year_annual_revenue_growth_rate),
        net_margin: to_metric(statistic.net_margin_mrq),
        roa: to_metric(statistic.roa),
        total_debt: money(statistic.total_debt_mrq),
        debt_to_equity: to_metric(statistic.total_debt_to_equity_mrq),
        dividends_per_share: money(statistic.dividends_per_share),
        five_year_dividend_yield: to_metric(statistic.five_years_average_dividend_yield),
        payout_ratio: to_metric(statistic.dividend_payout_ratio_fy),
        ex_dividend_date: to_optional_datetime_utc(statistic.ex_dividend_date.as_ref()),
        // The API gives a fraction, other shares are in percent.
        free_float: to_metric(statistic.free_float * 100.0),
        low_52_weeks: money(statistic.low_price_last_52_weeks),
        high_52_weeks: money(statistic.high_price_last_52_weeks),
    }
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

    fn rub(units: i64) -> MoneyValue {
        MoneyValue {
            currency: "rub".to_string(),
            units,
            nano: 0,
        }
    }

    #[test]
    fn invest_tls_config_accepts_embedded_cas() {
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(RUSSIAN_TRUSTED_CAS))
            .domain_name("invest-public-api.tbank.ru");

        Channel::from_static(INVEST_API_ENDPOINT)
            .tls_config(tls)
            .expect("TLS config with embedded Russian Trusted CAs should be valid");
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
        assert_eq!(forecast.analysts(), 0);
    }

    #[test]
    fn to_forecast_keeps_house_forecasts_latest_first() {
        // Arrange
        let target = |company: &str, seconds: i64, price: i64| TargetItem {
            company: company.to_string(),
            recommendation: ApiRecommendation::Hold as i32,
            recommendation_date: Some(prost_types::Timestamp { seconds, nanos: 0 }),
            target_price: Some(Quotation {
                units: price,
                nano: 0,
            }),
            ..Default::default()
        };
        let mut response = consensus("rub", 120);
        response.targets = vec![target("Old", 1_000, 110), target("New", 2_000, 130)];

        // Act
        let forecast = to_forecast(&response, None).unwrap();

        // Assert
        let companies: Vec<&str> = forecast
            .targets
            .iter()
            .map(|t| t.company.as_str())
            .collect();
        assert_eq!(companies, ["New", "Old"]);
        let latest = &forecast.targets[0];
        assert_eq!(latest.recommendation, Some(Recommendation::Hold));
        assert_eq!(
            latest.target_price,
            Money::from_value(dec!(130), Currency::RUB)
        );
        assert_eq!(forecast.upside_to(latest.target_price), Some(dec!(30)));
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
        assert_eq!(forecast.analysts(), 1);
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
                ..Fundamentals::default()
            }
        );
    }

    #[test]
    fn to_fundamentals_maps_amounts_in_reporting_currency() {
        // Arrange
        let statistic = StatisticResponse {
            currency: "RUB".to_string(),
            market_capitalization: 3_722_074_873_464.0,
            net_income_ttm: -908_369_000_000.0,
            free_float: 0.59,
            ..Default::default()
        };

        // Act
        let fundamentals = to_fundamentals(&statistic);

        // Assert
        let rub = |value| Some(Money::from_value(value, Currency::RUB));
        assert_eq!(fundamentals.market_cap, rub(dec!(3722074873464)));
        assert_eq!(fundamentals.net_income, rub(dec!(-908369000000)));
        assert_eq!(fundamentals.free_float, Some(dec!(59)));
        assert_eq!(fundamentals.revenue, None);
    }

    #[test]
    fn to_fundamentals_drops_amounts_of_unknown_currency() {
        // Arrange
        let statistic = StatisticResponse {
            currency: String::new(),
            market_capitalization: 1_000_000.0,
            pe_ratio_ttm: 5.0,
            ..Default::default()
        };

        // Act
        let fundamentals = to_fundamentals(&statistic);

        // Assert
        assert_eq!(fundamentals.market_cap, None);
        assert_eq!(fundamentals.pe, Some(dec!(5)));
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

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn daily_closes_keyed_by_candle_date() {
        // Arrange
        let candles = [
            candle("2026-09-24T07:00:00Z", 90),
            candle("2026-09-25T07:00:00Z", 91),
            HistoricCandle::default(),
        ];

        // Act
        let closes = daily_closes(&candles);

        // Assert
        let expected: DailyPrices =
            [(day(2026, 9, 24), dec!(90)), (day(2026, 9, 25), dec!(91))].into();
        assert_eq!(closes, expected);
    }

    #[test]
    fn daily_closes_keyed_by_moscow_date() {
        // Arrange
        let candles = [candle("2026-09-24T21:00:00Z", 90)];

        // Act
        let closes = daily_closes(&candles);

        // Assert
        let expected: DailyPrices = [(day(2026, 9, 25), dec!(90))].into();
        assert_eq!(closes, expected);
    }

    #[test]
    fn daily_closes_skip_candles_without_close() {
        // Arrange
        let candles = [HistoricCandle {
            close: None,
            ..candle("2026-09-25T07:00:00Z", 90)
        }];

        // Act
        let closes = daily_closes(&candles);

        // Assert
        assert!(closes.is_empty());
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
}
