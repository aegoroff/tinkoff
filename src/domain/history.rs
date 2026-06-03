use chrono::{DateTime, Utc};
use iso_currency::Currency;

use super::money::Money;
use super::NumberRange;

pub struct History {
    pub name: String,
    pub ticker: String,
    pub figi: String,
    pub currency: Currency,
    pub items: Vec<HistoryItem>,
}

pub struct HistoryItem {
    pub datetime: DateTime<Utc>,
    pub quantity: i64,
    pub quantity_rest: i64,
    pub price: Money,
    pub payment: Money,
    pub description: String,
    pub operation_state: &'static str,
}

impl History {
    #[must_use]
    pub fn expenses(&self) -> Money {
        self.sum(|i| i.payment.is_negative())
    }

    #[must_use]
    pub fn profit(&self) -> Money {
        self.sum(|i| !i.payment.is_negative())
    }

    #[must_use]
    pub fn balance(&self) -> Money {
        self.expenses() + self.profit()
    }

    fn sum<P>(&self, predicate: P) -> Money
    where
        P: FnMut(&&HistoryItem) -> bool,
    {
        self.items
            .iter()
            .filter(predicate)
            .fold(Money::zero(self.currency), |mut acc, p| {
                acc += p.payment;
                acc
            })
    }
}
