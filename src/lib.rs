use domain::Money;
use rust_decimal::{prelude::FromPrimitive, Decimal};
use tinkoff_invest_api::tcs::{MoneyValue, Quotation};

pub mod client;
pub mod domain;
pub mod progress;
pub mod ux;

pub fn to_decimal(val: Option<&Quotation>) -> Decimal {
    if let Some(x) = val {
        let s = if x.units == 0 && x.nano < 0 {
            format!("-{}.{}", x.units, x.nano.abs())
        } else {
            format!("{}.{}", x.units, x.nano.abs())
        };
        Decimal::from_str_exact(&s).unwrap()
    } else {
        Decimal::from_i64(0).unwrap()
    }
}

pub fn to_money(val: Option<&MoneyValue>) -> Option<Money> {
    if let Some(x) = val {
        let s = if x.units == 0 && x.nano < 0 {
            format!("-{}.{}", x.units, x.nano.abs())
        } else {
            format!("{}.{}", x.units, x.nano.abs())
        };
        let value = Decimal::from_str_exact(&s).unwrap();
        Money::new(value, x.currency.clone())
    } else {
        None
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
