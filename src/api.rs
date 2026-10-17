use std::collections::HashMap;
use std::future::Future;

use color_eyre::eyre::Result;
use iso_currency::Currency;
use t_invest_sdk::api::{
    Account, Dividend, IndicativeResponse, InstrumentShort, OperationItem, OperationType,
};

use crate::domain::{
    AccountPosition, Instrument, InstrumentKind,
    analytics::{Forecast, Fundamentals},
    bond::BondEvent,
    fx::{DailyCloses, FxCandidate},
};

/// Which executed operations of an account to get.
#[derive(Clone, Copy, Debug)]
pub enum OperationsFilter<'a> {
    /// All operations with the instrument of this FIGI.
    Instrument(&'a str),
    /// Operations of these types with any instrument.
    Types(&'a [OperationType]),
}

/// Data the reports need from the T-Invest API.
///
/// Implementations retry transient failures themselves, so returned errors are final.
pub trait InvestApi: DailyCloses + Clone + Send + Sync + 'static {
    /// All accounts of the user, whatever their status.
    fn accounts(&self) -> impl Future<Output = Result<Vec<Account>>> + Send;

    /// Positions of the account with prices in RUB.
    fn portfolio_positions(
        &self,
        account_id: &str,
    ) -> impl Future<Output = Result<Vec<AccountPosition>>> + Send;

    /// Instrument by FIGI; `kind` selects the request that gives the most details of it.
    fn instrument(
        &self,
        figi: &str,
        kind: Option<InstrumentKind>,
    ) -> impl Future<Output = Result<Instrument>> + Send;

    /// Instruments whose ticker or name matches `query`.
    fn find_instruments(
        &self,
        query: &str,
    ) -> impl Future<Output = Result<Vec<InstrumentShort>>> + Send;

    /// All executed operations of the account matching `filter`.
    fn operations(
        &self,
        account_id: &str,
        filter: OperationsFilter<'_>,
    ) -> impl Future<Output = Result<Vec<OperationItem>>> + Send;

    /// Consensus forecast of an instrument; `None` when investment houses give none.
    /// `currency` is used when the API does not specify the forecast currency.
    fn forecast(
        &self,
        instrument_uid: &str,
        currency: Option<Currency>,
    ) -> impl Future<Output = Result<Option<Forecast>>> + Send;

    /// Fundamentals by asset UID; assets without them are absent from the result.
    fn fundamentals(
        &self,
        asset_uids: &[String],
    ) -> impl Future<Output = Result<HashMap<String, Fundamentals>>> + Send;

    /// All dividends of a share or an ETF, past and declared.
    fn dividends(&self, figi: &str) -> impl Future<Output = Result<Vec<Dividend>>> + Send;

    /// Bond events from today on; redemptions but the last one are marked as amortization.
    fn bond_events(&self, figi: &str) -> impl Future<Output = Result<Vec<BondEvent>>> + Send;

    /// Currency instruments RUB rates can be quoted by.
    fn fx_candidates(&self) -> impl Future<Output = Result<Vec<FxCandidate>>> + Send;

    /// Market indices with their tickers and UIDs.
    fn indices(&self) -> impl Future<Output = Result<Vec<IndicativeResponse>>> + Send;
}
