use iso_currency::Currency;
use rust_decimal::Decimal;
use std::fmt;

use super::money::{Income, Money};

/// Newtype for FIGI (Financial Instrument Global Identifier)
/// Provides type safety and prevents mixing up with other string identifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Figi(pub String);

impl Figi {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Figi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Figi {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Figi {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Figi {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Newtype for ticker symbol
/// Provides type safety and prevents mixing up with other string identifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ticker(pub String);

impl Ticker {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Ticker {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Ticker {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Ticker {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub struct Instrument {
    pub name: String,
    pub ticker: Ticker,
}

#[derive(Clone, Copy)]
pub struct Position {
    pub currency: Currency,
    pub average_buy_price: Money,
    pub current_instrument_price: Money,
    /// Accrued coupon interest (NKD) per unit; zero for non-bond instruments
    pub accrued_interest: Money,
    pub quantity: Decimal,
}

#[derive(Clone)]
pub struct Totals {
    /// Dividends, coupons etc. i.e. some extra value
    /// an asset may earn
    pub additional_profit: Money,
    /// Taxes and fees
    pub fees: Money,
}

/// Represents additional asset profit
/// besides balance value growing due to price increase.
/// Used mainly for output
pub trait Profit: Copy + Clone {
    /// shows whether additional profit
    /// applicable to an asset
    fn applicable() -> bool;
    /// Profit name
    fn name() -> &'static str;
}

#[derive(Clone, Copy)]
pub struct DividendProfit;
#[derive(Clone, Copy)]
pub struct CouponProfit;
#[derive(Clone, Copy)]
pub struct NoneProfit;

/// Paper represents things like share, bond, currency, etf etc.
#[derive(Clone)]
pub struct Paper<P: Profit> {
    pub name: String,
    pub ticker: Ticker,
    pub figi: Figi,
    pub position: Position,
    pub totals: Totals,
    pub profit: P,
}

impl Profit for DividendProfit {
    fn applicable() -> bool {
        true
    }

    fn name() -> &'static str {
        "Dividends"
    }
}

impl Profit for CouponProfit {
    fn applicable() -> bool {
        true
    }

    fn name() -> &'static str {
        "Coupons"
    }
}

impl Profit for NoneProfit {
    fn applicable() -> bool {
        false
    }

    fn name() -> &'static str {
        ""
    }
}

impl<P: Profit> Paper<P> {
    /// Paper income (difference between current and balance prices)
    #[must_use]
    pub fn income(&self) -> Income {
        Income::new(self.current(), self.balance())
    }

    /// Total income (income + dividends)
    #[must_use]
    pub fn total_income(&self) -> Income {
        let div = self.dividends();
        Income::new(self.current() + (div.current - div.balance), self.balance())
    }

    /// Expences (the amount of money thea really spent), i.e. average position price multiplied to quantity
    #[must_use]
    pub fn balance(&self) -> Money {
        self.position.average_buy_price * self.position.quantity
    }

    /// Current position value, i.e. current position price plus accrued interest multiplied to quantity
    #[must_use]
    pub fn current(&self) -> Money {
        (self.position.current_instrument_price + self.position.accrued_interest)
            * self.position.quantity
    }

    /// Dividends and coupons
    #[must_use]
    pub fn dividends(&self) -> Income {
        Income::new(
            self.totals.additional_profit + self.balance(),
            self.balance(),
        )
    }

    /// Taxes and fees
    #[must_use]
    pub fn fees(&self) -> Income {
        // IMPORTANT: we must add self.totals.fees because their value is negative
        Income::new(self.balance() + self.totals.fees, self.balance())
    }

    #[must_use]
    pub fn currency(&self) -> Currency {
        self.position.currency
    }

    #[must_use]
    pub fn quantity(&self) -> Decimal {
        self.position.quantity
    }

    #[must_use]
    pub fn current_instrument_price(&self) -> Money {
        self.position.current_instrument_price
    }

    #[must_use]
    pub fn accrued_interest(&self) -> Money {
        self.position.accrued_interest
    }

    #[must_use]
    pub fn average_buy_price(&self) -> Money {
        self.position.average_buy_price
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn bond(accrued_interest: Decimal) -> Paper<CouponProfit> {
        let currency = Currency::RUB;
        Paper {
            name: "Bond".to_string(),
            ticker: Ticker::new("BND"),
            figi: Figi::new("FIGI"),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(1000), currency),
                current_instrument_price: Money::from_value(dec!(990), currency),
                accrued_interest: Money::from_value(accrued_interest, currency),
                quantity: dec!(10),
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
            },
            profit: CouponProfit,
        }
    }

    #[test]
    fn current_includes_accrued_interest() {
        // Arrange
        let paper = bond(dec!(15.5));

        // Act
        let current = paper.current();

        // Assert
        assert_eq!(current.value, dec!(10055));
    }

    #[test]
    fn current_without_accrued_interest() {
        // Arrange
        let paper = bond(Decimal::ZERO);

        // Act
        let current = paper.current();

        // Assert
        assert_eq!(current.value, dec!(9900));
    }

    #[test]
    fn income_accounts_accrued_interest() {
        // Arrange
        let paper = bond(dec!(15.5));

        // Act
        let income = paper.income();

        // Assert
        assert_eq!(income.current, dec!(10055));
        assert_eq!(income.balance, dec!(10000));
    }
}
