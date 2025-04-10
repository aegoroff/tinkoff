use chrono::{DateTime, Utc};
use domain::Money;
use iso_currency::Currency;
use prost_types::Timestamp;
use rust_decimal::Decimal;
use tinkoff_invest_api::tcs::{MoneyValue, Quotation};

pub mod client;
pub mod domain;
pub mod progress;
pub mod ux;

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
/// use tinkoff_invest_api::tcs::Quotation;
///
/// let q = Quotation { units: 1, nano: 1 };
/// let decimal = to_decimal(Some(&q));
/// assert_eq!(decimal.to_string(), "1.1");
///
/// let none_decimal = to_decimal(None);
/// assert!(none_decimal.is_zero());
/// ```
#[must_use]
pub fn to_decimal(val: Option<&Quotation>) -> Decimal {
    if let Some(x) = val {
        let s = if x.units == 0 && x.nano < 0 {
            format!("-{}.{}", x.units, x.nano.abs())
        } else {
            format!("{}.{}", x.units, x.nano.abs())
        };
        Decimal::from_str_exact(&s).unwrap_or_default()
    } else {
        Decimal::default()
    }
}

/// `Option<&MoneyValue>` to `Option<Money>`
#[must_use]
pub fn to_money(val: Option<&MoneyValue>) -> Option<Money> {
    let val = val?;
    let s = if val.units == 0 && val.nano < 0 {
        format!("-{}.{}", val.units, val.nano.abs())
    } else {
        format!("{}.{}", val.units, val.nano.abs())
    };
    let value = Decimal::from_str_exact(&s).ok()?;
    Money::new(value, &val.currency)
}

#[must_use]
pub fn to_currency(mv: &Option<MoneyValue>) -> Option<Currency> {
    iso_currency::Currency::from_code(&mv.as_ref()?.currency.to_ascii_uppercase())
}

#[must_use]
pub fn to_datetime_utc(opt_timespamp: Option<&Timestamp>) -> DateTime<Utc> {
    if let Some(dt) = opt_timespamp {
        DateTime::<Utc>::from_timestamp(dt.seconds, 0).unwrap_or_default()
    } else {
        DateTime::<Utc>::default()
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;

    use super::*;

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
        let q = Quotation { units: 1, nano: 1 };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("1.1"));
    }

    #[test]
    fn to_decimal_positive_above_zero() {
        // Arrange
        let q = Quotation { units: 0, nano: 1 };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("0.1"));
    }

    #[test]
    fn to_decimal_negative_below_minus_one() {
        // Arrange
        let q = Quotation {
            units: -1,
            nano: -1,
        };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("-1.1"));
    }

    #[test]
    fn to_decimal_negative_above_minus_one() {
        // Arrange
        let q = Quotation { units: 0, nano: -1 };

        // Act
        let r = to_decimal(Some(&q));

        // Assert
        assert_eq!(r.to_string(), String::from("-0.1"));
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
            nano: 1,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        let m = r.unwrap();
        assert_eq!(m.value.to_string(), String::from("1.1"));
        assert_eq!(m.currency, Currency::RUB);
    }

    #[test]
    fn to_money_positive_above_zero() {
        // Arrange
        let q = MoneyValue {
            units: 0,
            nano: 1,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("0.1"));
    }

    #[test]
    fn to_money_negative_below_minus_one() {
        // Arrange
        let q = MoneyValue {
            units: -1,
            nano: -1,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("-1.1"));
    }

    #[test]
    fn to_money_negative_above_minus_one() {
        // Arrange
        let q = MoneyValue {
            units: 0,
            nano: -1,
            currency: "rub".to_string(),
        };

        // Act
        let r = to_money(Some(&q));

        // Assert
        assert_eq!(r.unwrap().value.to_string(), String::from("-0.1"));
    }
}
