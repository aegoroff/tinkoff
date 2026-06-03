use iso_currency::Currency;
use rust_decimal::Decimal;

use super::money::{Income, Money};

#[derive(Clone)]
pub struct Instrument {
    pub name: String,
    pub ticker: String,
}

#[derive(Clone, Copy)]
pub struct Position {
    pub currency: Currency,
    pub average_buy_price: Money,
    pub current_instrument_price: Money,
    pub quantity: Decimal,
}

pub struct Totals {
    /// Dividents, coupons etc. i.e. some extra value
    /// an asset may earn
    pub additional_profit: Money,
    /// Taxes and fees
    pub fees: Money,
}

/// Represents additional asset profit
/// besides balance value growing due to price increase.
/// Used mainly for output
pub trait Profit: Copy {
    /// shows whether additional profit
    /// applicable to an asset
    fn applicable() -> bool;
    /// Profit name
    fn name() -> &'static str;
}

#[derive(Clone, Copy)]
pub struct DividentProfit;
#[derive(Clone, Copy)]
pub struct CouponProfit;
#[derive(Clone, Copy)]
pub struct NoneProfit;

/// Paper represents things like share, bond, currency, etf etc.
pub struct Paper<P: Profit> {
    pub name: String,
    pub ticker: String,
    pub figi: String,
    pub position: Position,
    pub totals: Totals,
    pub profit: P,
}

impl Profit for DividentProfit {
    fn applicable() -> bool {
        true
    }

    fn name() -> &'static str {
        "Dividents"
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

    /// Total income (income + dividents)
    #[must_use]
    pub fn total_income(&self) -> Income {
        let div = self.dividents();
        Income::new(self.current() + (div.current - div.balance), self.balance())
    }

    /// Expences (the amount of money thea really spent), i.e. average position price multiplied to quantity
    #[must_use]
    pub fn balance(&self) -> Money {
        self.position.average_buy_price * self.position.quantity
    }

    /// Current position value, i.e. current position price multiplied to quantity
    #[must_use]
    pub fn current(&self) -> Money {
        self.position.current_instrument_price * self.position.quantity
    }

    /// Dividents and coupons
    #[must_use]
    pub fn dividents(&self) -> Income {
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
    pub fn average_buy_price(&self) -> Money {
        self.position.average_buy_price
    }
}
