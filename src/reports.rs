use chrono::{DateTime, Datelike, Duration as ChronoDuration, Utc};
use color_eyre::eyre::{self, WrapErr};
use iso_currency::Currency;
use itertools::Itertools;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::{
    api::{InvestApi, OperationsFilter},
    domain::{
        AccountPosition, BondInfo, CouponCalendar, CouponPayment, CouponProfit, DividendCalendar,
        DividendPayment, DividendProfit, Figi, FoundInstrument, Instrument, InstrumentKind,
        LoadedPaper, Money, NoneProfit, Paper, Portfolio, Position, Ticker, Totals,
        account::{Account, AccountStatus, AccountType},
        analytics::{Analytics, ShareAnalytics},
        benchmark::{BENCHMARKS, IndexHistory, IndexPrices},
        bond::{BondEventKind, bond_info},
        calendar::{Calendar, CalendarKind, CombinedCalendar, Dividend},
        fx::{DailyCloses, RubConverter},
        history::{History, HistoryItem, HistoryPrice},
        income::{DividendRecord, IncomeForecast, IncomeItem, IncomeKind, expected_dividends},
        lots::{LotBook, PositionSale, SaleTaxReport, Trade, TradeKind, YearIncome},
        moscow_date,
        operation::{Operation, OperationKind},
        tax::{TaxOperation, TaxReport},
        xirr::CashFlow,
    },
    progress::Progress,
};

/// Passive income forecast horizon in days.
const INCOME_FORECAST_DAYS: u32 = 365;

/// Maximum number of concurrent API requests when loading portfolio positions or calendars.
pub const MAX_CONCURRENT_REQUESTS: usize = 10;

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
            .filter(|a| a.account_type == *account_type && a.status == AccountStatus::Open)
            .collect(),
    };
    let wanted = match selector {
        AccountSelector::Id(id) => format!("account with ID {id}"),
        AccountSelector::Type(t) => format!("open {} account", t.name()),
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
        .map(|a| format!("{} ({}, {})", a.id, a.name, a.account_type.name()))
        .join(", ");
    if described.is_empty() {
        "none".to_string()
    } else {
        described
    }
}

/// Portfolio position of a supported kind with valid prices and its instrument.
#[derive(Clone)]
pub struct Holding {
    pub kind: InstrumentKind,
    /// Loaded instrument or, when it failed to load, one named after the FIGI
    pub instrument: Instrument,
    pub figi: String,
    pub instrument_uid: String,
    pub position: Position,
}

impl Holding {
    /// Adds to `e` that this position was skipped because of it.
    fn skipped(&self, e: eyre::Report) -> eyre::Report {
        skipped(e, &self.instrument, &self.figi)
    }
}

/// Position the reports leave out: of an unsupported type, e.g. an option,
/// or of a supported `kind` without valid prices.
struct SkippedPosition {
    kind: Option<InstrumentKind>,
    instrument: Instrument,
    figi: String,
    reason: String,
}

/// Account portfolio with instruments of its positions.
pub struct AccountPortfolio {
    pub account_id: String,
    pub holdings: Vec<Holding>,
    skipped_positions: Vec<SkippedPosition>,
}

impl AccountPortfolio {
    /// Pairs positions with their instruments, falling back to FIGI for the ones not loaded;
    /// positions of unsupported types or without valid prices are set aside.
    fn new(
        account_id: String,
        positions: Vec<AccountPosition>,
        instruments: &HashMap<String, Instrument>,
    ) -> Self {
        let mut holdings = Vec::new();
        let mut skipped_positions = Vec::new();
        for p in positions {
            let instrument = instrument_or_figi(instruments, &p.figi);
            let kind = InstrumentKind::from_api(&p.instrument_type);
            match (kind, p.position) {
                (Some(kind), Ok(position)) => holdings.push(Holding {
                    kind,
                    instrument,
                    figi: p.figi,
                    instrument_uid: p.instrument_uid,
                    position,
                }),
                (Some(_), Err(reason)) => skipped_positions.push(SkippedPosition {
                    kind,
                    instrument,
                    figi: p.figi,
                    reason,
                }),
                (None, _) => skipped_positions.push(SkippedPosition {
                    kind,
                    instrument,
                    figi: p.figi,
                    reason: format!("Unsupported instrument type '{}'", p.instrument_type),
                }),
            }
        }
        Self {
            account_id,
            holdings,
            skipped_positions,
        }
    }

    /// Errors telling which positions the reports leave out and why.
    #[must_use]
    pub fn skipped(&self) -> Vec<eyre::Report> {
        self.skipped_where(|_| true)
    }

    /// Errors about the positions of the kinds a report `covers` that it leaves out.
    fn skipped_of(&self, covers: impl Fn(InstrumentKind) -> bool) -> Vec<eyre::Report> {
        self.skipped_where(|kind| kind.is_some_and(&covers))
    }

    fn skipped_where(&self, include: impl Fn(Option<InstrumentKind>) -> bool) -> Vec<eyre::Report> {
        self.skipped_positions
            .iter()
            .filter(|p| include(p.kind))
            .map(|p| skipped(eyre::eyre!("{}", p.reason), &p.instrument, &p.figi))
            .collect()
    }
}

/// Portfolio reports built from the data of an [`InvestApi`].
#[derive(Clone)]
pub struct Reports<A> {
    api: A,
}

impl<A: InvestApi> Reports<A> {
    #[must_use]
    pub fn new(api: A) -> Self {
        Self { api }
    }

    /// Lists all accounts of the user.
    ///
    /// # Errors
    ///
    /// Returns an error if accounts cannot be retrieved.
    pub async fn get_accounts(&self) -> color_eyre::Result<Vec<Account>> {
        self.api.accounts().await
    }

    /// Loads the portfolio of the account with instruments of its positions.
    ///
    /// With `kind`, only positions of this kind are loaded; otherwise all of them.
    /// Positions of unsupported types or without valid prices are kept for
    /// [`AccountPortfolio::skipped`].
    /// Instruments that fail to load are named after the FIGI.
    ///
    /// # Errors
    ///
    /// Returns an error if the account or its portfolio cannot be loaded after retries.
    pub async fn load_portfolio(
        &self,
        selector: &AccountSelector,
        kind: Option<InstrumentKind>,
    ) -> color_eyre::Result<AccountPortfolio> {
        let account = self.get_account(selector).await?;
        self.load_account_portfolio(&account, kind).await
    }

    async fn load_account_portfolio(
        &self,
        account: &Account,
        kind: Option<InstrumentKind>,
    ) -> color_eyre::Result<AccountPortfolio> {
        let mut positions = self.api.portfolio_positions(&account.id).await?;
        if let Some(kind) = kind {
            positions.retain(|p| InstrumentKind::from_api(&p.instrument_type) == Some(kind));
        }
        let instruments = self.get_instruments_for_positions(&positions).await;
        Ok(AccountPortfolio::new(
            account.id.clone(),
            positions,
            &instruments,
        ))
    }

    /// Looks up instruments of the given positions by FIGI concurrently;
    /// instruments that could not be loaded are absent from the result.
    async fn get_instruments_for_positions(
        &self,
        positions: &[AccountPosition],
    ) -> HashMap<String, Instrument> {
        self.parallel_for(
            positions,
            None,
            |reports, position: AccountPosition| async move {
                let kind = InstrumentKind::from_api(&position.instrument_type);
                let instrument = reports.api.instrument(&position.figi, kind).await;
                (position.figi, instrument)
            },
        )
        .await
        .into_iter()
        .filter_map(|(figi, instrument)| Some((figi, instrument.ok()?)))
        .collect()
    }

    /// Loads analyst forecasts and fundamentals of the portfolio holdings, meant for shares.
    ///
    /// Shares without a forecast or fundamentals are kept with `None`; failed requests
    /// are returned alongside so the caller can tell the user that the output is incomplete.
    pub async fn get_share_analytics(
        &self,
        portfolio: &AccountPortfolio,
    ) -> (Analytics, Vec<eyre::Report>) {
        let holdings = &portfolio.holdings;
        let asset_uids = holdings
            .iter()
            .filter_map(|h| h.instrument.asset_uid.clone())
            .unique()
            .collect_vec();
        let (forecasts, fundamentals) = tokio::join!(
            self.parallel_for(holdings, None, |reports, holding: Holding| async move {
                let forecast = reports
                    .api
                    .forecast(&holding.instrument_uid, holding.instrument.currency)
                    .await;
                (holding, forecast)
            }),
            self.api.fundamentals(&asset_uids),
        );

        let mut failures = portfolio.skipped_of(|k| k == InstrumentKind::Share);
        let fundamentals = fundamentals.unwrap_or_else(|e| {
            failures.push(e);
            HashMap::new()
        });
        let mut forecasts_by_figi = HashMap::new();
        for (holding, forecast) in forecasts {
            match forecast {
                Ok(forecast) => {
                    forecasts_by_figi.insert(holding.figi, forecast);
                }
                Err(e) => {
                    let ticker = &holding.instrument.ticker;
                    failures.push(e.wrap_err(format!("Forecast for {ticker} not loaded")));
                }
            }
        }

        let shares = holdings
            .iter()
            .map(|h| ShareAnalytics {
                forecast: forecasts_by_figi.get(&h.figi).cloned().flatten(),
                fundamentals: h
                    .instrument
                    .asset_uid
                    .as_ref()
                    .and_then(|uid| fundamentals.get(uid))
                    .copied(),
                name: h.instrument.name.clone(),
                ticker: h.instrument.ticker.clone(),
            })
            .collect();
        (Analytics::new(shares), failures)
    }

    /// Fetches data for each holding by its FIGI in parallel,
    /// limiting concurrent requests with a semaphore.
    ///
    /// Returns pairs of (holding, fetch result); task panics are logged to stderr.
    async fn fetch_parallel<T, F, Fut>(
        &self,
        holdings: &[Holding],
        fetch: F,
    ) -> Vec<(Holding, color_eyre::Result<Vec<T>>)>
    where
        T: Send + 'static,
        F: Fn(Self, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = color_eyre::Result<Vec<T>>> + Send,
    {
        let fetch = Arc::new(fetch);
        self.parallel_for(holdings, None, {
            move |reports, holding: Holding| {
                let fetch = Arc::clone(&fetch);
                async move {
                    let items = fetch(reports, holding.figi.clone()).await;
                    (holding, items)
                }
            }
        })
        .await
    }

    /// Runs `task` for each item concurrently, limited by [`MAX_CONCURRENT_REQUESTS`].
    ///
    /// Task panics are logged to stderr; failed permit acquisition skips the item.
    /// When `progress` is set, it is incremented once per completed task.
    async fn parallel_for<I, T, F, Fut>(
        &self,
        items: &[I],
        progress: Option<Arc<dyn Progress>>,
        task: F,
    ) -> Vec<T>
    where
        I: Clone + Send + 'static,
        T: Send + 'static,
        F: Fn(Self, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
        let task = Arc::new(task);
        let mut set = JoinSet::new();

        for item in items {
            let reports = self.clone();
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to acquire semaphore: {e}");
                    continue;
                }
            };
            let item = item.clone();
            let task = Arc::clone(&task);
            let progress = progress.clone();

            set.spawn(async move {
                let _permit = permit;
                let result = task(reports, item).await;
                if let Some(p) = &progress {
                    p.progress();
                }
                result
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(result) => results.push(result),
                Err(e) => eprintln!("Task panicked or cancelled: {e}"),
            }
        }
        results
    }

    /// Builds a [`Portfolio`] by loading papers for each holding in parallel.
    ///
    /// Position money fields and operation totals are converted to RUB via FX rates.
    /// Positions that failed to load or were skipped are not included into the portfolio;
    /// their errors are returned alongside so the caller can tell the user that totals are
    /// incomplete. Bond details are loaded only with `output_papers`, since only paper cards
    /// show them.
    pub async fn build_portfolio(
        &self,
        portfolio: &AccountPortfolio,
        output_papers: bool,
        progress: Option<Arc<dyn Progress>>,
    ) -> (Portfolio, Vec<eyre::Report>) {
        let fx = match self.load_fx_rates().await {
            Ok(fx) => fx,
            Err(e) => {
                eprintln!("Failed to load FX rates: {e:?}");
                Arc::new(RubConverter::new(std::iter::empty(), self.api.clone()))
            }
        };

        let papers = self
            .parallel_for(&portfolio.holdings, progress.clone(), {
                let account_id = portfolio.account_id.clone();
                move |reports, holding: Holding| {
                    let account_id = account_id.clone();
                    let fx = fx.clone();
                    async move {
                        reports
                            .paper_for_position(&account_id, &holding, &fx, output_papers)
                            .await
                    }
                }
            })
            .await;

        if let Some(p) = &progress {
            p.finish();
        }

        let mut loaded = Portfolio::new(output_papers);
        let mut failures = portfolio.skipped();
        for paper in papers {
            match paper {
                Ok(paper) => loaded.add_loaded_paper(paper),
                Err(e) => failures.push(e),
            }
        }
        (loaded, failures)
    }

    /// Loads a holding as a paper; bond details (events, YTM) only when `with_bond_details`,
    /// as they are shown in paper cards only and cost a request per bond.
    async fn paper_for_position(
        &self,
        account_id: &str,
        holding: &Holding,
        fx: &RubConverter<A>,
        with_bond_details: bool,
    ) -> color_eyre::Result<LoadedPaper> {
        let mut paper = self
            .create_paper_from_position(account_id, holding, fx)
            .await
            .map_err(|e| holding.skipped(e))?;
        if with_bond_details && holding.kind == InstrumentKind::Bond {
            // Bond details are informational: the position stays in totals without them.
            paper.bond = self
                .load_bond_info(&holding.figi, &paper.position, fx)
                .await
                .ok();
        }
        Ok(match holding.kind {
            InstrumentKind::Bond => LoadedPaper::Bond(paper.with_profit(CouponProfit)),
            InstrumentKind::Share => LoadedPaper::Share(paper.with_profit(DividendProfit)),
            InstrumentKind::Etf => LoadedPaper::Etf(paper.with_profit(DividendProfit)),
            InstrumentKind::Currency => LoadedPaper::Currency(paper),
            InstrumentKind::Future => LoadedPaper::Future(paper),
        })
    }

    /// Gets the account chosen by `selector` (see [`select_account`]).
    async fn get_account(&self, selector: &AccountSelector) -> color_eyre::Result<Account> {
        let accounts = self.api.accounts().await?;
        select_account(&accounts, selector).cloned()
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
            .api
            .operations(&account.id, OperationsFilter::Kinds(&OperationKind::TAXED))
            .await?;
        let fx = self.load_fx_rates().await?;

        let mut items = Vec::new();
        for op in operations.iter().unique_by(|op| &op.id) {
            let (Some(kind), Some(payment)) = (op.kind.tax_kind(), op.payment) else {
                continue;
            };
            let date = op.date;
            let amount = fx.to_rub(payment, date).await?;
            items.push(TaxOperation { date, kind, amount });
        }
        Ok(TaxReport::new(&items))
    }

    /// Tax due if shares, bonds and ETFs of the account were sold at `now`,
    /// with their lots matched by FIFO in RUB at trade dates.
    ///
    /// With `ticker`, only the position of this ticker is priced. Positions the report
    /// leaves out are returned alongside.
    ///
    /// # Errors
    ///
    /// Returns an error if the account, its portfolio, operations or FX rates cannot be
    /// loaded, or there is no security with `ticker` in the portfolio.
    pub async fn get_sale_tax(
        &self,
        selector: &AccountSelector,
        ticker: Option<&str>,
        now: DateTime<Utc>,
    ) -> color_eyre::Result<(SaleTaxReport, Vec<eyre::Report>)> {
        let account = self.get_account(selector).await?;
        let (portfolio, operations, fx) = tokio::join!(
            self.load_account_portfolio(&account, None),
            self.api
                .operations(&account.id, OperationsFilter::Kinds(&SALE_TAX_OPERATIONS)),
            self.load_fx_rates()
        );
        let (portfolio, operations, fx) = (portfolio?, operations?, fx?);

        let year = moscow_date(now).year();
        let mut trades: HashMap<&str, Vec<Trade>> = HashMap::new();
        let mut year_income = YearIncome::zero();
        for op in operations.iter().unique_by(|op| &op.id) {
            if matches!(op.kind, OperationKind::Dividend | OperationKind::Coupon) {
                if let Some(payment) = op.payment.filter(|_| moscow_date(op.date).year() == year) {
                    year_income.payments += fx.to_rub(payment, op.date).await?;
                }
            } else if op.instrument_kind.is_some_and(is_lot_kind)
                && let Some(trade) = lot_trade(&fx, op).await?
            {
                trades.entry(op.figi.as_str()).or_default().push(trade);
            }
        }
        let books: HashMap<&str, LotBook> = trades
            .into_iter()
            .map(|(figi, trades)| (figi, LotBook::new(trades)))
            .collect();
        year_income.securities = books
            .values()
            .flat_map(|b| &b.realized)
            .filter(|r| moscow_date(r.date).year() == year)
            .fold(Money::zero(Currency::RUB), |acc, r| acc + r.result);

        let holdings = portfolio
            .holdings
            .iter()
            .filter(|h| is_lot_kind(h.kind))
            .filter(|h| ticker.is_none_or(|t| h.instrument.ticker.as_str().eq_ignore_ascii_case(t)))
            .collect_vec();
        if let (Some(ticker), []) = (ticker, holdings.as_slice()) {
            return Err(eyre::eyre!(
                "No share, bond or ETF {ticker} in the portfolio"
            ));
        }
        let no_trades = LotBook::new([]);
        let mut positions = holdings
            .into_iter()
            .map(|h| {
                let price = h.position.current_instrument_price + h.position.accrued_interest;
                PositionSale::new(
                    h.instrument.name.clone(),
                    h.instrument.ticker.as_str().to_string(),
                    h.position.quantity,
                    books.get(h.figi.as_str()).unwrap_or(&no_trades),
                    price,
                    moscow_date(now),
                )
            })
            .collect_vec();
        positions.sort_by(|a, b| a.ticker.cmp(&b.ticker));

        let report = SaleTaxReport {
            positions,
            year_income,
            taxed: account.account_type != AccountType::TinkoffIis,
        };
        let failures = if ticker.is_some() {
            Vec::new()
        } else {
            portfolio.skipped_of(is_lot_kind)
        };
        Ok((report, failures))
    }

    /// Creates a paper from a portfolio position with prices and operation totals in RUB.
    ///
    /// The paper has no additional profit kind; tag it with [`Paper::with_profit`].
    async fn create_paper_from_position(
        &self,
        account_id: &str,
        holding: &Holding,
        fx: &RubConverter<A>,
    ) -> color_eyre::Result<Paper<NoneProfit>> {
        // Prices are in RUB; the position keeps the currency it is exposed to.
        let mut position = holding.position;

        let instrument = holding.instrument.clone();
        if let Some(currency) = instrument.currency {
            position.currency = currency;
        }

        let executed_ops = self
            .api
            .operations(account_id, OperationsFilter::Instrument(&holding.figi))
            .await?;

        let totals = reduce(fx, &executed_ops).await?;

        Ok(Paper {
            name: instrument.name,
            ticker: instrument.ticker,
            figi: Figi::new(holding.figi.clone()),
            position,
            totals,
            profit: NoneProfit,
            bond: None,
            sector: instrument.sector,
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
        kind: CalendarKind,
        days: u32,
    ) -> color_eyre::Result<(Calendar, Vec<eyre::Report>)> {
        let period = CalendarPeriod::days_ahead(Utc::now(), days);
        let fx = self.load_fx_rates().await?;
        let (calendar, mut failures) = match kind {
            CalendarKind::Dividends => {
                let (calendar, failures) =
                    self.get_dividend_calendar(portfolio, &period, &fx).await?;
                (Calendar::Dividends(calendar), failures)
            }
            CalendarKind::Coupons => {
                let (calendar, failures) =
                    self.get_coupon_calendar(portfolio, &period, &fx).await?;
                (Calendar::Coupons(calendar), failures)
            }
            CalendarKind::Combined => {
                let (dividends, mut failures) =
                    self.get_dividend_calendar(portfolio, &period, &fx).await?;
                let (coupons, coupon_failures) =
                    self.get_coupon_calendar(portfolio, &period, &fx).await?;
                failures.extend(coupon_failures);
                (
                    Calendar::Combined(CombinedCalendar::merge(dividends, coupons)),
                    failures,
                )
            }
        };
        failures.extend(portfolio.skipped_of(|k| match kind {
            CalendarKind::Dividends => k.pays_dividends(),
            CalendarKind::Coupons => k == InstrumentKind::Bond,
            CalendarKind::Combined => k.pays_dividends() || k == InstrumentKind::Bond,
        }));
        Ok((calendar, failures))
    }

    /// History of operations with the instrument of `ticker` on the account, in RUB.
    ///
    /// When the ticker matches several instruments with operations, the one of
    /// [`preferred_figi`] is taken. Instruments whose operations failed to load are
    /// reported to stderr and left out.
    ///
    /// # Errors
    ///
    /// Returns an error if the account or instruments cannot be loaded, FX conversion
    /// fails, or there are no operations with the ticker on the account.
    pub async fn get_history(
        &self,
        selector: &AccountSelector,
        ticker: &str,
    ) -> color_eyre::Result<History> {
        let (account, instruments) = tokio::join!(
            self.get_account(selector),
            self.api.find_instruments(ticker)
        );
        let account_id = account?.id;
        let matching = instruments?
            .into_iter()
            .filter(|i| i.ticker.eq_ignore_ascii_case(ticker))
            .collect_vec();

        let loaded = self
            .parallel_for(
                &matching,
                None,
                move |reports, instrument: FoundInstrument| {
                    let account_id = account_id.clone();
                    async move {
                        let operations = reports
                            .api
                            .operations(&account_id, OperationsFilter::Instrument(&instrument.figi))
                            .await;
                        (instrument, operations)
                    }
                },
            )
            .await;
        let mut with_operations = HashMap::new();
        for (instrument, operations) in loaded {
            match operations {
                Ok(operations) if !operations.is_empty() => {
                    with_operations.insert(instrument.figi.clone(), (instrument, operations));
                }
                Ok(_) => {}
                Err(e) => eprintln!("Failed to load operations: {e:?}"),
            }
        }

        let no_operations = || eyre::eyre!("No operations with {ticker} on the account");
        let (instrument, operations) = preferred_figi(with_operations.keys())
            .and_then(|f| with_operations.get(f))
            .ok_or_else(no_operations)?;
        self.history_in_rub(operations, instrument)
            .await?
            .ok_or_else(no_operations)
    }

    /// Builds instrument history with all money fields converted to RUB;
    /// `None` without operations.
    async fn history_in_rub(
        &self,
        operations: &[Operation],
        instrument: &FoundInstrument,
    ) -> color_eyre::Result<Option<History>> {
        let fx = self.load_fx_rates().await?;
        let mut items: Vec<HistoryItem> = Vec::new();
        for op in operations.iter().unique_by(|op| &op.id) {
            let mut item = HistoryItem::from(op);
            let at = item.datetime;
            item.payment = fx.to_rub(item.payment, at).await?;
            if let HistoryPrice::Money(price) = item.price {
                item.price = HistoryPrice::Money(fx.to_rub(price, at).await?);
            }
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

    /// Loads currency instruments for conversion to RUB at daily rates.
    async fn load_fx_rates(&self) -> color_eyre::Result<Arc<RubConverter<A>>> {
        let candidates = self.api.fx_candidates().await?;
        Ok(Arc::new(RubConverter::new(candidates, self.api.clone())))
    }

    /// Daily closes of [`BENCHMARKS`] from the start of `since_year` until today.
    ///
    /// # Errors
    ///
    /// Returns an error if the list of indices cannot be loaded. An index that is not found
    /// or whose candles fail to load gets an empty history and is reported alongside.
    pub async fn get_index_histories(
        &self,
        since_year: i32,
    ) -> color_eyre::Result<(Vec<IndexHistory>, Vec<eyre::Report>)> {
        let indicatives = self.api.indices().await?;
        let uids = BENCHMARKS.map(|benchmark| {
            indicatives
                .iter()
                .find(|i| i.ticker == benchmark.ticker)
                .map(|i| i.uid.clone())
        });
        let mut loaded = uids.clone().map(|uid| match uid {
            Some(_) => Ok(IndexPrices::new()),
            None => Err(eyre::eyre!("Index not found")),
        });

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
        let mut set = JoinSet::new();
        for (index, uid) in uids.into_iter().enumerate() {
            let Some(uid) = uid else {
                continue;
            };
            for year in since_year..=Utc::now().year() {
                let permit = semaphore
                    .clone()
                    .acquire_owned()
                    .await
                    .wrap_err("Failed to acquire semaphore")?;
                let api = self.api.clone();
                let uid = uid.clone();
                set.spawn(async move {
                    let _permit = permit;
                    (index, api.daily_closes(&uid, year).await)
                });
            }
        }
        while let Some(res) = set.join_next().await {
            match res {
                Ok((index, closes)) => {
                    if let Some(prices) = loaded.get_mut(index) {
                        add_index_closes(prices, closes);
                    }
                }
                Err(e) => eprintln!("Task panicked or cancelled: {e}"),
            }
        }
        Ok(index_histories(loaded))
    }

    /// Internal method for fetching dividend calendar with optional date filtering.
    async fn get_dividend_calendar(
        &self,
        portfolio: &AccountPortfolio,
        period: &CalendarPeriod,
        fx: &RubConverter<A>,
    ) -> color_eyre::Result<(DividendCalendar, Vec<eyre::Report>)> {
        let pairs = self.fetch_portfolio_dividends(portfolio).await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (holding, dividends) in pairs {
            let (instrument, position) = (&holding.instrument, &holding.position);
            let dividends = match dividends {
                Ok(dividends) => dividends,
                Err(e) => {
                    failures.push(holding.skipped(e));
                    continue;
                }
            };
            for dividend in dividends {
                // Without any date a dividend cannot be placed into the calendar.
                let Some(calendar_date) = dividend.date() else {
                    continue;
                };

                if !period.contains(calendar_date) {
                    continue;
                }

                // Future payments are converted at today's rate.
                let dividend_per_share = fx.to_rub(dividend.per_share, period.from).await?;

                let quantity = position.quantity;
                upcoming.push(DividendPayment {
                    figi: Figi::new(holding.figi.clone()),
                    ticker: Ticker::new(instrument.ticker.as_str().to_string()),
                    name: instrument.name.clone(),
                    currency: Currency::RUB,
                    dividend_per_share,
                    total_dividend: dividend_per_share * quantity,
                    quantity,
                    ex_dividend_date: dividend.record_date.unwrap_or(calendar_date),
                    payment_date: dividend.payment_date,
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
    ) -> Vec<(Holding, color_eyre::Result<Vec<Dividend>>)> {
        // The API rejects dividend requests for anything but shares and ETFs.
        let dividend_positions: Vec<Holding> = portfolio
            .holdings
            .iter()
            .filter(|h| h.kind.pays_dividends())
            .cloned()
            .collect();

        self.fetch_parallel(&dividend_positions, |reports, figi| async move {
            reports.api.dividends(&figi).await
        })
        .await
    }

    /// Passive income expected within a year: coupons, declared dividends and dividends
    /// estimated by the last year ones or, of monthly payers, by the latest ones,
    /// with the current portfolio value for its yield.
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
    ) -> color_eyre::Result<(IncomeForecast, Vec<eyre::Report>)> {
        let period = CalendarPeriod::days_ahead(Utc::now(), INCOME_FORECAST_DAYS);
        let fx = self.load_fx_rates().await?;

        let (coupons, mut failures) = self.get_coupon_calendar(portfolio, &period, &fx).await?;
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

        for (holding, dividends) in self.fetch_portfolio_dividends(portfolio).await {
            let dividends = match dividends {
                Ok(dividends) => dividends,
                Err(e) => {
                    failures.push(holding.skipped(e));
                    continue;
                }
            };
            let mut records = Vec::new();
            for dividend in &dividends {
                let Some(date) = dividend.date() else {
                    continue;
                };
                // Past and future payments are all converted at today's rate.
                let per_share = fx.to_rub(dividend.per_share, period.from).await?;
                records.push(DividendRecord { date, per_share });
            }
            let quantity = holding.position.quantity;
            items.extend(expected_dividends(
                &records,
                quantity,
                period.from,
                period.until,
            ));
        }

        // Skipped positions are left out of the portfolio value, as out of the other reports.
        failures.extend(portfolio.skipped());
        let mut portfolio_value = Money::zero(Currency::RUB);
        for holding in &portfolio.holdings {
            portfolio_value += holding.position.current();
        }

        let forecast = IncomeForecast::new(period.from, period.until, &items, portfolio_value);
        Ok((forecast, failures))
    }

    /// Internal method for fetching coupon calendar with optional date filtering.
    async fn get_coupon_calendar(
        &self,
        portfolio: &AccountPortfolio,
        period: &CalendarPeriod,
        fx: &RubConverter<A>,
    ) -> color_eyre::Result<(CouponCalendar, Vec<eyre::Report>)> {
        let bond_positions: Vec<Holding> = portfolio
            .holdings
            .iter()
            .filter(|h| h.kind == InstrumentKind::Bond)
            .cloned()
            .collect();

        let pairs = self
            .fetch_parallel(&bond_positions, |reports, figi| async move {
                reports.api.bond_events(&figi).await
            })
            .await;

        let mut upcoming = Vec::new();
        let mut failures = Vec::new();
        for (holding, events) in pairs {
            let (instrument, position) = (&holding.instrument, &holding.position);
            let events = match events {
                Ok(events) => events,
                Err(e) => {
                    failures.push(holding.skipped(e));
                    continue;
                }
            };
            for event in events.into_iter().filter(|e| e.kind.is_payment()) {
                let coupon_value = event.per_bond;
                let coupon_date = event.date;

                if !period.contains(coupon_date) {
                    continue;
                }

                let coupon_value = fx.to_rub(coupon_value, period.from).await?;

                let quantity = position.quantity;
                upcoming.push(CouponPayment {
                    figi: Figi::new(holding.figi.clone()),
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

    /// Maturity, next offer and YTM of a bond at its current dirty price.
    async fn load_bond_info(
        &self,
        figi: &str,
        position: &Position,
        fx: &RubConverter<A>,
    ) -> color_eyre::Result<BondInfo> {
        let now = Utc::now();
        let mut events = self.api.bond_events(figi).await?;
        for event in &mut events {
            event.per_bond = fx.to_rub(event.per_bond, now).await?;
        }
        let dirty_price = position.current_instrument_price.value + position.accrued_interest.value;
        Ok(bond_info(&events, dirty_price, now))
    }
}

/// Totals of a paper's operations in RUB: income net of taxes, fees and cash flows.
///
/// NKD paid or received on bond trades counts as coupon income.
/// A trade commission is not part of its payment, so it is added to fees and cash flows.
async fn reduce<S: DailyCloses>(
    fx: &RubConverter<S>,
    operations: &[Operation],
) -> color_eyre::Result<Totals> {
    let mut income = Vec::new();
    let mut fees = Vec::new();
    let mut cash_flows = Vec::new();
    for op in operations {
        let at = op.date;
        if let Some(payment) = op.payment {
            if let Some(accrued_interest) = trade_accrued_interest(op) {
                income.push(fx.to_rub(accrued_interest, at).await?);
            }
            let payment = fx.to_rub(payment, at).await?;
            if !payment.value.is_zero() {
                cash_flows.push(CashFlow {
                    date: at,
                    amount: payment.value,
                });
            }
            if op.kind.is_income() {
                income.push(payment);
            } else if op.kind.is_fee() {
                fees.push(payment);
            }
        }
        if let Some(commission) = trade_commission(op) {
            let commission = fx.to_rub(commission, at).await?;
            cash_flows.push(CashFlow {
                date: at,
                amount: commission.value,
            });
            fees.push(commission);
        }
    }
    Ok(Totals {
        cash_flows,
        ..accumulate_totals_rub(income, fees)
    })
}

/// Broker commission of a trade. Fee operations already carry theirs in the payment.
fn trade_commission(op: &Operation) -> Option<Money> {
    if op.kind.is_fee() {
        return None;
    }
    let commission = op.commission?;
    (!commission.value.is_zero()).then_some(commission)
}

/// Operations the sale tax report needs: trades and income of the year.
const SALE_TAX_OPERATIONS: [OperationKind; 5] = [
    OperationKind::Buy,
    OperationKind::Sell,
    OperationKind::Repayment,
    OperationKind::Dividend,
    OperationKind::Coupon,
];

/// Kinds of instruments that are sold by lots and taxed on sale.
fn is_lot_kind(kind: InstrumentKind) -> bool {
    matches!(
        kind,
        InstrumentKind::Share | InstrumentKind::Bond | InstrumentKind::Etf
    )
}

/// Trade in RUB of a buy, sell or repayment operation with a payment;
/// `None` for other operations and trades of no units.
async fn lot_trade<S: DailyCloses>(
    fx: &RubConverter<S>,
    op: &Operation,
) -> color_eyre::Result<Option<Trade>> {
    let Some(payment) = op.payment else {
        return Ok(None);
    };
    let payment = fx.to_rub(payment, op.date).await?;
    let commission = match trade_commission(op) {
        Some(commission) => fx.to_rub(commission, op.date).await?,
        None => Money::zero(Currency::RUB),
    };
    let (kind, amount) = match op.kind {
        OperationKind::Buy => (
            TradeKind::Buy,
            Money::zero(Currency::RUB) - payment - commission,
        ),
        OperationKind::Sell => (TradeKind::Sell, payment + commission),
        OperationKind::Repayment => (TradeKind::Repayment, payment),
        _ => return Ok(None),
    };
    let quantity = Decimal::from(op.quantity - op.quantity_rest);
    if kind != TradeKind::Repayment && quantity <= Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some(Trade {
        date: op.date,
        kind,
        quantity,
        amount,
    }))
}

/// Accrued coupon interest (NKD) paid on a bond buy (negative) or received on a sell (positive).
///
/// A bond trade payment includes NKD besides `price × quantity`, so adding it to coupons
/// gives the net coupon income.
fn trade_accrued_interest(op: &Operation) -> Option<Money> {
    if op.instrument_kind != Some(InstrumentKind::Bond) {
        return None;
    }
    let direction = match op.kind {
        OperationKind::Buy => Decimal::NEGATIVE_ONE,
        OperationKind::Sell => Decimal::ONE,
        _ => return None,
    };
    let accrued = op.accrued_interest?;
    (!accrued.value.is_zero()).then(|| accrued * direction)
}

/// Adds a year of index closes to the prices; any failed year fails the whole index.
fn add_index_closes(
    prices: &mut color_eyre::Result<IndexPrices>,
    closes: color_eyre::Result<IndexPrices>,
) {
    let Ok(loaded) = prices else {
        return;
    };
    match closes {
        Ok(closes) => loaded.extend(closes),
        Err(e) => *prices = Err(e),
    }
}

/// Pairs [`BENCHMARKS`] with their loaded prices; a failed index gets an empty history
/// and its error is returned alongside.
fn index_histories(
    loaded: [color_eyre::Result<IndexPrices>; BENCHMARKS.len()],
) -> (Vec<IndexHistory>, Vec<eyre::Report>) {
    let mut failures = Vec::new();
    let histories = BENCHMARKS
        .into_iter()
        .zip(loaded)
        .map(|(benchmark, prices)| {
            let prices = prices.unwrap_or_else(|e| {
                failures.push(e.wrap_err(format!("Index {} skipped", benchmark.ticker)));
                IndexPrices::new()
            });
            IndexHistory { benchmark, prices }
        })
        .collect();
    (histories, failures)
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
            currency: None,
            asset_uid: None,
            sector: None,
        })
}

/// Accumulates fees and pure-income payments already denominated in RUB.
#[must_use]
fn accumulate_totals_rub(
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

/// FIGI whose history is shown when a ticker matches several instruments:
/// the first one in order, exchange FIGIs before broker's own `TCS` ones.
fn preferred_figi<'a>(figis: impl IntoIterator<Item = &'a String>) -> Option<&'a String> {
    figis.into_iter().min_by_key(|f| (f.starts_with("TCS"), *f))
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use std::future::ready;

    use crate::domain::{
        analytics::{Forecast, Fundamentals},
        benchmark::MarketIndex,
        bond::BondEvent,
        fx::{DailyPrices, FxCandidate},
        operation::OperationState,
    };

    /// In-memory T-Invest API serving fixed data.
    #[derive(Clone, Default)]
    struct FakeApi {
        accounts: Vec<Account>,
        positions: Vec<AccountPosition>,
        instruments: HashMap<String, Instrument>,
        found: Vec<FoundInstrument>,
        /// Operations by FIGI.
        operations: HashMap<String, Vec<Operation>>,
        dividends: HashMap<String, Vec<Dividend>>,
        bond_events: HashMap<String, Vec<BondEvent>>,
        /// Forecasts by instrument UID.
        forecasts: HashMap<String, Forecast>,
        fx: Vec<FxCandidate>,
        /// Close of every day of any year.
        close: Decimal,
        indices: Vec<MarketIndex>,
        /// FIGIs and instrument UIDs whose requests fail.
        failing: Vec<String>,
    }

    impl FakeApi {
        fn get<T: Clone>(
            &self,
            data: &HashMap<String, Vec<T>>,
            id: &str,
        ) -> color_eyre::Result<Vec<T>> {
            if self.failing.iter().any(|f| f == id) {
                return Err(eyre::eyre!("{id} failed"));
            }
            Ok(data.get(id).cloned().unwrap_or_default())
        }
    }

    impl DailyCloses for FakeApi {
        fn daily_closes(
            &self,
            _: &str,
            year: i32,
        ) -> impl Future<Output = color_eyre::Result<DailyPrices>> + Send {
            let closes = (1..=366)
                .filter_map(|day| NaiveDate::from_yo_opt(year, day))
                .map(|date| (date, self.close))
                .collect();
            ready(Ok(closes))
        }
    }

    impl InvestApi for FakeApi {
        fn accounts(&self) -> impl Future<Output = color_eyre::Result<Vec<Account>>> + Send {
            ready(Ok(self.accounts.clone()))
        }

        fn portfolio_positions(
            &self,
            _: &str,
        ) -> impl Future<Output = color_eyre::Result<Vec<AccountPosition>>> + Send {
            ready(Ok(self.positions.clone()))
        }

        fn instrument(
            &self,
            figi: &str,
            _: Option<InstrumentKind>,
        ) -> impl Future<Output = color_eyre::Result<Instrument>> + Send {
            ready(
                self.instruments
                    .get(figi)
                    .cloned()
                    .ok_or_else(|| eyre::eyre!("Instrument {figi} not found")),
            )
        }

        fn find_instruments(
            &self,
            _: &str,
        ) -> impl Future<Output = color_eyre::Result<Vec<FoundInstrument>>> + Send {
            ready(Ok(self.found.clone()))
        }

        fn operations(
            &self,
            _: &str,
            filter: OperationsFilter<'_>,
        ) -> impl Future<Output = color_eyre::Result<Vec<Operation>>> + Send {
            ready(match filter {
                OperationsFilter::Instrument(figi) => self.get(&self.operations, figi),
                OperationsFilter::Kinds(kinds) => Ok(self
                    .operations
                    .values()
                    .flatten()
                    .filter(|op| kinds.contains(&op.kind))
                    .cloned()
                    .collect()),
            })
        }

        fn forecast(
            &self,
            instrument_uid: &str,
            _: Option<Currency>,
        ) -> impl Future<Output = color_eyre::Result<Option<Forecast>>> + Send {
            let forecasts: HashMap<String, Vec<Forecast>> = self
                .forecasts
                .iter()
                .map(|(uid, f)| (uid.clone(), vec![f.clone()]))
                .collect();
            ready(
                self.get(&forecasts, instrument_uid)
                    .map(|f| f.into_iter().next()),
            )
        }

        fn fundamentals(
            &self,
            _: &[String],
        ) -> impl Future<Output = color_eyre::Result<HashMap<String, Fundamentals>>> + Send
        {
            ready(Ok(HashMap::new()))
        }

        fn dividends(
            &self,
            figi: &str,
        ) -> impl Future<Output = color_eyre::Result<Vec<Dividend>>> + Send {
            ready(self.get(&self.dividends, figi))
        }

        fn bond_events(
            &self,
            figi: &str,
        ) -> impl Future<Output = color_eyre::Result<Vec<BondEvent>>> + Send {
            ready(self.get(&self.bond_events, figi))
        }

        fn fx_candidates(
            &self,
        ) -> impl Future<Output = color_eyre::Result<Vec<FxCandidate>>> + Send {
            ready(Ok(self.fx.clone()))
        }

        fn indices(&self) -> impl Future<Output = color_eyre::Result<Vec<MarketIndex>>> + Send {
            ready(Ok(self.indices.clone()))
        }
    }

    /// API with one open broker account and USD at 90 RUB on any date.
    fn api() -> FakeApi {
        FakeApi {
            accounts: vec![account("ACC", AccountType::Tinkoff, AccountStatus::Open)],
            fx: vec![FxCandidate {
                currency: Currency::USD,
                instrument_id: "USD".to_string(),
                lot: 1,
                nominal: dec!(1),
                settlement_is_rub: true,
            }],
            close: dec!(90),
            ..FakeApi::default()
        }
    }

    fn broker() -> AccountSelector {
        AccountSelector::Type(AccountType::Tinkoff)
    }

    /// Position of `quantity` units bought and priced at `price` RUB.
    fn priced(figi: &str, instrument_type: &str, quantity: i64, price: i64) -> AccountPosition {
        let price = Money::from_value(Decimal::from(price), Currency::RUB);
        AccountPosition {
            position: Ok(Position {
                average_buy_price: price,
                current_instrument_price: price,
                quantity: Decimal::from(quantity),
                ..position_in(Currency::RUB)
            }),
            ..portfolio_position(figi, instrument_type)
        }
    }

    /// Position the API gave no valid prices of.
    fn unpriced(figi: &str, instrument_type: &str) -> AccountPosition {
        AccountPosition {
            position: Err("Failed to get current price".to_string()),
            ..portfolio_position(figi, instrument_type)
        }
    }

    fn usd(units: i64) -> Money {
        Money::from_value(Decimal::from(units), Currency::USD)
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn accumulate_totals_rub_sums_converted_payments() {
        let totals = accumulate_totals_rub(
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

    fn portfolio_position(figi: &str, instrument_type: &str) -> AccountPosition {
        AccountPosition {
            figi: figi.to_string(),
            instrument_uid: String::new(),
            instrument_type: instrument_type.to_string(),
            position: Ok(position_in(Currency::RUB)),
        }
    }

    fn named(name: &str, ticker: &str) -> Instrument {
        Instrument {
            name: name.to_string(),
            ticker: Ticker::new(ticker),
            currency: Some(Currency::RUB),
            asset_uid: None,
            sector: None,
        }
    }

    #[test]
    fn account_portfolio_pairs_positions_with_instruments() {
        // Arrange
        let positions = vec![
            portfolio_position("FIGI1", "share"),
            portfolio_position("FIGI2", "bond"),
        ];
        let instruments = HashMap::from([("FIGI1".to_string(), named("Sber", "SBER"))]);

        // Act
        let portfolio = AccountPortfolio::new("account".to_string(), positions, &instruments);

        // Assert
        let holdings: Vec<_> = portfolio
            .holdings
            .iter()
            .map(|h| (h.kind, h.instrument.ticker.as_str(), h.figi.as_str()))
            .collect();
        assert_eq!(
            holdings,
            [
                (InstrumentKind::Share, "SBER", "FIGI1"),
                (InstrumentKind::Bond, "FIGI2", "FIGI2"),
            ]
        );
        assert!(portfolio.skipped().is_empty());
    }

    #[test]
    fn account_portfolio_sets_unsupported_positions_aside() {
        // Arrange
        let positions = vec![
            portfolio_position("OPT1", "option"),
            portfolio_position("FIGI1", "etf"),
        ];
        let instruments = HashMap::from([("OPT1".to_string(), named("Option", "OPTX"))]);

        // Act
        let portfolio = AccountPortfolio::new("account".to_string(), positions, &instruments);

        // Assert
        assert_eq!(portfolio.holdings.len(), 1);
        assert_eq!(portfolio.holdings[0].kind, InstrumentKind::Etf);
        let messages: Vec<_> = portfolio
            .skipped()
            .iter()
            .map(|e| format!("{e:#}"))
            .collect();
        assert_eq!(
            messages,
            ["Position OPTX (OPT1) skipped: Unsupported instrument type 'option'"]
        );
    }

    #[test]
    fn account_portfolio_reports_skipped_positions_on_every_call() {
        // Arrange
        let positions = vec![portfolio_position("OPT1", "option")];
        let portfolio = AccountPortfolio::new("account".to_string(), positions, &HashMap::new());

        // Act
        let first = portfolio.skipped();
        let second = portfolio.skipped();

        // Assert
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
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

    fn holdings(figis: &[&str]) -> Vec<Holding> {
        figis
            .iter()
            .map(|figi| Holding {
                kind: InstrumentKind::Share,
                instrument: instrument_or_figi(&HashMap::new(), figi),
                figi: (*figi).to_string(),
                instrument_uid: String::new(),
                position: position_in(Currency::RUB),
            })
            .collect()
    }

    #[tokio::test]
    async fn fetch_parallel_keeps_failed_positions_as_errors() {
        // Arrange
        let reports = Reports::new(FakeApi::default());
        let positions = holdings(&["OK", "BAD"]);

        // Act
        let results = reports
            .fetch_parallel(&positions, |_, figi| async move {
                if figi == "BAD" {
                    Err(eyre::eyre!("bad request"))
                } else {
                    Ok(vec![figi])
                }
            })
            .await;

        // Assert
        let results: HashMap<_, _> = results
            .into_iter()
            .map(|(holding, items)| (holding.figi, items))
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results["OK"].as_ref().ok(), Some(&vec!["OK".to_string()]));
        assert!(results["BAD"].is_err());
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
            var_margin: None,
        }
    }

    #[test]
    fn account_portfolio_sets_positions_without_valid_prices_aside() {
        // Arrange
        let positions = vec![
            unpriced("FIGI1", "share"),
            portfolio_position("FIGI2", "bond"),
        ];

        // Act
        let portfolio = AccountPortfolio::new("account".to_string(), positions, &HashMap::new());

        // Assert
        let figis: Vec<_> = portfolio.holdings.iter().map(|h| h.figi.as_str()).collect();
        assert_eq!(figis, ["FIGI2"]);
        let messages: Vec<_> = portfolio
            .skipped()
            .iter()
            .map(|e| format!("{e:#}"))
            .collect();
        assert_eq!(
            messages,
            ["Position FIGI1 (FIGI1) skipped: Failed to get current price"]
        );
    }

    #[rstest]
    #[case::shares(InstrumentKind::Share, &["S1"])]
    #[case::bonds(InstrumentKind::Bond, &["B1"])]
    #[case::futures(InstrumentKind::Future, &[])]
    fn account_portfolio_reports_skipped_positions_of_covered_kinds(
        #[case] covered: InstrumentKind,
        #[case] expected: &[&str],
    ) {
        // Arrange
        let positions = vec![
            unpriced("S1", "share"),
            unpriced("B1", "bond"),
            portfolio_position("OPT1", "option"),
        ];
        let portfolio = AccountPortfolio::new("account".to_string(), positions, &HashMap::new());

        // Act
        let skipped = portfolio.skipped_of(|k| k == covered);

        // Assert
        let messages: Vec<_> = skipped.iter().map(ToString::to_string).collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|figi| format!("Position {figi} ({figi}) skipped"))
            .collect();
        assert_eq!(messages, expected);
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
            account_type,
            status,
            opened: None,
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
        instrument_kind: InstrumentKind,
        kind: OperationKind,
        accrued_interest: Option<i64>,
    ) -> Operation {
        Operation {
            accrued_interest: accrued_interest.map(rub),
            ..operation(kind, Some(instrument_kind), rub(-1000))
        }
    }

    #[rstest]
    #[case::buy_pays_nkd(OperationKind::Buy, Some(19), Some(dec!(-19)))]
    #[case::sell_receives_nkd(OperationKind::Sell, Some(19), Some(dec!(19)))]
    #[case::buy_without_nkd(OperationKind::Buy, None, None)]
    #[case::zero_nkd(OperationKind::Buy, Some(0), None)]
    #[case::coupon_is_not_a_trade(OperationKind::Coupon, Some(19), None)]
    fn trade_accrued_interest_of_bond(
        #[case] kind: OperationKind,
        #[case] accrued_int: Option<i64>,
        #[case] expected: Option<Decimal>,
    ) {
        // Arrange
        let op = trade(InstrumentKind::Bond, kind, accrued_int);

        // Act
        let accrued = trade_accrued_interest(&op);

        // Assert
        assert_eq!(accrued.map(|m| m.value), expected);
    }

    #[test]
    fn trade_accrued_interest_ignores_shares() {
        // Arrange
        let op = trade(InstrumentKind::Share, OperationKind::Buy, Some(19));

        // Act
        let accrued = trade_accrued_interest(&op);

        // Assert
        assert!(accrued.is_none());
    }

    #[test]
    fn add_index_closes_merges_years_in_any_order() {
        // Arrange
        let mut prices = Ok(IndexPrices::new());

        // Act
        add_index_closes(&mut prices, Ok([(day(2025, 1, 3), dec!(2900))].into()));
        add_index_closes(&mut prices, Ok([(day(2024, 1, 3), dec!(3100))].into()));

        // Assert
        let expected: IndexPrices =
            [(day(2024, 1, 3), dec!(3100)), (day(2025, 1, 3), dec!(2900))].into();
        assert_eq!(prices.unwrap(), expected);
    }

    #[rstest]
    #[case::failed_year_after_loaded(true)]
    #[case::loaded_year_after_failed(false)]
    fn add_index_closes_fails_index_on_any_failed_year(#[case] loaded_first: bool) {
        // Arrange
        let mut prices = Ok(IndexPrices::new());
        let loaded = || Ok([(day(2025, 1, 3), dec!(2900))].into());
        let failed = || Err(eyre::eyre!("year failed"));

        // Act
        if loaded_first {
            add_index_closes(&mut prices, loaded());
            add_index_closes(&mut prices, failed());
        } else {
            add_index_closes(&mut prices, failed());
            add_index_closes(&mut prices, loaded());
        }

        // Assert
        assert!(prices.is_err());
    }

    #[test]
    fn index_histories_keep_benchmark_order_and_report_failures() {
        // Arrange
        let imoex: IndexPrices = [(day(2025, 1, 3), dec!(2900))].into();
        let loaded = [
            Ok(imoex.clone()),
            Err(eyre::eyre!("Index not found")),
            Ok(IndexPrices::new()),
            Err(eyre::eyre!("year failed")),
        ];

        // Act
        let (histories, failures) = index_histories(loaded);

        // Assert
        let tickers: Vec<_> = histories.iter().map(|h| h.benchmark.ticker).collect();
        assert_eq!(tickers, ["IMOEX", "MCFTR", "RGBI", "RGBITR"]);
        assert_eq!(histories[0].prices, imoex);
        assert!(histories[1].prices.is_empty());
        assert!(histories[3].prices.is_empty());
        let messages: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(messages, ["Index MCFTR skipped", "Index RGBITR skipped"]);
    }

    fn rub(units: i64) -> Money {
        Money::from_value(Decimal::from(units), Currency::RUB)
    }

    /// Closes of every instrument, whatever its ID and year.
    struct FixedCloses(DailyPrices);

    impl DailyCloses for FixedCloses {
        fn daily_closes(
            &self,
            _: &str,
            _: i32,
        ) -> impl Future<Output = color_eyre::Result<DailyPrices>> + Send {
            std::future::ready(Ok(self.0.clone()))
        }
    }

    /// Converter with USD at 90 RUB on the date of [`operation`].
    fn usd_at_90() -> RubConverter<FixedCloses> {
        RubConverter::new(
            [FxCandidate {
                currency: Currency::USD,
                instrument_id: "USD".to_string(),
                lot: 1,
                nominal: dec!(1),
                settlement_is_rub: true,
            }],
            FixedCloses([(day(2026, 9, 25), dec!(90))].into()),
        )
    }

    fn operation(
        kind: OperationKind,
        instrument_kind: Option<InstrumentKind>,
        payment: Money,
    ) -> Operation {
        Operation {
            id: String::new(),
            kind,
            state: OperationState::Executed,
            date: "2026-09-25T09:00:00Z".parse().unwrap(),
            figi: String::new(),
            instrument_kind,
            payment: Some(payment),
            commission: None,
            price: None,
            accrued_interest: None,
            quantity: 1,
            quantity_rest: 0,
            description: String::new(),
        }
    }

    #[tokio::test]
    async fn reduce_sums_income_net_of_taxes_and_fees() {
        // Arrange
        let operations = [
            operation(OperationKind::Coupon, Some(InstrumentKind::Bond), rub(50)),
            operation(
                OperationKind::CouponTax,
                Some(InstrumentKind::Bond),
                rub(-7),
            ),
            operation(OperationKind::Fee, Some(InstrumentKind::Bond), rub(-3)),
        ];

        // Act
        let totals = reduce(&usd_at_90(), &operations).await.unwrap();

        // Assert
        assert_eq!(totals.additional_profit.value, dec!(43));
        assert_eq!(totals.fees.value, dec!(-3));
        assert_eq!(totals.cash_flows.len(), 3);
    }

    #[tokio::test]
    async fn reduce_takes_other_taxes_out_of_income() {
        // Arrange
        let share = Some(InstrumentKind::Share);
        let operations = [
            operation(OperationKind::Dividend, share, rub(100)),
            operation(OperationKind::OtherTax, share, rub(-13)),
        ];

        // Act
        let totals = reduce(&usd_at_90(), &operations).await.unwrap();

        // Assert
        assert_eq!(totals.additional_profit.value, dec!(87));
    }

    #[tokio::test]
    async fn reduce_counts_bond_trade_accrued_interest_as_income() {
        // Arrange
        let buy = Operation {
            accrued_interest: Some(rub(20)),
            ..operation(OperationKind::Buy, Some(InstrumentKind::Bond), rub(-1020))
        };

        // Act
        let totals = reduce(&usd_at_90(), &[buy]).await.unwrap();

        // Assert
        assert_eq!(totals.additional_profit.value, dec!(-20));
        assert!(totals.fees.value.is_zero());
        assert_eq!(totals.cash_flows[0].amount, dec!(-1020));
    }

    #[tokio::test]
    async fn reduce_counts_trade_commission_as_fee_and_cash_flow() {
        // Arrange: payment is price × quantity; commission sits beside it
        let sale = Operation {
            commission: Some(Money::from_value(dec!(-142.62), Currency::RUB)),
            ..operation(
                OperationKind::Sell,
                Some(InstrumentKind::Currency),
                Money::from_value(dec!(285243.75), Currency::RUB),
            )
        };

        // Act
        let totals = reduce(&usd_at_90(), &[sale]).await.unwrap();

        // Assert
        assert_eq!(totals.fees.value, dec!(-142.62));
        assert!(totals.additional_profit.value.is_zero());
        let amounts: Vec<Decimal> = totals.cash_flows.iter().map(|f| f.amount).collect();
        assert_eq!(amounts, vec![dec!(285243.75), dec!(-142.62)]);
    }

    #[tokio::test]
    async fn reduce_leaves_fee_operation_commission_out() {
        // Arrange: a service fee already is its payment
        let fee = Operation {
            commission: Some(rub(-3)),
            ..operation(OperationKind::Fee, Some(InstrumentKind::Share), rub(-3))
        };

        // Act
        let totals = reduce(&usd_at_90(), &[fee]).await.unwrap();

        // Assert
        assert_eq!(totals.fees.value, dec!(-3));
        assert_eq!(totals.cash_flows.len(), 1);
    }

    #[tokio::test]
    async fn reduce_converts_foreign_payments_to_rub() {
        // Arrange
        let dividend = operation(
            OperationKind::Dividend,
            Some(InstrumentKind::Share),
            usd(10),
        );

        // Act
        let totals = reduce(&usd_at_90(), &[dividend]).await.unwrap();

        // Assert
        assert_eq!(
            totals.additional_profit,
            Money::from_value(dec!(900), Currency::RUB)
        );
        assert_eq!(totals.cash_flows[0].amount, dec!(900));
    }

    #[tokio::test]
    async fn reduce_leaves_zero_payments_out_of_cash_flows() {
        // Arrange
        let operations = [
            operation(OperationKind::Dividend, Some(InstrumentKind::Share), rub(0)),
            operation(OperationKind::Other, None, rub(100)),
        ];

        // Act
        let totals = reduce(&usd_at_90(), &operations).await.unwrap();

        // Assert
        assert!(totals.additional_profit.value.is_zero());
        assert_eq!(totals.cash_flows.len(), 1);
    }

    #[rstest]
    #[case::exchange_before_broker(&["TCS00A107T19", "BBG004730N88"], Some("BBG004730N88"))]
    #[case::broker_first_in_input(&["BBG004730N88", "TCS00A107T19"], Some("BBG004730N88"))]
    #[case::exchange_ones_ordered(&["BBG2", "BBG1", "TCS1"], Some("BBG1"))]
    #[case::only_broker_ones(&["TCS2", "TCS1"], Some("TCS1"))]
    #[case::none(&[], None)]
    fn preferred_figi_cases(#[case] figis: &[&str], #[case] expected: Option<&str>) {
        // Arrange
        let figis: Vec<String> = figis.iter().map(ToString::to_string).collect();

        // Act
        let figi = preferred_figi(&figis);

        // Assert
        assert_eq!(figi.map(String::as_str), expected);
    }

    #[tokio::test]
    async fn load_portfolio_keeps_only_requested_kind() {
        // Arrange
        let reports = Reports::new(FakeApi {
            positions: vec![
                priced("FIGI1", "share", 1, 100),
                priced("FIGI2", "bond", 1, 900),
            ],
            instruments: HashMap::from([("FIGI1".to_string(), named("Sber", "SBER"))]),
            ..api()
        });

        // Act
        let portfolio = reports
            .load_portfolio(&broker(), Some(InstrumentKind::Share))
            .await
            .unwrap();

        // Assert
        assert_eq!(portfolio.account_id, "ACC");
        assert_eq!(portfolio.holdings.len(), 1);
        assert_eq!(portfolio.holdings[0].kind, InstrumentKind::Share);
        assert_eq!(portfolio.holdings[0].instrument.ticker.as_str(), "SBER");
    }

    #[tokio::test]
    async fn load_portfolio_names_instrument_not_loaded_by_figi() {
        // Arrange
        let reports = Reports::new(FakeApi {
            positions: vec![priced("FIGI1", "share", 1, 100)],
            ..api()
        });

        // Act
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Assert
        assert_eq!(portfolio.holdings[0].instrument.ticker.as_str(), "FIGI1");
    }

    #[tokio::test]
    async fn build_portfolio_sums_operations_into_totals() {
        // Arrange
        let api = FakeApi {
            positions: vec![priced("FIGI1", "share", 10, 100)],
            instruments: HashMap::from([("FIGI1".to_string(), named("Sber", "SBER"))]),
            operations: HashMap::from([(
                "FIGI1".to_string(),
                vec![
                    operation(OperationKind::Buy, Some(InstrumentKind::Share), rub(-1000)),
                    operation(OperationKind::Dividend, Some(InstrumentKind::Share), usd(1)),
                    operation(OperationKind::Fee, Some(InstrumentKind::Share), rub(-3)),
                ],
            )]),
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (loaded, failures) = reports.build_portfolio(&portfolio, false, None).await;

        // Assert
        assert!(failures.is_empty());
        let papers: Vec<_> = loaded.papers().collect();
        let [LoadedPaper::Share(paper)] = papers.as_slice() else {
            panic!("expected one share");
        };
        assert_eq!(paper.ticker.as_str(), "SBER");
        assert_eq!(paper.totals.additional_profit.value, dec!(90));
        assert_eq!(paper.totals.fees.value, dec!(-3));
        assert_eq!(paper.totals.cash_flows.len(), 3);
    }

    #[tokio::test]
    async fn build_portfolio_reports_unsupported_and_failed_positions() {
        // Arrange
        let api = FakeApi {
            positions: vec![
                priced("OPT1", "option", 1, 10),
                priced("FIGI1", "share", 1, 100),
            ],
            failing: vec!["FIGI1".to_string()],
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (loaded, failures) = reports.build_portfolio(&portfolio, false, None).await;

        // Assert
        assert_eq!(loaded.papers().count(), 0);
        let messages: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "Position OPT1 (OPT1) skipped",
                "Position FIGI1 (FIGI1) skipped"
            ]
        );
    }

    fn dividend(payment: DateTime<Utc>, usd_per_share: i64) -> Dividend {
        Dividend {
            per_share: Money::from_value(Decimal::from(usd_per_share), Currency::USD),
            payment_date: Some(payment),
            record_date: None,
            dividend_type: String::new(),
        }
    }

    #[tokio::test]
    async fn dividend_calendar_has_upcoming_payments_in_rub_by_date() {
        // Arrange
        let now = Utc::now();
        let api = FakeApi {
            positions: vec![priced("FIGI1", "share", 10, 100)],
            dividends: HashMap::from([(
                "FIGI1".to_string(),
                vec![
                    dividend(now + ChronoDuration::days(20), 1),
                    dividend(now + ChronoDuration::days(10), 2),
                    dividend(now + ChronoDuration::days(400), 3),
                    dividend(now - ChronoDuration::days(10), 4),
                ],
            )]),
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (calendar, failures) = reports
            .get_calendar(&portfolio, CalendarKind::Dividends, 365)
            .await
            .unwrap();

        // Assert
        assert!(failures.is_empty());
        let Calendar::Dividends(calendar) = calendar else {
            panic!("expected dividend calendar");
        };
        let totals: Vec<_> = calendar.upcoming.iter().map(|d| d.total_dividend).collect();
        assert_eq!(
            totals,
            [
                Money::from_value(dec!(1800), Currency::RUB),
                Money::from_value(dec!(900), Currency::RUB),
            ]
        );
    }

    #[tokio::test]
    async fn dividend_calendar_reports_skipped_shares_only() {
        // Arrange
        let api = FakeApi {
            positions: vec![
                unpriced("S1", "share"),
                unpriced("B1", "bond"),
                portfolio_position("OPT1", "option"),
            ],
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (_, failures) = reports
            .get_calendar(&portfolio, CalendarKind::Dividends, 365)
            .await
            .unwrap();

        // Assert
        let messages: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(messages, ["Position S1 (S1) skipped"]);
    }

    fn coupon(date: DateTime<Utc>, per_bond: i64) -> BondEvent {
        BondEvent {
            kind: BondEventKind::Coupon,
            date,
            per_bond: Money::from_value(Decimal::from(per_bond), Currency::RUB),
        }
    }

    #[tokio::test]
    async fn coupon_calendar_skips_bonds_failed_to_load() {
        // Arrange
        let in_month = Utc::now() + ChronoDuration::days(30);
        let api = FakeApi {
            positions: vec![priced("B1", "bond", 2, 1000), priced("B2", "bond", 1, 1000)],
            bond_events: HashMap::from([
                ("B1".to_string(), vec![coupon(in_month, 25)]),
                ("B2".to_string(), vec![coupon(in_month, 40)]),
            ]),
            failing: vec!["B2".to_string()],
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (calendar, failures) = reports
            .get_calendar(&portfolio, CalendarKind::Coupons, 365)
            .await
            .unwrap();

        // Assert
        let Calendar::Coupons(calendar) = calendar else {
            panic!("expected coupon calendar");
        };
        let tickers: Vec<_> = calendar
            .upcoming
            .iter()
            .map(|c| c.ticker.as_str())
            .collect();
        assert_eq!(tickers, ["B1"]);
        assert_eq!(calendar.upcoming[0].total_coupon.value, dec!(50));
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].to_string(), "Position B2 (B2) skipped");
    }

    #[tokio::test]
    async fn income_forecast_leaves_unsupported_positions_out_of_value() {
        // Arrange
        let api = FakeApi {
            positions: vec![
                priced("FIGI1", "share", 2, 100),
                priced("OPT1", "option", 1, 1000),
            ],
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (forecast, failures) = reports.get_income_forecast(&portfolio).await.unwrap();

        // Assert
        assert_eq!(
            forecast.portfolio_value,
            Money::from_value(dec!(200), Currency::RUB)
        );
        let messages: Vec<_> = failures.iter().map(|e| format!("{e:#}")).collect();
        assert_eq!(
            messages,
            ["Position OPT1 (OPT1) skipped: Unsupported instrument type 'option'"]
        );
    }

    #[tokio::test]
    async fn tax_report_counts_repeated_operations_once() {
        // Arrange
        let with_id = |id: &str, op: Operation| Operation {
            id: id.to_string(),
            ..op
        };
        let dividend = with_id(
            "1",
            operation(OperationKind::Dividend, Some(InstrumentKind::Share), usd(1)),
        );
        let api = FakeApi {
            operations: HashMap::from([(
                "FIGI1".to_string(),
                vec![
                    dividend.clone(),
                    dividend,
                    with_id(
                        "2",
                        operation(
                            OperationKind::DividendTax,
                            Some(InstrumentKind::Share),
                            rub(-12),
                        ),
                    ),
                    with_id(
                        "3",
                        operation(OperationKind::Buy, Some(InstrumentKind::Share), rub(-1000)),
                    ),
                ],
            )]),
            ..api()
        };

        // Act
        let report = Reports::new(api).get_tax_report(&broker()).await.unwrap();

        // Assert
        let total = report.total();
        assert_eq!(total.dividends.value, dec!(90));
        assert_eq!(total.dividend_tax.value, dec!(12));
    }

    fn found(ticker: &str, figi: &str) -> FoundInstrument {
        FoundInstrument {
            ticker: ticker.to_string(),
            figi: figi.to_string(),
            name: format!("{ticker} {figi}"),
        }
    }

    #[tokio::test]
    async fn history_leaves_futures_price_in_points() {
        // Arrange: the API writes points into a money field, here tagged USD
        let mut buy = operation(OperationKind::Buy, Some(InstrumentKind::Future), usd(-10));
        buy.price = Some(Money::from_value(dec!(93_250), Currency::USD));

        // Act
        let history = Reports::new(api())
            .history_in_rub(&[buy], &found("SiZ5", "FUTSI"))
            .await
            .unwrap()
            .unwrap();

        // Assert: points stay 93 250; the cash payment is converted at 90
        let rendered = history.to_string();
        assert_eq!(history.items[0].price, HistoryPrice::Points(dec!(93_250)));
        assert_eq!(history.items[0].payment.value, dec!(-900));
        assert!(rendered.contains("pt"), "{rendered}");
        assert!(!rendered.contains('$'), "{rendered}");
    }

    #[tokio::test]
    async fn history_converts_share_price_to_rub() {
        // Arrange
        let mut buy = operation(OperationKind::Buy, Some(InstrumentKind::Share), usd(-10));
        buy.price = Some(usd(2));

        // Act
        let history = Reports::new(api())
            .history_in_rub(&[buy], &found("SBER", "BBG004730N88"))
            .await
            .unwrap()
            .unwrap();

        // Assert
        assert_eq!(
            history.items[0].price,
            HistoryPrice::Money(Money::from_value(dec!(180), Currency::RUB))
        );
        assert_eq!(history.items[0].payment.value, dec!(-900));
    }

    #[tokio::test]
    async fn history_prefers_exchange_instrument_with_operations() {
        // Arrange
        let buy = || {
            vec![operation(
                OperationKind::Buy,
                Some(InstrumentKind::Share),
                rub(-1000),
            )]
        };
        let api = FakeApi {
            found: vec![
                found("SBER", "TCS00A107T19"),
                found("sber", "BBG004730N88"),
                found("SBERP", "BBG0047315Y7"),
            ],
            operations: HashMap::from([
                ("TCS00A107T19".to_string(), buy()),
                ("BBG004730N88".to_string(), buy()),
                ("BBG0047315Y7".to_string(), buy()),
            ]),
            ..api()
        };

        // Act
        let history = Reports::new(api)
            .get_history(&broker(), "SBER")
            .await
            .unwrap();

        // Assert
        assert_eq!(history.figi, "BBG004730N88");
        assert_eq!(history.items.len(), 1);
    }

    #[tokio::test]
    async fn history_fails_without_operations() {
        // Arrange
        let api = FakeApi {
            found: vec![found("SBER", "BBG004730N88")],
            ..api()
        };

        // Act
        let result = Reports::new(api).get_history(&broker(), "SBER").await;

        // Assert
        let message = result.err().map(|e| e.to_string());
        assert_eq!(
            message.as_deref(),
            Some("No operations with SBER on the account")
        );
    }

    #[tokio::test]
    async fn share_analytics_reports_forecasts_failed_to_load() {
        // Arrange
        let share = |figi: &str, uid: &str| AccountPosition {
            instrument_uid: uid.to_string(),
            ..priced(figi, "share", 1, 100)
        };
        let forecast = Forecast {
            recommendation: None,
            current_price: Money::from_value(dec!(100), Currency::RUB),
            target_price: Money::from_value(dec!(120), Currency::RUB),
            min_target: Money::from_value(dec!(110), Currency::RUB),
            max_target: Money::from_value(dec!(130), Currency::RUB),
            targets: Vec::new(),
        };
        let api = FakeApi {
            positions: vec![share("FIGI1", "U1"), share("FIGI2", "U2")],
            instruments: HashMap::from([
                ("FIGI1".to_string(), named("Sber", "SBER")),
                ("FIGI2".to_string(), named("Gazprom", "GAZP")),
            ]),
            forecasts: HashMap::from([("U1".to_string(), forecast)]),
            failing: vec!["U2".to_string()],
            ..api()
        };
        let reports = Reports::new(api);
        let portfolio = reports.load_portfolio(&broker(), None).await.unwrap();

        // Act
        let (analytics, failures) = reports.get_share_analytics(&portfolio).await;

        // Assert
        let forecasts: Vec<_> = analytics
            .shares()
            .iter()
            .map(|s| (s.ticker.as_str(), s.forecast.is_some()))
            .collect();
        assert!(forecasts.contains(&("SBER", true)), "{forecasts:?}");
        assert!(forecasts.contains(&("GAZP", false)), "{forecasts:?}");
        let messages: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(messages, ["Forecast for GAZP not loaded"]);
    }

    #[tokio::test]
    async fn index_histories_report_indices_not_found() {
        // Arrange
        let api = FakeApi {
            indices: vec![MarketIndex {
                ticker: "IMOEX".to_string(),
                uid: "IMOEX-UID".to_string(),
            }],
            ..api()
        };
        let this_year = Utc::now().year();

        // Act
        let (histories, failures) = Reports::new(api)
            .get_index_histories(this_year)
            .await
            .unwrap();

        // Assert
        assert_eq!(
            histories[0].prices.get(&day(this_year, 1, 1)),
            Some(&dec!(90))
        );
        assert!(histories[1..].iter().all(|h| h.prices.is_empty()));
        let messages: Vec<_> = failures.iter().map(ToString::to_string).collect();
        assert_eq!(
            messages,
            [
                "Index MCFTR skipped",
                "Index RGBI skipped",
                "Index RGBITR skipped"
            ]
        );
    }

    /// Executed trade of `figi` on Moscow `date` in RUB.
    fn lot_operation(
        figi: &str,
        kind: OperationKind,
        date: &str,
        quantity: i64,
        payment: i64,
    ) -> Operation {
        Operation {
            id: format!("{figi}-{date}-{kind:?}"),
            figi: figi.to_string(),
            date: format!("{date}T09:00:00Z").parse().unwrap(),
            quantity,
            ..operation(kind, Some(InstrumentKind::Share), rub(payment))
        }
    }

    fn sale_tax_api() -> FakeApi {
        let commission = |op: Operation, value: i64| Operation {
            commission: Some(rub(value)),
            ..op
        };
        let dividend = |date: &str, payment: Money| Operation {
            payment: Some(payment),
            ..lot_operation("F1", OperationKind::Dividend, date, 0, 0)
        };
        let currency_buy = Operation {
            instrument_kind: Some(InstrumentKind::Currency),
            ..lot_operation("USD", OperationKind::Buy, "2026-01-15", 100, -9000)
        };
        FakeApi {
            positions: vec![
                priced("F1", "share", 15, 300),
                priced("F2", "bond", 3, 1000),
            ],
            instruments: HashMap::from([
                ("F1".to_string(), named("Sber", "SBER")),
                ("F2".to_string(), named("OFZ", "SU26238")),
            ]),
            operations: HashMap::from([
                (
                    "F1".to_string(),
                    vec![
                        commission(
                            lot_operation("F1", OperationKind::Buy, "2020-01-10", 10, -1000),
                            -10,
                        ),
                        lot_operation("F1", OperationKind::Buy, "2026-02-01", 10, -2000),
                        commission(
                            lot_operation("F1", OperationKind::Sell, "2026-03-01", 5, 1500),
                            -5,
                        ),
                        dividend("2026-07-01", usd(1)),
                        dividend("2025-07-01", rub(1000)),
                    ],
                ),
                ("USD".to_string(), vec![currency_buy]),
            ]),
            ..api()
        }
    }

    fn today() -> DateTime<Utc> {
        "2026-10-27T09:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn sale_tax_matches_lots_by_fifo_and_counts_year_income() {
        // Arrange
        let reports = Reports::new(sale_tax_api());

        // Act
        let (report, failures) = reports
            .get_sale_tax(&broker(), None, today())
            .await
            .unwrap();

        // Assert
        assert_eq!(failures.len(), 0);
        let tickers: Vec<&str> = report.positions.iter().map(|p| p.ticker.as_str()).collect();
        assert_eq!(tickers, ["SBER", "SU26238"]);
        assert_eq!(report.unknown().count(), 1);
        assert_eq!(report.year_income.securities.value, dec!(990));
        assert_eq!(report.year_income.payments.value, dec!(90));
        assert_eq!(report.cost().value, dec!(2505));
        assert_eq!(report.result().value, dec!(1995));
        assert_eq!(report.ldv_exemption().value, dec!(995));
        assert_eq!(report.tax().value, dec!(130));
    }

    #[tokio::test]
    async fn sale_tax_of_ticker_prices_its_position_only() {
        // Arrange
        let reports = Reports::new(sale_tax_api());

        // Act
        let (report, _) = reports
            .get_sale_tax(&broker(), Some("sber"), today())
            .await
            .unwrap();

        // Assert
        let tickers: Vec<&str> = report.positions.iter().map(|p| p.ticker.as_str()).collect();
        assert_eq!(tickers, ["SBER"]);
        assert_eq!(report.positions[0].lots().len(), 2);
    }

    #[tokio::test]
    async fn sale_tax_of_ticker_not_in_portfolio_fails() {
        // Arrange
        let reports = Reports::new(sale_tax_api());

        // Act
        let result = reports.get_sale_tax(&broker(), Some("GAZP"), today()).await;

        // Assert
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sale_tax_of_iis_is_not_taxed() {
        // Arrange
        let api = FakeApi {
            accounts: vec![account("IIS", AccountType::TinkoffIis, AccountStatus::Open)],
            ..sale_tax_api()
        };
        let reports = Reports::new(api);
        let iis = AccountSelector::Type(AccountType::TinkoffIis);

        // Act
        let (report, _) = reports.get_sale_tax(&iis, None, today()).await.unwrap();

        // Assert
        assert!(!report.taxed);
        assert_eq!(report.tax().value, Decimal::ZERO);
    }
}
