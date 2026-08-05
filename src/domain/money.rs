use std::{
    fmt::Display,
    ops::{self, AddAssign, DivAssign, MulAssign, SubAssign},
};

use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ux::format_decimal;

use super::NumberRange;

const HUNDRED: Decimal = dec!(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Money {
    pub value: Decimal,
    pub currency: Currency,
}

#[derive(Clone, Copy)]
pub struct Income {
    currency: Currency,
    pub(crate) current: Decimal,
    pub(crate) balance: Decimal,
}

impl Money {
    #[must_use]
    pub fn new(value: Decimal, currency: &str) -> Option<Self> {
        Currency::from_code(&currency.to_ascii_uppercase()).map(|currency| Self { value, currency })
    }

    #[must_use]
    pub fn from_value(value: Decimal, currency: Currency) -> Self {
        Self { value, currency }
    }

    #[must_use]
    pub fn zero(currency: Currency) -> Self {
        Self {
            value: Decimal::default(),
            currency,
        }
    }

    /// Converts this amount to `target` using `rate` (units of target per 1 unit of self).
    #[must_use]
    pub fn to_currency(self, rate: Decimal, target: Currency) -> Self {
        Self {
            value: self.value * rate,
            currency: target,
        }
    }
}

fn assert_same_currency(a: Currency, b: Currency) {
    assert_eq!(
        a, b,
        "currency mismatch: {a:?} vs {b:?}; convert to a common currency before combining"
    );
}

impl Income {
    #[must_use]
    pub fn new(current: Money, balance: Money) -> Self {
        assert_same_currency(current.currency, balance.currency);
        Self {
            currency: current.currency,
            current: current.value,
            balance: balance.value,
        }
    }

    #[must_use]
    pub fn zero(currency: Currency) -> Self {
        Self {
            currency,
            current: Decimal::default(),
            balance: Decimal::default(),
        }
    }

    #[must_use]
    pub fn percent(&self) -> Decimal {
        let income = self.income();
        if self.balance.is_zero() {
            Decimal::default()
        } else {
            (income / self.balance) * HUNDRED
        }
    }

    fn income(&self) -> Decimal {
        self.current - self.balance
    }
}

impl ops::Add<Money> for Money {
    type Output = Money;

    fn add(self, rhs: Money) -> Money {
        assert_same_currency(self.currency, rhs.currency);
        Money {
            value: self.value + rhs.value,
            currency: self.currency,
        }
    }
}

impl ops::Add<Decimal> for Money {
    type Output = Money;

    fn add(self, rhs: Decimal) -> Money {
        Money {
            value: self.value + rhs,
            currency: self.currency,
        }
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, other: Self) {
        assert_same_currency(self.currency, other.currency);
        self.value += other.value;
    }
}

impl AddAssign<Decimal> for Money {
    fn add_assign(&mut self, other: Decimal) {
        self.value += other;
    }
}

impl ops::Sub<Money> for Money {
    type Output = Money;

    fn sub(self, rhs: Money) -> Money {
        assert_same_currency(self.currency, rhs.currency);
        Money {
            value: self.value - rhs.value,
            currency: self.currency,
        }
    }
}

impl ops::Sub<Decimal> for Money {
    type Output = Money;

    fn sub(self, rhs: Decimal) -> Money {
        Money {
            value: self.value - rhs,
            currency: self.currency,
        }
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, other: Self) {
        assert_same_currency(self.currency, other.currency);
        self.value -= other.value;
    }
}

impl SubAssign<Decimal> for Money {
    fn sub_assign(&mut self, other: Decimal) {
        self.value -= other;
    }
}

impl ops::Mul<Money> for Money {
    type Output = Money;

    fn mul(self, rhs: Money) -> Money {
        assert_same_currency(self.currency, rhs.currency);
        Money {
            value: self.value * rhs.value,
            currency: self.currency,
        }
    }
}

impl ops::Mul<Decimal> for Money {
    type Output = Money;

    fn mul(self, rhs: Decimal) -> Money {
        Money {
            value: self.value * rhs,
            currency: self.currency,
        }
    }
}

impl MulAssign for Money {
    fn mul_assign(&mut self, other: Self) {
        assert_same_currency(self.currency, other.currency);
        self.value *= other.value;
    }
}

impl MulAssign<Decimal> for Money {
    fn mul_assign(&mut self, other: Decimal) {
        self.value *= other;
    }
}

impl ops::Div<Money> for Money {
    type Output = Money;

    fn div(self, rhs: Money) -> Money {
        assert_same_currency(self.currency, rhs.currency);
        Money {
            value: self.value / rhs.value,
            currency: self.currency,
        }
    }
}

impl ops::Div<Decimal> for Money {
    type Output = Money;

    fn div(self, rhs: Decimal) -> Money {
        Money {
            value: self.value / rhs,
            currency: self.currency,
        }
    }
}

impl DivAssign for Money {
    fn div_assign(&mut self, other: Self) {
        assert_same_currency(self.currency, other.currency);
        self.value /= other.value;
    }
}

impl DivAssign<Decimal> for Money {
    fn div_assign(&mut self, other: Decimal) {
        self.value /= other;
    }
}

impl ops::Add<Income> for Income {
    type Output = Income;

    fn add(self, rhs: Income) -> Income {
        assert_same_currency(self.currency, rhs.currency);
        Income {
            current: self.current + rhs.current,
            balance: self.balance + rhs.balance,
            currency: self.currency,
        }
    }
}

impl AddAssign for Income {
    fn add_assign(&mut self, other: Self) {
        assert_same_currency(self.currency, other.currency);
        self.current += other.current;
        self.balance += other.balance;
    }
}

impl Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}",
            format_decimal(self.value)?,
            self.currency.symbol()
        )
    }
}

impl NumberRange for Money {
    fn is_negative(&self) -> bool {
        self.value.is_sign_negative()
    }

    fn is_zero(&self) -> bool {
        self.value.is_zero()
    }
}

impl Display for Income {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} ({}%)",
            format_decimal(self.income())?,
            self.currency.symbol(),
            self.percent().round_dp(2)
        )
    }
}

impl NumberRange for Income {
    fn is_negative(&self) -> bool {
        self.income().is_sign_negative()
    }

    fn is_zero(&self) -> bool {
        self.income().is_zero()
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn money_add_same_currency_ok() {
        let m1 = Money::from_value(dec!(100), Currency::RUB);
        let m2 = Money::from_value(dec!(50), Currency::RUB);
        let result = m1 + m2;
        assert_eq!(result.value, dec!(150));
        assert_eq!(result.currency, Currency::RUB);
    }

    #[test]
    #[should_panic(expected = "currency mismatch")]
    fn money_add_different_currency_panics() {
        let m1 = Money::from_value(dec!(100), Currency::RUB);
        let m2 = Money::from_value(dec!(50), Currency::USD);
        let _ = m1 + m2;
    }

    #[test]
    fn money_to_currency_applies_rate() {
        let usd = Money::from_value(dec!(10), Currency::USD);
        let rub = usd.to_currency(dec!(90), Currency::RUB);
        assert_eq!(rub.value, dec!(900));
        assert_eq!(rub.currency, Currency::RUB);
    }

    #[test]
    fn income_percent_zero_balance() {
        let income = Income::new(
            Money::from_value(dec!(100), Currency::RUB),
            Money::from_value(dec!(0), Currency::RUB),
        );
        assert!(income.percent().is_zero());
    }
}
