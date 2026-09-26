use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use color_eyre::eyre::{self, WrapErr};
use iso_currency::Currency;
use itertools::Itertools;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tinkoff_invest_api::{
    TinkoffInvestService,
    tcs::{
        Account, AccountType, CandleInterval, Coupon, Dividend, FindInstrumentRequest,
        GetAccountsRequest, GetAccountsResponse, GetBondCouponsRequest, GetCandlesRequest,
        GetDividendsRequest, GetLastPricesRequest, InstrumentIdType, InstrumentRequest,
        InstrumentShort, InstrumentStatus, InstrumentType, InstrumentsRequest, Operation,
        OperationState, OperationType, OperationsRequest, PortfolioPosition, PortfolioRequest,
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
    domain::{
        CouponCalendar, CouponPayment, CouponProfit, DividendCalendar, DividendPayment,
        DividendProfit, Figi, Instrument, LoadedPaper, Money, NoneProfit, Paper, Portfolio,
        Position, Profit, Ticker, Totals,
        calendar::{CalendarPayment, CombinedCalendar, CombinedPayment},
        fx::{FxCandidate, FxInstrument, build_fx_map, quote_to_rub_rate},
        history::{History, HistoryItem},
    },
    progress::Progress,
    to_currency, to_datetime_utc, to_decimal, to_money,
};

/// Maximum number of concurrent API requests when loading portfolio positions or calendars.
pub const MAX_CONCURRENT_REQUESTS: usize = 10;

/// FX instrument map + per-request rate cache (spot and historical).
pub struct FxBook {
    instruments: HashMap<Currency, FxInstrument>,
    spot: Mutex<HashMap<Currency, Decimal>>,
    hist: Mutex<HashMap<(Currency, NaiveDate), Decimal>>,
}

/// Builder for calendar queries with fluent API.
pub struct CalendarBuilder<'a> {
    client: &'a TinkoffInvestment,
    include_dividends: bool,
    include_coupons: bool,
}

impl<'a> CalendarBuilder<'a> {
    fn new(client: &'a TinkoffInvestment) -> Self {
        Self {
            client,
            include_dividends: false,
            include_coupons: false,
        }
    }

    /// Include dividend payments in the calendar.
    #[must_use]
    pub fn dividends(mut self) -> Self {
        self.include_dividends = true;
        self
    }

    /// Include coupon payments in the calendar.
    #[must_use]
    pub fn coupons(mut self) -> Self {
        self.include_coupons = true;
        self
    }

    /// Fetches the calendar based on the builder configuration.
    ///
    /// Positions whose payments failed to load are not included into the calendar;
    /// their errors are returned alongside so the caller can report an incomplete result.
    ///
    /// # Errors
    ///
    /// Returns an error if FX rates cannot be loaded or converted.
    pub async fn fetch(
        self,
        portfolio: &AccountPortfolio,
        instruments: Arc<HashMap<String, Instrument>>,
    ) -> color_eyre::Result<(CombinedCalendar, Vec<eyre::Report>)> {
        let now = Some(chrono::Utc::now());

        let fx = self.client.load_fx_book().await?;
        let mut payments = Vec::new();
        let mut failures = Vec::new();

        if self.include_dividends {
            let (dividend_calendar, errors) = self
                .client
                .get_dividend_calendar(portfolio, instruments.clone(), now, fx.clone())
                .await?;
            payments.extend(
                dividend_calendar
                    .upcoming
                    .into_iter()
                    .map(CombinedPayment::Dividend),
            );
            failures.extend(errors);
        }

        if self.include_coupons {
            let (coupon_calendar, errors) = self
                .client
                .get_coupon_calendar(portfolio, instruments, now, fx)
                .await?;
            payments.extend(
                coupon_calendar
                    .upcoming
                    .into_iter()
                    .map(CombinedPayment::Coupon),
            );
            failures.extend(errors);
        }

        payments.sort_by_key(CalendarPayment::payment_date);
        Ok((CombinedCalendar { upcoming: payments }, failures))
    }
}

#[derive(Default)]
pub struct AccountPortfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

#[derive(Clone)]
pub struct TinkoffInvestment {
    service: Arc<TinkoffInvestService>,
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
        tinkoff_invest_api::tcs::OperationType::DividendTax
        | tinkoff_invest_api::tcs::OperationType::DividendTaxProgressive
        | tinkoff_invest_api::tcs::OperationType::BondTax
        | tinkoff_invest_api::tcs::OperationType::BondTaxProgressive
        | tinkoff_invest_api::tcs::OperationType::Coupon
        | tinkoff_invest_api::tcs::OperationType::BenefitTax
        | tinkoff_invest_api::tcs::OperationType::BenefitTaxProgressive
        | tinkoff_invest_api::tcs::OperationType::Overnight
        | tinkoff_invest_api::tcs::OperationType::Tax
        | tinkoff_invest_api::tcs::OperationType::Dividend => OperationInfluence::PureIncome,
        tinkoff_invest_api::tcs::OperationType::ServiceFee
        | tinkoff_invest_api::tcs::OperationType::MarginFee
        | tinkoff_invest_api::tcs::OperationType::BrokerFee
        | tinkoff_invest_api::tcs::OperationType::SuccessFee
        | tinkoff_invest_api::tcs::OperationType::TrackMfee
        | tinkoff_invest_api::tcs::OperationType::TrackPfee
        | tinkoff_invest_api::tcs::OperationType::CashFee
        | tinkoff_invest_api::tcs::OperationType::OutFee
        | tinkoff_invest_api::tcs::OperationType::OutStampDuty
        | tinkoff_invest_api::tcs::OperationType::AdviceFee
        | tinkoff_invest_api::tcs::OperationType::OutputPenalty => OperationInfluence::Fees,
        _ => OperationInfluence::Unspecified,
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

        Ok(Self {
            currency,
            average_buy_price,
            current_instrument_price,
            accrued_interest,
            quantity,
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
/// Errors without a gRPC status (e.g. transport failures) are considered transient,
/// while statuses like `Unauthenticated` or `InvalidArgument` are not.
fn is_transient(e: &eyre::Report) -> bool {
    grpc_status(e).is_none_or(|s| {
        matches!(
            s.code(),
            Code::Unavailable
                | Code::ResourceExhausted
                | Code::DeadlineExceeded
                | Code::Aborted
                | Code::Internal
                | Code::Unknown
        )
    })
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
            service: Arc::new(TinkoffInvestService::new(token)),
            channel: Arc::new(OnceCell::new()),
        }
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
        account: AccountType,
    ) -> color_eyre::Result<(AccountPortfolio, HashMap<String, Instrument>)> {
        let portfolio = self.get_portfolio_until_done(account).await?;
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
            let instrument =
                with_retry(|| client.get_instrument_by_figi(position.figi.clone())).await;
            (position.figi, instrument)
        })
        .await
        .into_iter()
        .filter_map(|(figi, instrument)| Some((figi, instrument.ok()?)))
        .collect()
    }

    async fn get_instrument_by_figi(&self, figi: String) -> color_eyre::Result<Instrument> {
        let channel = self.create_channel().await?;
        let mut instruments = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("Failed to get instruments service: {e:?}"))?;
        let response = instruments
            .get_instrument_by(InstrumentRequest {
                id_type: InstrumentIdType::Figi as i32,
                class_code: None,
                id: figi.clone(),
            })
            .await
            .wrap_err_with(|| format!("Failed to get instrument {figi}"))?;
        let instrument = response
            .into_inner()
            .instrument
            .ok_or_else(|| eyre::eyre!("Instrument {figi} not found"))?;
        Ok(Instrument {
            name: instrument.name,
            ticker: Ticker::new(instrument.ticker),
        })
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
                    spot: Mutex::new(HashMap::new()),
                    hist: Mutex::new(HashMap::new()),
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
                            .paper_for_position(&instruments, &account_id, &position, &fx)
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

    async fn paper_for_position(
        &self,
        instruments: &HashMap<String, Instrument>,
        account_id: &str,
        position: &PortfolioPosition,
        fx: &FxBook,
    ) -> color_eyre::Result<LoadedPaper> {
        let account_id = account_id.to_string();
        let paper = match position.instrument_type.as_str() {
            "bond" => self
                .create_paper_from_position(instruments, account_id, position, CouponProfit, fx)
                .await
                .map(LoadedPaper::Bond),
            "share" => self
                .create_paper_from_position(instruments, account_id, position, DividendProfit, fx)
                .await
                .map(LoadedPaper::Share),
            "etf" => self
                .create_paper_from_position(instruments, account_id, position, DividendProfit, fx)
                .await
                .map(LoadedPaper::Etf),
            "currency" => self
                .create_paper_from_position(instruments, account_id, position, NoneProfit, fx)
                .await
                .map(LoadedPaper::Currency),
            "futures" => self
                .create_paper_from_position(instruments, account_id, position, NoneProfit, fx)
                .await
                .map(LoadedPaper::Future),
            other => Err(eyre::eyre!("Unsupported instrument type '{other}'")),
        };
        paper.map_err(|e| {
            skipped(
                e,
                &instrument_or_figi(instruments, &position.figi),
                &position.figi,
            )
        })
    }

    async fn get_portfolio(&self, account: AccountType) -> color_eyre::Result<AccountPortfolio> {
        let (accounts_res, ops_res) = tokio::join!(self.get_accounts_response(), async {
            let ch = self.create_channel().await?;
            self.service
                .operations(ch)
                .await
                .map_err(|e| eyre::eyre!("Failed to get operations service: {e:?}"))
        },);

        let accounts_res = accounts_res?;
        let mut operations = ops_res?;

        let Some(account) = accounts_res.accounts.iter().find(|a| a.r#type() == account) else {
            return Ok(AccountPortfolio::default());
        };

        // Native instrument currencies; convert to RUB in domain via FX.
        let portfolio = operations
            .get_portfolio(PortfolioRequest {
                account_id: account.id.clone(),
                currency: None,
            })
            .await
            .wrap_err("Failed to get portfolio")?;
        Ok(AccountPortfolio {
            account_id: account.id.clone(),
            positions: portfolio.into_inner().positions,
        })
    }

    /// Get accounts response from API.
    ///
    /// # Errors
    ///
    /// This function will return an error if accounts cannot be retrieved.
    async fn get_accounts_response(&self) -> color_eyre::Result<GetAccountsResponse> {
        let channel = self.create_channel().await?;
        let mut users = self
            .service
            .users(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let accounts = users
            .get_accounts(GetAccountsRequest {})
            .await
            .wrap_err("Failed to get accounts")?;
        Ok(accounts.into_inner())
    }

    /// Get an account by type.
    ///
    /// # Errors
    ///
    /// This function will return an error if account cannot be get.
    pub async fn get_account(&self, account_type: AccountType) -> color_eyre::Result<Account> {
        let accounts = &self.get_accounts_response().await?;
        let account = accounts
            .accounts
            .iter()
            .find(|a| a.r#type() == account_type)
            .or_else(|| accounts.accounts.first())
            .ok_or_else(|| eyre::eyre!("No accounts found"))?;
        Ok(account.clone())
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
        let channel = self.create_channel().await?;
        let mut instruments = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let instrument = instruments
            .find_instrument(FindInstrumentRequest {
                instrument_kind: Some(InstrumentType::Unspecified.into()),
                query: ticker,
                api_trade_available_flag: Some(false),
            })
            .await?;
        Ok(instrument.get_ref().instruments.clone())
    }

    /// Get portfolio until done with retry logic.
    ///
    /// # Errors
    ///
    /// This function will return an error if portfolio cannot be retrieved after multiple retries.
    pub async fn get_portfolio_until_done(
        &self,
        account: AccountType,
    ) -> color_eyre::Result<AccountPortfolio> {
        with_retry(|| self.get_portfolio(account)).await
    }

    async fn get_operations(
        &self,
        account_id: String,
        figi: String,
    ) -> color_eyre::Result<Vec<Operation>> {
        let channel = self.create_channel().await?;
        let mut operations = self
            .service
            .operations(channel)
            .await
            .map_err(|e| eyre::eyre!("Failed to get operations service: {e:?}"))?;
        let operations = operations
            .get_operations(OperationsRequest {
                account_id,
                from: None,
                to: None,
                state: Some(OperationState::Executed as i32),
                figi: Some(figi),
            })
            .await
            .wrap_err("Failed to get operations")?;

        Ok(operations.into_inner().operations)
    }

    /// Get operations until done with retry logic.
    ///
    /// # Errors
    ///
    /// This function will return an error if operations cannot be retrieved after multiple retries.
    pub async fn get_operations_until_done(
        &self,
        account_id: String,
        figi: String,
    ) -> color_eyre::Result<Vec<Operation>> {
        with_retry(|| self.get_operations(account_id.clone(), figi.clone())).await
    }

    /// Creates a paper from a portfolio position with prices and operation totals in RUB.
    ///
    /// # Errors
    ///
    /// Returns an error if position prices are missing, FX conversion fails,
    /// or operations cannot be loaded.
    pub async fn create_paper_from_position<P: Profit>(
        &self,
        instruments: &HashMap<String, Instrument>,
        account_id: String,
        portfolio_position: &PortfolioPosition,
        profit: P,
        fx: &FxBook,
    ) -> color_eyre::Result<Paper<P>> {
        let mut position = Position::try_from(portfolio_position)?;

        // Convert position prices to RUB at spot; keep nominal on position.currency.
        position.average_buy_price = self
            .money_to_rub(fx, position.average_buy_price, None)
            .await?;
        position.current_instrument_price = self
            .money_to_rub(fx, position.current_instrument_price, None)
            .await?;
        position.accrued_interest = if position.accrued_interest.value.is_zero() {
            Money::zero(Currency::RUB)
        } else {
            self.money_to_rub(fx, position.accrued_interest, None)
                .await?
        };

        let executed_ops = self
            .get_operations_until_done(account_id, portfolio_position.figi.clone())
            .await?;

        let totals = self.reduce(fx, &executed_ops).await?;

        let instrument = instrument_or_figi(instruments, &portfolio_position.figi);
        Ok(Paper {
            name: instrument.name,
            ticker: instrument.ticker,
            figi: Figi::new(portfolio_position.figi.clone()),
            position,
            totals,
            profit,
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
        }
    }

    async fn reduce(&self, fx: &FxBook, operations: &[Operation]) -> color_eyre::Result<Totals> {
        let mut income = Vec::new();
        let mut fees = Vec::new();
        for op in operations {
            let Some(payment) = crate::to_money(op.payment.as_ref()) else {
                continue;
            };
            let at = op.date.as_ref().map(|d| to_datetime_utc(Some(d)));
            let payment = self.money_to_rub(fx, payment, at).await?;
            match to_influence(op.operation_type()) {
                OperationInfluence::PureIncome => income.push(payment),
                OperationInfluence::Fees => fees.push(payment),
                OperationInfluence::Unspecified => {}
            }
        }
        Ok(Self::accumulate_totals_rub(income, fees))
    }

    /// Creates a new calendar builder for fluent API.
    #[must_use]
    pub fn calendar(&self) -> CalendarBuilder<'_> {
        CalendarBuilder::new(self)
    }

    /// Builds instrument history with all money fields converted to RUB.
    ///
    /// # Errors
    ///
    /// Returns an error if FX conversion fails.
    pub async fn history_in_rub(
        &self,
        operations: &[Operation],
        instrument: &InstrumentShort,
    ) -> color_eyre::Result<Option<History>> {
        let fx = self.load_fx_book().await?;
        let mut items: Vec<HistoryItem> = Vec::new();
        for op in operations.iter().unique_by(|op| &op.id) {
            let mut item = HistoryItem::from(op);
            let at = Some(item.datetime);
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
        let channel = self.create_channel().await?;
        let mut instruments = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("Failed to get instruments service: {e:?}"))?;
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
            spot: Mutex::new(HashMap::new()),
            hist: Mutex::new(HashMap::new()),
        }))
    }

    async fn money_to_rub(
        &self,
        fx: &FxBook,
        money: Money,
        at: Option<DateTime<Utc>>,
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
        at: Option<DateTime<Utc>>,
    ) -> color_eyre::Result<Decimal> {
        if currency == Currency::RUB {
            return Ok(Decimal::ONE);
        }
        let instr = fx
            .instruments
            .get(&currency)
            .ok_or_else(|| eyre::eyre!("No FX instrument for currency {currency:?}"))?;
        match at {
            None => self.spot_rate(fx, currency, instr).await,
            Some(dt) => self.hist_rate(fx, currency, instr, dt).await,
        }
    }

    async fn spot_rate(
        &self,
        fx: &FxBook,
        currency: Currency,
        instr: &FxInstrument,
    ) -> color_eyre::Result<Decimal> {
        if let Ok(guard) = fx.spot.lock()
            && let Some(rate) = guard.get(&currency).copied()
        {
            return Ok(rate);
        }

        let channel = self.create_channel().await?;
        let mut market = self
            .service
            .marketdata(channel)
            .await
            .map_err(|e| eyre::eyre!("Failed to get marketdata service: {e:?}"))?;
        let response = market
            .get_last_prices(GetLastPricesRequest {
                instrument_id: vec![instr.instrument_id.clone()],
                ..Default::default()
            })
            .await
            .wrap_err_with(|| format!("GetLastPrices failed for {currency:?}"))?;
        let price = response
            .into_inner()
            .last_prices
            .into_iter()
            .next()
            .and_then(|lp| lp.price)
            .ok_or_else(|| eyre::eyre!("Empty last price for {currency:?}"))?;
        let rate = quote_to_rub_rate(to_decimal(Some(&price)), instr.lot, instr.nominal);
        if let Ok(mut guard) = fx.spot.lock() {
            guard.insert(currency, rate);
        }
        Ok(rate)
    }

    async fn hist_rate(
        &self,
        fx: &FxBook,
        currency: Currency,
        instr: &FxInstrument,
        at: DateTime<Utc>,
    ) -> color_eyre::Result<Decimal> {
        let date = at.date_naive();
        for offset in 0i64..3 {
            let day = date
                .checked_sub_signed(ChronoDuration::days(offset))
                .unwrap_or(date);
            if let Ok(guard) = fx.hist.lock()
                && let Some(rate) = guard.get(&(currency, day)).copied()
            {
                return Ok(rate);
            }

            let from = day
                .and_hms_opt(0, 0, 0)
                .map(|naive| naive.and_utc())
                .ok_or_else(|| eyre::eyre!("Invalid date {day}"))?;
            let to = day
                .succ_opt()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|naive| naive.and_utc())
                .ok_or_else(|| eyre::eyre!("Invalid date {day}"))?;

            let channel = self.create_channel().await?;
            let mut market = self
                .service
                .marketdata(channel)
                .await
                .map_err(|e| eyre::eyre!("Failed to get marketdata service: {e:?}"))?;
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
                .wrap_err_with(|| format!("GetCandles failed for {currency:?}"))?;

            let candles = response.into_inner().candles;
            if let Some(close) = candles.last().and_then(|c| c.close.as_ref()) {
                let rate = quote_to_rub_rate(to_decimal(Some(close)), instr.lot, instr.nominal);
                if let Ok(mut guard) = fx.hist.lock() {
                    guard.insert((currency, day), rate);
                }
                return Ok(rate);
            }
        }
        Err(eyre::eyre!("No FX candle for {currency:?} around {date}"))
    }

    /// Internal method for fetching dividend calendar with optional date filtering.
    async fn get_dividend_calendar(
        &self,
        portfolio: &AccountPortfolio,
        instruments: Arc<HashMap<String, Instrument>>,
        filter_after: Option<DateTime<Utc>>,
        fx: Arc<FxBook>,
    ) -> color_eyre::Result<(DividendCalendar, Vec<eyre::Report>)> {
        // The API rejects dividend requests for anything but shares and ETFs.
        let dividend_positions: Vec<PortfolioPosition> = portfolio
            .positions
            .iter()
            .filter(|p| p.instrument_type == "share" || p.instrument_type == "etf")
            .cloned()
            .collect();

        let pairs = self
            .fetch_parallel(&dividend_positions, |client, figi| async move {
                client.get_dividends_for_figi(figi).await
            })
            .await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (position, dividends) in pairs {
            let instrument = instrument_or_figi(&instruments, &position.figi);
            let dividends = match dividends {
                Ok(dividends) => dividends,
                Err(e) => {
                    failures.push(skipped(e, &instrument, &position.figi));
                    continue;
                }
            };
            for dividend in dividends {
                let dividend_per_share = dividend
                    .dividend_net
                    .as_ref()
                    .and_then(|d| to_money(Some(d)))
                    .unwrap_or_else(|| Money::zero(Currency::RUB));

                let payment_date = dividend
                    .payment_date
                    .as_ref()
                    .map_or_else(chrono::Utc::now, |d| to_datetime_utc(Some(d)));

                if let Some(cutoff) = filter_after
                    && payment_date < cutoff
                {
                    continue;
                }

                let ex_dividend_date = dividend
                    .record_date
                    .as_ref()
                    .map_or_else(chrono::Utc::now, |d| to_datetime_utc(Some(d)));

                // Upcoming: spot; otherwise rate on payment date.
                let at = if filter_after.is_some() {
                    None
                } else {
                    Some(payment_date)
                };
                let dividend_per_share = self.money_to_rub(&fx, dividend_per_share, at).await?;

                let quantity = to_decimal(position.quantity.as_ref());
                upcoming.push(DividendPayment {
                    figi: Figi::new(position.figi.clone()),
                    ticker: Ticker::new(instrument.ticker.as_str().to_string()),
                    name: instrument.name.clone(),
                    currency: Currency::RUB,
                    dividend_per_share,
                    total_dividend: dividend_per_share * quantity,
                    quantity,
                    ex_dividend_date,
                    payment_date: Some(payment_date),
                    dividend_type: dividend.dividend_type,
                });
            }
        }

        upcoming.sort_by_key(|a| a.ex_dividend_date);
        Ok((DividendCalendar { upcoming }, failures))
    }

    async fn get_dividends_for_figi(&self, figi: String) -> color_eyre::Result<Vec<Dividend>> {
        let channel = self.create_channel().await?;
        let mut instruments = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
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
        instruments: Arc<HashMap<String, Instrument>>,
        filter_after: Option<DateTime<Utc>>,
        fx: Arc<FxBook>,
    ) -> color_eyre::Result<(CouponCalendar, Vec<eyre::Report>)> {
        let bond_positions: Vec<PortfolioPosition> = portfolio
            .positions
            .iter()
            .filter(|p| p.instrument_type == "bond")
            .cloned()
            .collect();

        let pairs = self
            .fetch_parallel(&bond_positions, |client, figi| async move {
                client.get_coupons_for_figi(figi).await
            })
            .await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (position, coupons) in pairs {
            let instrument = instrument_or_figi(&instruments, &position.figi);
            let coupons = match coupons {
                Ok(coupons) => coupons,
                Err(e) => {
                    failures.push(skipped(e, &instrument, &position.figi));
                    continue;
                }
            };
            for coupon in coupons {
                let coupon_value = coupon
                    .pay_one_bond
                    .as_ref()
                    .and_then(|d| to_money(Some(d)))
                    .unwrap_or_else(|| Money::zero(Currency::RUB));

                let coupon_date = coupon
                    .coupon_date
                    .as_ref()
                    .map_or_else(chrono::Utc::now, |d| to_datetime_utc(Some(d)));

                if let Some(cutoff) = filter_after
                    && coupon_date <= cutoff
                {
                    continue;
                }

                let at = if filter_after.is_some() {
                    None
                } else {
                    Some(coupon_date)
                };
                let coupon_value = self.money_to_rub(&fx, coupon_value, at).await?;

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
                    coupon_type: coupon_type_to_str(coupon.coupon_type()).to_string(),
                });
            }
        }

        upcoming.sort_by_key(|a| a.coupon_date);
        Ok((CouponCalendar { upcoming }, failures))
    }

    async fn get_coupons_for_figi(&self, figi: String) -> color_eyre::Result<Vec<Coupon>> {
        let channel = self.create_channel().await?;
        let mut instruments = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let response = instruments
            .get_bond_coupons(GetBondCouponsRequest {
                instrument_id: figi,
                from: None,
                to: None,
                ..Default::default()
            })
            .await
            .wrap_err("Failed to get bond coupons")?;
        Ok(response.into_inner().events)
    }
}

/// Adds to `e` which position was skipped because of it.
fn skipped(e: eyre::Report, instrument: &Instrument, figi: &str) -> eyre::Report {
    e.wrap_err(format!("Position {} ({figi}) skipped", instrument.ticker))
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
        })
}

fn money_value_amount(mv: &tinkoff_invest_api::tcs::MoneyValue) -> Decimal {
    Decimal::from(mv.units) + Decimal::from(mv.nano) / dec!(1_000_000_000)
}

#[must_use]
fn coupon_type_to_str(coupon_type: tinkoff_invest_api::tcs::CouponType) -> &'static str {
    match coupon_type {
        tinkoff_invest_api::tcs::CouponType::Unspecified => "Unspecified",
        tinkoff_invest_api::tcs::CouponType::Constant => "Constant",
        tinkoff_invest_api::tcs::CouponType::Floating => "Floating",
        tinkoff_invest_api::tcs::CouponType::Discount => "Discount",
        tinkoff_invest_api::tcs::CouponType::Mortgage => "Mortgage",
        tinkoff_invest_api::tcs::CouponType::Fix => "Fix",
        tinkoff_invest_api::tcs::CouponType::Variable => "Variable",
        tinkoff_invest_api::tcs::CouponType::Other => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tinkoff_invest_api::tcs::{MoneyValue, Quotation};

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
    fn is_transient_without_status() {
        // Arrange
        let error = eyre::eyre!("connection reset");

        // Act
        let transient = is_transient(&error);

        // Assert
        assert!(transient);
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
            spot: Mutex::new(HashMap::new()),
            hist: Mutex::new(HashMap::new()),
        };
        let instruments = HashMap::from([(
            "OPT1".to_string(),
            Instrument {
                name: "Option".to_string(),
                ticker: Ticker::new("OPTX"),
            },
        )]);
        let position = PortfolioPosition {
            figi: "OPT1".to_string(),
            instrument_type: "option".to_string(),
            ..Default::default()
        };

        // Act
        let result = client
            .paper_for_position(&instruments, "account", &position, &fx)
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
