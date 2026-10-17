use chrono::{DateTime, Utc};
use color_eyre::eyre;
use domain::{AccountPosition, Money, Position};
use iso_currency::Currency;
use prost_types::Timestamp;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use t_invest_sdk::api::{AccountStatus, AccountType, MoneyValue, PortfolioPosition, Quotation};

pub mod api;
pub mod client;
pub mod domain;
pub mod progress;
pub mod reports;
pub mod ux;

pub use domain::risk;

const ACCOUNT_TYPE_HELP: &str =
    "tinkoff (broker, default), iis, invest-box, invest-fund, debit, saving, dfa";

/// Parses a CLI account type name into [`AccountType`].
///
/// # Errors
///
/// Returns an error if `value` is not a known account type name.
pub fn parse_account_type(value: &str) -> Result<AccountType, String> {
    if value.eq_ignore_ascii_case("tinkoff") || value.eq_ignore_ascii_case("broker") {
        return Ok(AccountType::Tinkoff);
    }
    if value.eq_ignore_ascii_case("iis") || value.eq_ignore_ascii_case("tinkoff-iis") {
        return Ok(AccountType::TinkoffIis);
    }
    if value.eq_ignore_ascii_case("invest-box") || value.eq_ignore_ascii_case("investbox") {
        return Ok(AccountType::InvestBox);
    }
    if value.eq_ignore_ascii_case("invest-fund") || value.eq_ignore_ascii_case("investfund") {
        return Ok(AccountType::InvestFund);
    }
    if value.eq_ignore_ascii_case("debit") {
        return Ok(AccountType::Debit);
    }
    if value.eq_ignore_ascii_case("saving") {
        return Ok(AccountType::Saving);
    }
    if value.eq_ignore_ascii_case("dfa") {
        return Ok(AccountType::Dfa);
    }

    Err(format!(
        "unknown account type '{value}'; expected one of: {ACCOUNT_TYPE_HELP}"
    ))
}

/// Returns the CLI name of an account type, the inverse of [`parse_account_type`].
#[must_use]
pub fn account_type_name(account_type: AccountType) -> &'static str {
    match account_type {
        AccountType::Tinkoff => "tinkoff",
        AccountType::TinkoffIis => "iis",
        AccountType::InvestBox => "invest-box",
        AccountType::InvestFund => "invest-fund",
        AccountType::Debit => "debit",
        AccountType::Saving => "saving",
        AccountType::Dfa => "dfa",
        AccountType::Unspecified => "unspecified",
    }
}

/// Returns a short lowercase name of an account status.
#[must_use]
pub fn account_status_name(status: AccountStatus) -> &'static str {
    match status {
        AccountStatus::New => "new",
        AccountStatus::Open => "open",
        AccountStatus::Closed => "closed",
        AccountStatus::All => "all",
        AccountStatus::Unspecified => "unspecified",
    }
}

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

/// Position as the portfolio API reports it. The portfolio is requested in RUB, so prices
/// that are missing or in another currency make [`AccountPosition::position`] an error
/// instead of mixing currencies in totals.
impl From<PortfolioPosition> for AccountPosition {
    fn from(value: PortfolioPosition) -> Self {
        let position = Position::try_from(&value)
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

/// Checks that position money values are in RUB.
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

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use t_invest_sdk::api::AccountType;

    use super::*;

    fn rub(units: i64) -> MoneyValue {
        MoneyValue {
            currency: "rub".to_string(),
            units,
            nano: 0,
        }
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

    #[test]
    fn parse_account_type_defaults() {
        assert_eq!(parse_account_type("tinkoff").unwrap(), AccountType::Tinkoff);
        assert_eq!(parse_account_type("iis").unwrap(), AccountType::TinkoffIis);
        assert_eq!(
            parse_account_type("invest-box").unwrap(),
            AccountType::InvestBox
        );
    }

    #[rstest::rstest]
    #[case(AccountType::Tinkoff)]
    #[case(AccountType::TinkoffIis)]
    #[case(AccountType::InvestBox)]
    #[case(AccountType::InvestFund)]
    #[case(AccountType::Debit)]
    #[case(AccountType::Saving)]
    #[case(AccountType::Dfa)]
    fn account_type_name_round_trips(#[case] account_type: AccountType) {
        // Arrange
        let name = account_type_name(account_type);

        // Act
        let parsed = parse_account_type(name);

        // Assert
        assert_eq!(parsed, Ok(account_type));
    }

    #[test]
    fn parse_account_type_unknown() {
        assert!(parse_account_type("savings").is_err());
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
