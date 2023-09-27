use std::collections::HashMap;

use tinkoff_invest_api::{
    tcs::{
        portfolio_request::CurrencyRequest, AccountType, Bond, Currency, Etf, GetAccountsRequest,
        InstrumentStatus, InstrumentsRequest, Operation, OperationState, OperationType,
        OperationsRequest, PortfolioPosition, PortfolioRequest, Share,
    },
    TIResult, TinkoffInvestService,
};

use crate::domain::{Money, Totals};

#[derive(Default)]
pub struct Portfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

pub struct TinkoffInvestment {
    service: TinkoffInvestService,
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

#[must_use]
pub fn reduce(operations: &[Operation], currency: iso_currency::Currency) -> Totals {
    let mut fees = Money::zero(currency);
    let mut dividents = Money::zero(currency);
    for op in operations {
        let Some(payment) = crate::to_money(op.payment.as_ref()) else {
            continue;
        };
        match to_influence(op.operation_type()) {
            OperationInfluence::PureIncome => {
                dividents += payment;
            }
            OperationInfluence::Fees => {
                fees += payment;
            }
            OperationInfluence::Unspecified => {}
        }
    }
    Totals { dividents, fees }
}

macro_rules! loop_until_success {
    ($e:expr) => {{
        loop {
            match $e {
                Ok(x) => break x,
                Err(_) => continue,
            }
        }
    }};
}

macro_rules! collect {
    ($response:ident, $t:ty) => {{
        $response
            .into_inner()
            .instruments
            .into_iter()
            .map(|x| (x.figi.clone(), x))
            .collect::<HashMap<String, $t>>()
    }};
}

macro_rules! impl_get_until_done {
    ($(($wrapped:ident, $type:ty, $method:ident)),*) => {
        $(
            pub async fn $method(&self) -> HashMap<String, $type> {
                loop_until_success!(self.$wrapped().await)
            }
        )*
    };
}

macro_rules! impl_get_instrument_method {
    ($(($name:ident, $type:ty, $method:ident)),*) => {
        $(
            async fn $name(&self) -> TIResult<HashMap<String, $type>> {
                let channel = self.service.create_channel().await?;
                let mut instruments = self.service.instruments(channel).await?;
                let instruments = instruments
                    .$method(InstrumentsRequest {
                        instrument_status: InstrumentStatus::All as i32,
                    })
                    .await?;
                let instruments = collect!(instruments, $type);
                Ok(instruments)
            }
        )*
    };
}

impl TinkoffInvestment {
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            service: TinkoffInvestService::new(token),
        }
    }
    impl_get_instrument_method!(
        (get_all_bonds, Bond, bonds),
        (get_all_shares, Share, shares),
        (get_all_etfs, Etf, etfs),
        (get_all_currencies, Currency, currencies)
    );

    impl_get_until_done!(
        (get_all_bonds, Bond, get_all_bonds_until_done),
        (get_all_shares, Share, get_all_shares_until_done),
        (get_all_etfs, Etf, get_all_etfs_until_done),
        (get_all_currencies, Currency, get_all_currencies_until_done)
    );

    async fn get_portfolio(&self, account: AccountType) -> TIResult<Portfolio> {
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
            return Ok(Portfolio::default());
        };

        let portfolio = operations
            .get_portfolio(PortfolioRequest {
                account_id: account.id.clone(),
                currency: CurrencyRequest::Rub as i32,
            })
            .await?;
        Ok(Portfolio {
            account_id: account.id.clone(),
            positions: portfolio.into_inner().positions,
        })
    }

    pub async fn get_portfolio_until_done(&self, account: AccountType) -> Portfolio {
        loop_until_success!(self.get_portfolio(account).await)
    }

    async fn get_operations(&self, account_id: String, figi: String) -> TIResult<Vec<Operation>> {
        let channel = self.service.create_channel().await?;
        let mut operations = self.service.operations(channel).await?;
        let operations = operations
            .get_operations(OperationsRequest {
                account_id,
                from: None,
                to: None,
                state: OperationState::Executed as i32,
                figi,
            })
            .await?;

        Ok(operations.into_inner().operations)
    }

    pub async fn get_operations_until_done(
        &self,
        account_id: String,
        figi: String,
    ) -> Vec<Operation> {
        loop_until_success!(self.get_operations(account_id.clone(), figi.clone()).await)
    }
}
