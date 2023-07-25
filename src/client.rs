use std::collections::HashMap;

use tinkoff_invest_api::{
    tcs::{
        portfolio_request::CurrencyRequest, AccountType, Bond, Currency, Etf, GetAccountsRequest,
        InstrumentStatus, InstrumentsRequest, Operation, OperationState, OperationType,
        OperationsRequest, PortfolioPosition, PortfolioRequest, Share,
    },
    TIResult, TinkoffInvestService,
};

#[derive(Default)]
pub struct Portfolio {
    pub account_id: String,
    pub positions: Vec<PortfolioPosition>,
}

pub struct TinkoffClient {
    service: TinkoffInvestService,
}

pub enum OperationInfluence {
    /// Anything that affects to dividents or coupons value.<br/>
    /// Including negative values like divident tax etc. to calculate pure income<br/>
    /// without taxes.
    PureIncome,
    /// Comissions and other losses
    Fees,
    Unspecified,
}

pub fn to_influence(op: OperationType) -> OperationInfluence {
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

macro_rules! collect {
    ($response:ident, $t:tt) => {{
        $response
            .into_inner()
            .instruments
            .into_iter()
            .map(|x| (x.figi.clone(), x))
            .collect::<HashMap<String, $t>>()
    }};
}

impl TinkoffClient {
    pub fn new(token: String) -> Self {
        Self {
            service: TinkoffInvestService::new(token),
        }
    }

    pub async fn get_all_bonds(&self) -> TIResult<HashMap<String, Bond>> {
        let channel = self.service.create_channel().await?;
        let mut bonds = self.service.instruments(channel).await?;
        let bonds = bonds
            .bonds(InstrumentsRequest {
                instrument_status: InstrumentStatus::All as i32,
            })
            .await?;
        let bonds = collect!(bonds, Bond);
        Ok(bonds)
    }

    pub async fn get_all_shares(&self) -> TIResult<HashMap<String, Share>> {
        let channel = self.service.create_channel().await?;
        let mut shares = self.service.instruments(channel).await?;
        let shares = shares
            .shares(InstrumentsRequest {
                instrument_status: InstrumentStatus::All as i32,
            })
            .await?;
        let shares = collect!(shares, Share);
        Ok(shares)
    }

    pub async fn get_all_etfs(&self) -> TIResult<HashMap<String, Etf>> {
        let channel = self.service.create_channel().await?;
        let mut etfs = self.service.instruments(channel).await?;
        let etfs = etfs
            .etfs(InstrumentsRequest {
                instrument_status: InstrumentStatus::All as i32,
            })
            .await?;
        let etfs = collect!(etfs, Etf);
        Ok(etfs)
    }

    pub async fn get_all_currencies(&self) -> TIResult<HashMap<String, Currency>> {
        let channel = self.service.create_channel().await?;
        let mut currencies = self.service.instruments(channel).await?;
        let currencies = currencies
            .currencies(InstrumentsRequest {
                instrument_status: InstrumentStatus::All as i32,
            })
            .await?;
        let currencies = collect!(currencies, Currency);
        Ok(currencies)
    }

    pub async fn get_portfolio(&self, account: AccountType) -> TIResult<Portfolio> {
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
            .find(|a| a.r#type() == account) else { return Ok(Portfolio::default()); };

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

    pub async fn get_operations(
        &self,
        account_id: String,
        figi: String,
    ) -> TIResult<Vec<Operation>> {
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
}
