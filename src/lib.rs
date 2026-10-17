use chrono::{DateTime, Utc};
use color_eyre::eyre;
use domain::{
    AccountPosition, FoundInstrument, InstrumentKind, Money, Position,
    account::{Account, AccountStatus, AccountType},
    benchmark::MarketIndex,
    calendar::Dividend,
    operation::{Operation, OperationKind, OperationState},
};
use iso_currency::Currency;
use prost_types::Timestamp;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use t_invest_sdk::api::{
    Account as ApiAccount, AccountStatus as ApiAccountStatus, AccountType as ApiAccountType,
    Dividend as ApiDividend, IndicativeResponse, InstrumentShort, MoneyValue, OperationItem,
    OperationState as ApiOperationState, OperationType, PortfolioPosition, Quotation,
};

pub mod api;
pub mod client;
pub mod domain;
pub mod progress;
pub mod reports;
pub mod ux;

pub use domain::risk;

/// Converts an `Option<&Quotation>` to `Decimal`.
///
/// # Arguments
///
/// * `val` - An optional reference to a `Quotation` which contains financial data with units and nano fields.
///
/// # Returns
///
/// * `Decimal` - A decimal representation of the financial data. If the input is `None` or the conversion fails, a default `Decimal` value is returned.
///
/// # Examples
///
/// ```
/// use tinkoff::to_decimal;
/// use rust_decimal::Decimal;
/// use t_invest_sdk::api::Quotation;
///
/// let q = Quotation { units: 1, nano: 100_000_000 };
/// let decimal = to_decimal(Some(&q));
/// assert_eq!(decimal.to_string(), "1.10");
///
/// let none_decimal = to_decimal(None);
/// assert!(none_decimal.is_zero());
/// ```
#[must_use]
pub fn to_decimal(val: Option<&Quotation>) -> Decimal {
    let Some(x) = val else {
        return Decimal::default();
    };
    // Simply add units and nano. The Tinkoff API returns consistent signs for both fields.
    // For example:
    // - units=1, nano=100_000_000  => 1.10
    // - units=-1, nano=-100_000_000 => -1.10
    // - units=0, nano=-100_000_000 => -0.10
    Decimal::from(x.units) + Decimal::from(x.nano) / dec!(1_000_000_000)
}

/// `Option<&MoneyValue>` to `Option<Money>`
#[must_use]
pub fn to_money(val: Option<&MoneyValue>) -> Option<Money> {
    let val = val?;
    let value = Decimal::from(val.units) + Decimal::from(val.nano) / dec!(1_000_000_000);
    Money::new(value, &val.currency)
}

#[must_use]
pub fn to_currency(mv: &Option<MoneyValue>) -> Option<Currency> {
    iso_currency::Currency::from_code(&mv.as_ref()?.currency.to_ascii_uppercase())
}

/// Converts an API metric to a decimal rounded to 2 places; `None` for zero,
/// which the API returns for unknown metrics, and for non-finite values.
#[must_use]
pub fn to_metric(value: f64) -> Option<Decimal> {
    Decimal::try_from(value)
        .ok()
        .filter(|d| !d.is_zero())
        .map(|d| d.round_dp(2))
}

/// Converts an API timestamp to UTC date time; `None` when it is absent or unset (zero).
#[must_use]
pub fn to_optional_datetime_utc(timestamp: Option<&Timestamp>) -> Option<DateTime<Utc>> {
    timestamp
        .filter(|t| t.seconds != 0)
        .and_then(|t| DateTime::<Utc>::from_timestamp(t.seconds, 0))
}

#[must_use]
pub fn to_datetime_utc(opt_timespamp: Option<&Timestamp>) -> DateTime<Utc> {
    if let Some(dt) = opt_timespamp {
        DateTime::<Utc>::from_timestamp(dt.seconds, 0).unwrap_or_default()
    } else {
        DateTime::<Utc>::default()
    }
}

impl From<ApiAccountType> for AccountType {
    fn from(account_type: ApiAccountType) -> Self {
        match account_type {
            ApiAccountType::Tinkoff => Self::Tinkoff,
            ApiAccountType::TinkoffIis => Self::TinkoffIis,
            ApiAccountType::InvestBox => Self::InvestBox,
            ApiAccountType::InvestFund => Self::InvestFund,
            ApiAccountType::Debit => Self::Debit,
            ApiAccountType::Saving => Self::Saving,
            ApiAccountType::Dfa => Self::Dfa,
            ApiAccountType::Unspecified => Self::Unspecified,
        }
    }
}

impl From<ApiAccountStatus> for AccountStatus {
    fn from(status: ApiAccountStatus) -> Self {
        match status {
            ApiAccountStatus::New => Self::New,
            ApiAccountStatus::Open => Self::Open,
            ApiAccountStatus::Closed => Self::Closed,
            ApiAccountStatus::All | ApiAccountStatus::Unspecified => Self::Unspecified,
        }
    }
}

impl From<ApiAccount> for Account {
    fn from(account: ApiAccount) -> Self {
        Self {
            account_type: account.r#type().into(),
            status: account.status().into(),
            opened: to_optional_datetime_utc(account.opened_date.as_ref()),
            id: account.id,
            name: account.name,
        }
    }
}

impl From<ApiDividend> for Dividend {
    fn from(dividend: ApiDividend) -> Self {
        Self {
            per_share: to_money(dividend.dividend_net.as_ref())
                .unwrap_or_else(|| Money::zero(Currency::RUB)),
            payment_date: to_optional_datetime_utc(dividend.payment_date.as_ref()),
            record_date: to_optional_datetime_utc(dividend.record_date.as_ref()),
            dividend_type: dividend.dividend_type,
        }
    }
}

impl From<InstrumentShort> for FoundInstrument {
    fn from(instrument: InstrumentShort) -> Self {
        Self {
            figi: instrument.figi,
            ticker: instrument.ticker,
            name: instrument.name,
        }
    }
}

impl From<IndicativeResponse> for MarketIndex {
    fn from(index: IndicativeResponse) -> Self {
        Self {
            ticker: index.ticker,
            uid: index.uid,
        }
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
            var_margin: None,
            blocked: value.blocked,
            blocked_lots: to_decimal(value.blocked_lots.as_ref()),
        })
    }
}

/// Position as the portfolio API reports it. The portfolio is requested in RUB, so prices
/// that are missing or in another currency make [`AccountPosition::position`] an error
/// instead of mixing currencies in totals.
impl From<PortfolioPosition> for AccountPosition {
    fn from(value: PortfolioPosition) -> Self {
        let position = Position::try_from(&value)
            .map(|p| futures_margin(p, &value))
            .and_then(|p| {
                ensure_rub(&p)?;
                Ok(p)
            })
            .map_err(|e| format!("{e:#}"));
        Self {
            figi: value.figi,
            instrument_uid: value.instrument_uid,
            instrument_type: value.instrument_type,
            position,
        }
    }
}

/// Kinds of API operation types; types absent here are [`OperationKind::Other`].
pub(crate) const OPERATION_KINDS: [(OperationType, OperationKind); 38] = [
    (OperationType::Buy, OperationKind::Buy),
    (OperationType::BuyCard, OperationKind::Buy),
    (OperationType::BuyMargin, OperationKind::Buy),
    (OperationType::Sell, OperationKind::Sell),
    (OperationType::SellCard, OperationKind::Sell),
    (OperationType::SellMargin, OperationKind::Sell),
    (OperationType::Dividend, OperationKind::Dividend),
    (OperationType::DivExt, OperationKind::Dividend),
    (OperationType::Coupon, OperationKind::Coupon),
    (OperationType::DividendTax, OperationKind::DividendTax),
    (
        OperationType::DividendTaxProgressive,
        OperationKind::DividendTax,
    ),
    (OperationType::BondTax, OperationKind::CouponTax),
    (OperationType::BondTaxProgressive, OperationKind::CouponTax),
    (OperationType::TaxCorrectionCoupon, OperationKind::CouponTax),
    (OperationType::Tax, OperationKind::OtherTax),
    (OperationType::TaxProgressive, OperationKind::OtherTax),
    (OperationType::TaxCorrection, OperationKind::OtherTax),
    (
        OperationType::TaxCorrectionProgressive,
        OperationKind::OtherTax,
    ),
    (OperationType::BenefitTax, OperationKind::OtherTax),
    (
        OperationType::BenefitTaxProgressive,
        OperationKind::OtherTax,
    ),
    (OperationType::TaxRepo, OperationKind::OtherTax),
    (OperationType::TaxRepoProgressive, OperationKind::OtherTax),
    (OperationType::TaxRepoHold, OperationKind::OtherTax),
    (
        OperationType::TaxRepoHoldProgressive,
        OperationKind::OtherTax,
    ),
    (OperationType::TaxRepoRefund, OperationKind::OtherTax),
    (
        OperationType::TaxRepoRefundProgressive,
        OperationKind::OtherTax,
    ),
    (OperationType::Overnight, OperationKind::Overnight),
    (OperationType::ServiceFee, OperationKind::Fee),
    (OperationType::MarginFee, OperationKind::Fee),
    (OperationType::BrokerFee, OperationKind::Fee),
    (OperationType::SuccessFee, OperationKind::Fee),
    (OperationType::TrackMfee, OperationKind::Fee),
    (OperationType::TrackPfee, OperationKind::Fee),
    (OperationType::CashFee, OperationKind::Fee),
    (OperationType::OutFee, OperationKind::Fee),
    (OperationType::OutStampDuty, OperationKind::Fee),
    (OperationType::AdviceFee, OperationKind::Fee),
    (OperationType::OutputPenalty, OperationKind::Fee),
];

/// Kind of an API operation type code; unknown codes are [`OperationKind::Other`].
fn operation_kind(code: i32) -> OperationKind {
    OPERATION_KINDS
        .iter()
        .find(|(api, _)| *api as i32 == code)
        .map_or(OperationKind::Other, |(_, kind)| *kind)
}

impl From<OperationItem> for Operation {
    fn from(op: OperationItem) -> Self {
        let state = match op.state() {
            ApiOperationState::Executed => OperationState::Executed,
            ApiOperationState::Canceled => OperationState::Canceled,
            ApiOperationState::Progress => OperationState::InProgress,
            ApiOperationState::Unspecified => OperationState::Unspecified,
        };
        Self {
            kind: operation_kind(op.r#type),
            state,
            date: to_datetime_utc(op.date.as_ref()),
            instrument_kind: InstrumentKind::from_api(&op.instrument_type),
            payment: to_money(op.payment.as_ref()),
            commission: to_money(op.commission.as_ref()),
            price: to_money(op.price.as_ref()),
            accrued_interest: to_money(op.accrued_int.as_ref()),
            quantity: op.quantity,
            quantity_rest: op.quantity_rest,
            description: op.description,
            id: op.id,
        }
    }
}

/// A futures contract is valued by its variation margin, including when the API omits it.
fn futures_margin(mut position: Position, value: &PortfolioPosition) -> Position {
    if InstrumentKind::from_api(&value.instrument_type) == Some(InstrumentKind::Future) {
        position.var_margin =
            Some(to_money(value.var_margin.as_ref()).unwrap_or_else(|| Money::zero(Currency::RUB)));
    }
    position
}

/// Checks that position money values are in RUB.
fn ensure_rub(position: &Position) -> color_eyre::Result<()> {
    for money in [
        position.average_buy_price,
        position.current_instrument_price,
        position.accrued_interest,
        position.daily_yield,
    ]
    .into_iter()
    .chain(position.var_margin)
    {
        if money.currency != Currency::RUB {
            return Err(eyre::eyre!(
                "Expected position prices in RUB, got {}",
                money.currency.code()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::rstest;

    use super::*;

    fn rub(units: i64) -> MoneyValue {
        MoneyValue {
            currency: "rub".to_string(),
            units,
            nano: 0,
        }
    }

    #[rstest]
    #[case::buy_card(OperationType::BuyCard, OperationKind::Buy)]
    #[case::sell_margin(OperationType::SellMargin, OperationKind::Sell)]
    #[case::dividend_to_card(OperationType::DivExt, OperationKind::Dividend)]
    #[case::coupon_tax_correction(OperationType::TaxCorrectionCoupon, OperationKind::CouponTax)]
    #[case::progressive_tax(OperationType::TaxProgressive, OperationKind::OtherTax)]
    #[case::overnight(OperationType::Overnight, OperationKind::Overnight)]
    #[case::penalty(OperationType::OutputPenalty, OperationKind::Fee)]
    #[case::input(OperationType::Input, OperationKind::Other)]
    fn operation_kind_of_api_type(#[case] api: OperationType, #[case] expected: OperationKind) {
        // Act
        let kind = operation_kind(api as i32);

        // Assert
        assert_eq!(kind, expected);
    }

    #[test]
    fn operation_kind_of_unknown_code_is_other() {
        // Act
        let kind = operation_kind(-1);

        // Assert
        assert_eq!(kind, OperationKind::Other);
    }

    #[test]
    fn operation_keeps_api_values() {
        // Arrange
        let item = OperationItem {
            id: "OP1".to_string(),
            r#type: OperationType::Buy as i32,
            state: ApiOperationState::Executed as i32,
            date: Some(Timestamp {
                seconds: 1_758_790_800,
                nanos: 0,
            }),
            instrument_type: "bond".to_string(),
            payment: Some(rub(-1020)),
            commission: Some(rub(-15)),
            price: Some(rub(1000)),
            accrued_int: Some(rub(20)),
            quantity: 1,
            description: "Bond purchase".to_string(),
            ..Default::default()
        };

        // Act
        let op = Operation::from(item);

        // Assert
        assert_eq!(op.id, "OP1");
        assert_eq!(op.kind, OperationKind::Buy);
        assert_eq!(op.state, OperationState::Executed);
        assert_eq!(op.date.timestamp(), 1_758_790_800);
        assert_eq!(op.instrument_kind, Some(InstrumentKind::Bond));
        assert_eq!(op.payment.map(|p| p.value), Some(dec!(-1020)));
        assert_eq!(op.commission.map(|p| p.value), Some(dec!(-15)));
        assert_eq!(op.price.map(|p| p.value), Some(dec!(1000)));
        assert_eq!(op.accrued_interest.map(|p| p.value), Some(dec!(20)));
        assert_eq!(op.quantity, 1);
        assert_eq!(op.description, "Bond purchase");
    }

    #[test]
    fn operation_without_known_currency_has_no_payment() {
        // Arrange
        let item = OperationItem {
            payment: Some(MoneyValue {
                currency: "xyz".to_string(),
                units: 1,
                nano: 0,
            }),
            ..Default::default()
        };

        // Act
        let op = Operation::from(item);

        // Assert
        assert!(op.payment.is_none());
        assert_eq!(op.instrument_kind, None);
    }

    fn api_position(price: MoneyValue) -> PortfolioPosition {
        PortfolioPosition {
            figi: "FIGI1".to_string(),
            instrument_uid: "UID1".to_string(),
            instrument_type: "share".to_string(),
            quantity: Some(Quotation { units: 2, nano: 0 }),
            average_position_price: Some(price.clone()),
            current_price: Some(price),
            ..Default::default()
        }
    }

    #[test]
    fn account_position_keeps_rub_prices() {
        // Arrange
        let position = api_position(rub(990));

        // Act
        let position = AccountPosition::from(position);

        // Assert
        assert_eq!(position.figi, "FIGI1");
        assert_eq!(position.instrument_uid, "UID1");
        assert_eq!(position.instrument_type, "share");
        let prices = position.position.unwrap();
        assert_eq!(prices.current_instrument_price.value, dec!(990));
        assert_eq!(prices.quantity, dec!(2));
    }

    #[test]
    fn account_position_rejects_foreign_prices() {
        // Arrange
        let position = api_position(MoneyValue {
            currency: "usd".to_string(),
            units: 11,
            nano: 0,
        });

        // Act
        let position = AccountPosition::from(position);

        // Assert
        assert_eq!(
            position.position.err().as_deref(),
            Some("Expected position prices in RUB, got USD")
        );
    }

    #[test]
    fn account_position_rejects_missing_price() {
        // Arrange
        let position = PortfolioPosition {
            current_price: None,
            ..api_position(rub(990))
        };

        // Act
        let position = AccountPosition::from(position);

        // Assert
        assert_eq!(
            position.position.err().as_deref(),
            Some("Failed to get currency")
        );
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

    #[rstest]
    #[case::margin("futures", Some(1500), 1, 1500, 93250)]
    #[case::no_margin("futures", None, 1, 0, 93250)]
    #[case::short("futures", Some(1500), -1, 1500, -93250)]
    #[case::share_ignores_margin("share", Some(1500), 2, 186_500, 186_500)]
    fn futures_position_is_valued_by_variation_margin(
        #[case] instrument_type: &str,
        #[case] margin: Option<i64>,
        #[case] quantity: i64,
        #[case] value: i64,
        #[case] notional: i64,
    ) {
        // Arrange
        let position = PortfolioPosition {
            instrument_type: instrument_type.to_string(),
            quantity: Some(Quotation {
                units: quantity,
                nano: 0,
            }),
            average_position_price: Some(rub(90_000)),
            current_price: Some(rub(93_250)),
            var_margin: margin.map(rub),
            ..Default::default()
        };

        // Act
        let position = AccountPosition::from(position).position.unwrap();

        // Assert
        assert_eq!(position.current().value, Decimal::from(value));
        assert_eq!(position.notional().value, Decimal::from(notional));
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

    #[rstest]
    #[case::open(ApiAccountStatus::Open, AccountStatus::Open)]
    #[case::closed(ApiAccountStatus::Closed, AccountStatus::Closed)]
    #[case::all(ApiAccountStatus::All, AccountStatus::Unspecified)]
    fn account_from_api(#[case] api_status: ApiAccountStatus, #[case] status: AccountStatus) {
        // Arrange
        let account = ApiAccount {
            id: "ACC".to_string(),
            name: "Broker".to_string(),
            r#type: ApiAccountType::TinkoffIis as i32,
            status: api_status as i32,
            opened_date: Some(Timestamp {
                seconds: 1_790_000_000,
                nanos: 0,
            }),
            ..Default::default()
        };

        // Act
        let account = Account::from(account);

        // Assert
        assert_eq!(
            (account.id.as_str(), account.name.as_str()),
            ("ACC", "Broker")
        );
        assert_eq!(account.account_type, AccountType::TinkoffIis);
        assert_eq!(account.status, status);
        assert_eq!(
            account.opened,
            DateTime::<Utc>::from_timestamp(1_790_000_000, 0)
        );
    }

    #[test]
    fn dividend_from_api() {
        // Arrange
        let dividend = ApiDividend {
            dividend_net: Some(MoneyValue {
                currency: "usd".to_string(),
                units: 2,
                nano: 500_000_000,
            }),
            record_date: Some(Timestamp {
                seconds: 1_790_000_000,
                nanos: 0,
            }),
            payment_date: Some(Timestamp {
                seconds: 0,
                nanos: 0,
            }),
            dividend_type: "Regular Cash".to_string(),
            ..Default::default()
        };

        // Act
        let dividend = Dividend::from(dividend);

        // Assert
        assert_eq!(
            dividend.per_share,
            Money::from_value(dec!(2.5), Currency::USD)
        );
        assert_eq!(dividend.payment_date, None);
        assert_eq!(
            dividend.date(),
            DateTime::<Utc>::from_timestamp(1_790_000_000, 0)
        );
        assert_eq!(dividend.dividend_type, "Regular Cash");
    }

    #[rstest]
    #[case::absent(None)]
    #[case::unknown_currency(Some(MoneyValue { currency: "xxx1".to_string(), units: 1, nano: 0 }))]
    fn dividend_from_api_without_amount_is_zero_rub(#[case] dividend_net: Option<MoneyValue>) {
        // Arrange
        let dividend = ApiDividend {
            dividend_net,
            ..Default::default()
        };

        // Act
        let dividend = Dividend::from(dividend);

        // Assert
        assert_eq!(dividend.per_share, Money::zero(Currency::RUB));
        assert_eq!(dividend.date(), None);
    }

    #[rstest::rstest]
    #[case::absent(None, None)]
    #[case::unset(Some(Timestamp { seconds: 0, nanos: 0 }), None)]
    #[case::set(
        Some(Timestamp { seconds: 1_790_000_000, nanos: 0 }),
        DateTime::<Utc>::from_timestamp(1_790_000_000, 0)
    )]
    fn to_optional_datetime_utc_cases(
        #[case] timestamp: Option<Timestamp>,
        #[case] expected: Option<DateTime<Utc>>,
    ) {
        // Arrange

        // Act
        let result = to_optional_datetime_utc(timestamp.as_ref());

        // Assert
        assert_eq!(result, expected);
    }

    #[rstest::rstest]
    #[case::ratio(4.567, Some(dec!(4.57)))]
    #[case::negative(-1.2, Some(dec!(-1.2)))]
    #[case::unset(0.0, None)]
    #[case::not_a_number(f64::NAN, None)]
    #[case::infinite(f64::INFINITY, None)]
    fn to_metric_cases(#[case] value: f64, #[case] expected: Option<Decimal>) {
        // Arrange

        // Act
        let result = to_metric(value);

        // Assert
        assert_eq!(result, expected);
    }

    #[test]
    fn to_decimal_from_none() {
        // Arrange

        // Act
        let r = to_decimal(None);

        // Assert
        assert!(r.is_zero());
    }

    #[test]
    fn to_decimal_positive_above_one() {
        // Arrange
        let q = Quotation {
            units: 1,
            nano: 100_000_000,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("1.10"));
    }

    #[test]
    fn to_decimal_positive_above_zero() {
        // Arrange
        let q = Quotation {
            units: 0,
            nano: 100_000_000,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("0.10"));
    }

    #[test]
    fn to_decimal_two_decimal_places() {
        // Arrange
        let q = Quotation {
            units: 1,
            nano: 10_000_000,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), "1.01");
    }

    #[test]
    fn to_decimal_negative_below_minus_one() {
        // Arrange
        let q = Quotation {
            units: -1,
            nano: -100_000_000,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("-1.10"));
    }

    #[test]
    fn to_decimal_negative_above_minus_one() {
        // Arrange
        let q = Quotation {
            units: 0,
            nano: -100_000_000,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("-0.10"));
    }

    #[test]
    fn to_money_from_none() {
        // Arrange

        // Act
        let r = to_money(None);

        // Assert
        assert!(r.is_none());
    }

    #[test]
    fn to_money_positive_above_one() {
        // Arrange
        let q = MoneyValue {
            units: 1,
            nano: 100_000_000,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        let m = r.unwrap();
        assert_eq!(m.value.to_string(), String::from("1.10"));
        assert_eq!(m.currency, Currency::RUB);
    }

    #[test]
    fn to_money_positive_above_zero() {
        // Arrange
        let q = MoneyValue {
            units: 0,
            nano: 100_000_000,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("0.10"));
    }

    #[test]
    fn to_money_negative_below_minus_one() {
        // Arrange
        let q = MoneyValue {
            units: -1,
            nano: -100_000_000,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("-1.10"));
    }

    #[test]
    fn to_money_negative_above_minus_one() {
        // Arrange
        let q = MoneyValue {
            units: 0,
            nano: -100_000_000,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("-0.10"));
    }
}
