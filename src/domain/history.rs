use chrono::{DateTime, Utc};
use iso_currency::Currency;
use itertools::Itertools;
use tinkoff_invest_api::tcs::{InstrumentShort, Operation, OperationState};

use crate::{to_datetime_utc, to_money};

use super::NumberRange;
use super::money::Money;

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
    pub fn new(operations: &[Operation], instrument: &InstrumentShort) -> Option<Self> {
        let items = operations
            .iter()
            .unique_by(|op| &op.id)
            .map(HistoryItem::from)
            .sorted_by(|a, b| Ord::cmp(&a.datetime, &b.datetime))
            .collect_vec();
        let currency = items.first()?.payment.currency;
        Some(Self {
            name: instrument.name.clone(),
            ticker: instrument.ticker.clone(),
            figi: instrument.figi.clone(),
            items,
            currency,
        })
    }

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
    #[must_use]
    pub fn from(op: &Operation) -> Self {
        let currency =
            Currency::from_code(&op.currency.to_ascii_uppercase()).unwrap_or(Currency::RUB);
        let payment = if let Some(payment) = to_money(op.payment.as_ref()) {
            payment
        } else {
            Money::zero(currency)
        };
        let price = if let Some(price) = to_money(op.price.as_ref()) {
            price
        } else {
            Money::zero(currency)
        };
        let state = match op.state() {
            OperationState::Unspecified => "Not specified",
            OperationState::Executed => "Executed",
            OperationState::Canceled => "Canceled",
            OperationState::Progress => "In progress",
        };

        let dt = to_datetime_utc(op.date.as_ref());
        Self {
            datetime: dt,
            quantity: op.quantity,
            quantity_rest: op.quantity_rest,
            price,
            payment,
            description: op.r#type.clone(),
            operation_state: state,
        }
    }
}
