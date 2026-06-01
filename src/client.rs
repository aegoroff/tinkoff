use color_eyre::eyre;
use iso_currency::Currency;
use itertools::Itertools;
use std::collections::HashMap;
use std::sync::Arc;
use tinkoff_invest_api::{
    TIError, TIResult, TinkoffInvestService,
    tcs::{
        Account, AccountType, Dividend, FindInstrumentRequest, GetAccountsRequest,
        GetDividendsRequest, InstrumentShort, InstrumentStatus, InstrumentType, InstrumentsRequest,
        Operation, OperationState, OperationType, OperationsRequest, PortfolioPosition,
        PortfolioRequest, portfolio_request::CurrencyRequest,
    },
};
use tokio::time::{Duration, sleep};

use crate::{
    domain::{
        DividendCalendar, DividendPayment, History, HistoryItem, Instrument, Money, Paper,
        Position, Profit, Totals,
    },
    to_currency, to_datetime_utc, to_decimal, to_money,
};

#[derive(Default)]
pub struct AccountPortfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

#[derive(Clone)] // <-- добавить
pub struct TinkoffInvestment {
    service: Arc<TinkoffInvestService>, // <-- обернуть в Arc
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

    /// Search intsruments by ticker.
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
        let mut insrument_client = self
            .service
            .instruments(channel)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let instrument = insrument_client
            .find_instrument(FindInstrumentRequest {
                instrument_kind: Some(InstrumentType::Unspecified.into()),
                query: ticker,
                api_trade_available_flag: Some(false),
            })
            .await?;
        let instrument = instrument.get_ref();
        Ok(instrument.instruments.clone())
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
        let mut upcoming = Vec::new();

        let now = chrono::Utc::now();

        for position in positions {
            // Get dividends for each position
            let dividends = self
                .get_dividends_for_figi(position.figi.clone())
                .await
                .unwrap_or_default();

            for dividend in dividends {
                let Some(instrument) = instruments.get(&position.figi) else {
                    continue;
                };

                let dividend_per_share = dividend
                    .dividend_net
                    .as_ref()
                    .and_then(|d| to_money(Some(d)))
                    .unwrap_or_else(|| Money::zero(Currency::RUB));

                // Use record_date as ex-dividend date (ex_dividend_date not available)
                let ex_dividend_date = dividend
                    .record_date
                    .as_ref()
                    .map(|d| to_datetime_utc(Some(d)))
                    .unwrap_or(now);

                let payment_date = dividend
                    .payment_date
                    .as_ref()
                    .map(|d| to_datetime_utc(Some(d)));

                // Get quantity from portfolio position
                let quantity = to_decimal(position.quantity.as_ref());

                // Calculate total dividend = dividend_per_share * quantity
                let total_dividend = dividend_per_share * quantity;

                let dividend_payment = DividendPayment {
                    figi: position.figi.clone(),
                    ticker: instrument.ticker.clone(),
                    name: instrument.name.clone(),
                    currency: dividend_per_share.currency,
                    dividend_per_share,
                    total_dividend,
                    quantity,
                    ex_dividend_date,
                    payment_date,
                    dividend_type: dividend.dividend_type,
                };

                // Only include upcoming payments
                if ex_dividend_date > now {
                    upcoming.push(dividend_payment);
                }
            }
        }

        // Sort by date
        upcoming.sort_by_key(|a| a.ex_dividend_date);

        Ok(DividendCalendar { upcoming })
    }

    /// Get dividends for a specific FIGI
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

        let request = GetDividendsRequest {
            instrument_id: figi,
            from: None,
            to: None,
            ..Default::default()
        };
        let response = instruments
            .get_dividends(request)
            .await
            .map_err(|e| eyre::eyre!("{e:?}"))?;
        let response = response.into_inner();

        Ok(response.dividends)
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
