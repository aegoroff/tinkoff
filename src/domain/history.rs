use super::NumberRange;
use super::money::Money;
use super::operation::Operation;
use chrono::{DateTime, Utc};
use iso_currency::Currency;

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

impl HistoryItem {
    /// History line of an operation; missing amounts are zero in the payment currency.
    #[must_use]
    pub fn from(op: &Operation) -> Self {
        let currency = op.payment.map_or(Currency::RUB, |p| p.currency);
        Self {
            datetime: op.date,
            quantity: op.quantity,
            quantity_rest: op.quantity_rest,
            price: op.price.unwrap_or_else(|| Money::zero(currency)),
            payment: op.payment.unwrap_or_else(|| Money::zero(currency)),
            description: op.description.clone(),
            operation_state: op.state.label(),
        }
    }
}
