use color_eyre::eyre;
use iso_currency::Currency;
use itertools::Itertools;
use std::collections::HashMap;
use std::sync::Arc;
use tinkoff_invest_api::{
    TIError, TIResult, TinkoffInvestService,
    tcs::{
        Account, AccountType, Coupon, Dividend, FindInstrumentRequest, GetAccountsRequest,
        GetBondCouponsRequest, GetDividendsRequest, InstrumentShort, InstrumentStatus,
        InstrumentType, InstrumentsRequest, Operation, OperationState, OperationType,
        OperationsRequest, PortfolioPosition, PortfolioRequest, portfolio_request::CurrencyRequest,
    },
};
use tokio::time::{Duration, sleep};

use crate::{
    domain::{
        CouponCalendar, CouponPayment, DividendCalendar, DividendPayment, History, HistoryItem,
        Instrument, Money, Paper, Position, Profit, Totals,
    },
    to_currency, to_datetime_utc, to_decimal, to_money,
};

#[derive(Default)]
pub struct AccountPortfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

#[derive(Clone)]
pub struct TinkoffInvestment {
    service: Arc<TinkoffInvestService>,
}

enum OperationInfluence {
    /// Anything that affects to dividents or coupons value.<br/>
    /// Including negative values like divident tax etc. to calculate pure income<br/>
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

        Ok(Self {
            currency,
            average_buy_price,
            current_instrument_price,
            quantity,
        })
    }
}

macro_rules! collect {
    ($response:ident) => {{
        $response
            .into_inner()
            .instruments
            .into_iter()
            .map(|x| {
                (
                    x.figi.clone(),
                    Instrument {
                        name: x.name.clone(),
                        ticker: x.ticker.clone(),
                    },
                )
            })
            .collect::<HashMap<String, Instrument>>()
    }};
}

macro_rules! impl_get_until_done {
    ($(($target_method:ident, $source_method:ident)),*) => {
        $(
            #[allow(clippy::missing_errors_doc)]
            pub async fn $target_method(&self) -> color_eyre::Result<HashMap<String, Instrument>> {
                with_retry(|| self.$source_method()).await
            }
        )*
    };
}

macro_rules! impl_get_instrument_method {
    ($(($target_method:ident, $source_method:ident)),*) => {
        $(
            async fn $target_method(&self) -> TIResult<HashMap<String, Instrument>> {
                let channel = self.service.create_channel().await?;
                let mut instruments = self.service.instruments(channel).await?;
                let instruments = instruments
                    .$source_method(InstrumentsRequest {
                        instrument_status: Some(InstrumentStatus::All as i32),
                        instrument_exchange: None,
                    })
                    .await?;
                let instruments = collect!(instruments);
                Ok(instruments)
            }
        )*
    };
}

async fn with_retry<T, F, Fut>(f: F) -> color_eyre::Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, TIError>>,
{
    let mut delay = Duration::from_millis(100);
    for attempt in 1..=5 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 5 => {
                return Err(eyre::eyre!("{e:?}"));
            }
            Err(_) => {
                sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!()
}

impl TinkoffInvestment {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            service: Arc::new(TinkoffInvestService::new(token)),
        }
    }

    impl_get_instrument_method!(
        (get_all_bonds, bonds),
        (get_all_shares, shares),
        (get_all_etfs, etfs),
        (get_all_currencies, currencies),
        (get_all_futures, futures)
    );

    impl_get_until_done!(
        (get_all_bonds_until_done, get_all_bonds),
        (get_all_shares_until_done, get_all_shares),
        (get_all_etfs_until_done, get_all_etfs),
        (get_all_currencies_until_done, get_all_currencies),
        (get_all_futures_until_done, get_all_futures)
    );

    /// Fetches data for each position in parallel,
    /// limiting concurrent requests with a semaphore.
    ///
    /// Returns pairs of (position, list of items). Positions with errors
    /// are skipped (empty vector), and task panics are logged to stderr.
    async fn fetch_parallel<T, F, Fut>(
        &self,
        positions: &[PortfolioPosition],
        fetch: F,
    ) -> Vec<(PortfolioPosition, Vec<T>)>
    where
        T: Send + 'static,
        F: Fn(Self, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = color_eyre::Result<Vec<T>>> + Send,
    {
        use tokio::sync::Semaphore;
        use tokio::task::JoinSet;

        const MAX_CONCURRENT_REQUESTS: usize = 10;

        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS));
        let fetch = Arc::new(fetch);
        let mut set = JoinSet::new();

        for position in positions {
            let client = self.clone();
            let figi = position.figi.clone();
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let position_clone = position.clone();
            let fetch = Arc::clone(&fetch);

            set.spawn(async move {
                let _permit = permit;
                let items = fetch(client, figi).await.unwrap_or_default();
                (position_clone, items)
            });
        }

        let mut results = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(pair) => results.push(pair),
                Err(e) => eprintln!("Task panicked or cancelled: {e}"),
            }
        }
        results
    }

    async fn get_portfolio(&self, account: AccountType) -> TIResult<AccountPortfolio> {
        let (channel, users_channel) =
            tokio::join!(self.service.create_channel(), self.service.create_channel());
        let channel = channel?;
        let users_channel = users_channel?;

        let (users, operations) = tokio::join!(
            self.service.users(users_channel),
            self.service.operations(channel)
        );

        let mut operations = operations?;
        let mut users = users?;

        let accounts = users.get_accounts(GetAccountsRequest {}).await?;

        let Some(account) = accounts
            .get_ref()
            .accounts
            .iter()
            .find(|a| a.r#type() == account)
        else {
            return Ok(AccountPortfolio::default());
        };

        let portfolio = operations
            .get_portfolio(PortfolioRequest {
                account_id: account.id.clone(),
                currency: Some(CurrencyRequest::Rub as i32),
            })
            .await?;
        Ok(AccountPortfolio {
            account_id: account.id.clone(),
            positions: portfolio.into_inner().positions,
        })
    }

    /// Get an account by type.
    ///
    /// # Panics
    ///
    /// Panics if no account get.
    ///
    /// # Errors
    ///
    /// This function will return an error if account cannot be get.
    pub async fn get_account(&self, account_type: AccountType) -> color_eyre::Result<Account> {
        let channel = self
            .service
            .create_channel()
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let mut users = self
            .service
            .users(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let accounts = users
            .get_accounts(GetAccountsRequest {})
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let all_accounts = &accounts.get_ref().accounts;
        let account = all_accounts
            .iter()
            .find(|a| a.r#type() == account_type)
            .or_else(|| all_accounts.first())
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
        let channel = self
            .service
            .create_channel()
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
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

    async fn get_operations(&self, account_id: String, figi: String) -> TIResult<Vec<Operation>> {
        let channel = self.service.create_channel().await?;
        let mut operations = self.service.operations(channel).await?;
        let operations = operations
            .get_operations(OperationsRequest {
                account_id,
                from: None,
                to: None,
                state: Some(OperationState::Executed as i32),
                figi: Some(figi),
            })
            .await?;

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

    pub async fn create_paper_from_position<P: Profit>(
        &self,
        instruments: &HashMap<String, Instrument>,
        account_id: String,
        portfolio_position: &PortfolioPosition,
        profit: P,
    ) -> Option<Paper<P>> {
        let position = Position::try_from(portfolio_position).ok()?;

        let executed_ops = self
            .get_operations_until_done(account_id, portfolio_position.figi.clone())
            .await;
        let executed_ops = executed_ops.ok()?;

        let totals = Self::reduce(&executed_ops, position.currency);

        let instrument = instruments.get(&portfolio_position.figi)?;
        Some(Paper {
            name: instrument.name.clone(),
            ticker: instrument.ticker.clone(),
            figi: portfolio_position.figi.clone(),
            position,
            totals,
            profit,
        })
    }

    #[must_use]
    fn reduce(operations: &[Operation], currency: iso_currency::Currency) -> Totals {
        let mut fees = Money::zero(currency);
        let mut additional_profit = Money::zero(currency);
        for op in operations {
            let Some(payment) = crate::to_money(op.payment.as_ref()) else {
                continue;
            };
            match to_influence(op.operation_type()) {
                OperationInfluence::PureIncome => {
                    additional_profit += payment;
                }
                OperationInfluence::Fees => {
                    fees += payment;
                }
                OperationInfluence::Unspecified => {}
            }
        }
        Totals {
            additional_profit,
            fees,
        }
    }

    /// Get dividend calendar for the portfolio
    ///
    /// # Errors
    ///
    /// This function will return an error if dividends cannot be retrieved from remote server.
    pub async fn get_dividend_calendar(
        &self,
        _account_id: String,
        positions: &[PortfolioPosition],
        instruments: &HashMap<String, Instrument>,
    ) -> color_eyre::Result<DividendCalendar> {
        let now = chrono::Utc::now();
        let instruments = Arc::new(instruments.clone());

        let pairs = self
            .fetch_parallel(positions, |client, figi| {
                Box::pin(async move { client.get_dividends_for_figi(figi).await })
            })
            .await;

        let mut upcoming = Vec::new();
        for (position, dividends) in pairs {
            let Some(instrument) = instruments.get(&position.figi) else {
                continue;
            };
            for dividend in dividends {
                let dividend_per_share = dividend
                    .dividend_net
                    .as_ref()
                    .and_then(|d| to_money(Some(d)))
                    .unwrap_or_else(|| Money::zero(Currency::RUB));

                let ex_dividend_date = dividend
                    .record_date
                    .as_ref()
                    .map_or(now, |d| to_datetime_utc(Some(d)));

                if ex_dividend_date <= now {
                    continue;
                }

                let quantity = to_decimal(position.quantity.as_ref());
                upcoming.push(DividendPayment {
                    figi: position.figi.clone(),
                    ticker: instrument.ticker.clone(),
                    name: instrument.name.clone(),
                    currency: dividend_per_share.currency,
                    dividend_per_share,
                    total_dividend: dividend_per_share * quantity,
                    quantity,
                    ex_dividend_date,
                    payment_date: dividend
                        .payment_date
                        .as_ref()
                        .map(|d| to_datetime_utc(Some(d))),
                    dividend_type: dividend.dividend_type,
                });
            }
        }

        upcoming.sort_by_key(|a| a.ex_dividend_date);
        Ok(DividendCalendar { upcoming })
    }

    async fn get_dividends_for_figi(&self, figi: String) -> color_eyre::Result<Vec<Dividend>> {
        let channel = self
            .service
            .create_channel()
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
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
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        Ok(response.into_inner().dividends)
    }

    /// Get coupon calendar for the portfolio
    ///
    /// # Errors
    ///
    /// This function will return an error if coupons cannot be retrieved from remote server.
    pub async fn get_coupon_calendar(
        &self,
        _account_id: String,
        positions: &[PortfolioPosition],
        instruments: &HashMap<String, Instrument>,
    ) -> color_eyre::Result<CouponCalendar> {
        let now = chrono::Utc::now();
        let instruments = Arc::new(instruments.clone());

        // Filter only bonds before launching parallel tasks
        let bond_positions: Vec<PortfolioPosition> = positions
            .iter()
            .filter(|p| p.instrument_type == "bond")
            .cloned()
            .collect();

        let pairs = self
            .fetch_parallel(&bond_positions, |client, figi| {
                Box::pin(async move { client.get_coupons_for_figi(figi).await })
            })
            .await;

        let mut upcoming = Vec::new();
        for (position, coupons) in pairs {
            let Some(instrument) = instruments.get(&position.figi) else {
                continue;
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
                    .map_or(now, |d| to_datetime_utc(Some(d)));

                if coupon_date <= now {
                    continue;
                }

                let quantity = to_decimal(position.quantity.as_ref());
                upcoming.push(CouponPayment {
                    figi: position.figi.clone(),
                    ticker: instrument.ticker.clone(),
                    name: instrument.name.clone(),
                    currency: coupon_value.currency,
                    coupon_per_bond: coupon_value,
                    total_coupon: coupon_value * quantity,
                    quantity,
                    coupon_date,
                    coupon_type: coupon_type_to_str(coupon.coupon_type()).to_string(),
                });
            }
        }

        upcoming.sort_by_key(|a| a.coupon_date);
        Ok(CouponCalendar { upcoming })
    }

    async fn get_coupons_for_figi(&self, figi: String) -> color_eyre::Result<Vec<Coupon>> {
        let channel = self
            .service
            .create_channel()
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
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
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        Ok(response.into_inner().events)
    }
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

impl HistoryItem {
    #[must_use]
    pub fn from(op: &Operation) -> Self {
        let currency =
            Currency::from_code(&op.currency.to_ascii_uppercase()).unwrap_or(Currency::RUB);
        let payment = if let Some(payment) = to_money(op.payment.as_ref()) {
            payment
        } else {
            Money::zero(currency)
        };
        let price = if let Some(price) = to_money(op.price.as_ref()) {
            price
        } else {
            Money::zero(currency)
        };
        let state = match op.state() {
            OperationState::Unspecified => "Not specified",
            OperationState::Executed => "Executed",
            OperationState::Canceled => "Canceled",
            OperationState::Progress => "In progress",
        };

        let dt = to_datetime_utc(op.date.as_ref());
        Self {
            datetime: dt,
            quantity: op.quantity,
            quantity_rest: op.quantity_rest,
            price,
            payment,
            description: op.r#type.clone(),
            operation_state: state,
        }
    }
}

impl History {
    pub fn new(operations: &[Operation], instrument: &InstrumentShort) -> Option<Self> {
        let items = operations
            .iter()
            .unique_by(|op| &op.id)
            .map(HistoryItem::from)
            .sorted_by(|a, b| Ord::cmp(&a.datetime, &b.datetime))
            .collect_vec();
        let currency = items.first()?.payment.currency;
        Some(Self {
            name: instrument.name.clone(),
            ticker: instrument.ticker.clone(),
            figi: instrument.figi.clone(),
            items,
            currency,
        })
    }
}