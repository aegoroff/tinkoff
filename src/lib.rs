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

/// Converts `Option<&Quotation>` into `Decimal`
/// if None passed - zero `Decimal` will be retured
#[must_use]
pub fn to_decimal(val: Option<&Quotation>) -> Decimal {
    if let Some(x) = val {
        let s = if x.units == 0 && x.nano < 0 {
            format!("-{}.{}", x.units, x.nano.abs())
        } else {
            format!("{}.{}", x.units, x.nano.abs())
        };
        if let Ok(d) = Decimal::from_str_exact(&s) {
            d
        } else {
            Decimal::default()
        }
    } else {
        Decimal::default()
    }
}

/// `Option<&MoneyValue>` to `Option<Money>`
#[must_use]
pub fn to_money(val: Option<&MoneyValue>) -> Option<Money> {
    if let Some(x) = val {
        let s = if x.units == 0 && x.nano < 0 {
            format!("-{}.{}", x.units, x.nano.abs())
        } else {
            format!("{}.{}", x.units, x.nano.abs())
        };
        let value = Decimal::from_str_exact(&s).ok()?;
        Money::new(value, &x.currency)
    } else {
        None
    }
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
