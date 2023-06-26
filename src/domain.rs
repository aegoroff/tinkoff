use std::fmt::Display;

use iso_currency::Currency;
use prettytable::{cell, format, row, Cell, Row, Table};
use rust_decimal::{prelude::FromPrimitive, Decimal};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Money {
    pub value: Decimal,
    pub currency: Currency,
}

pub struct Income {
    currency: Currency,
    income: Decimal,
    percent: Decimal,
}

trait NumberRange {
    fn is_negative(&self) -> bool;
    fn is_zero(&self) -> bool;
}

pub struct Paper {
    pub name: String,
    pub ticker: String,
    pub figi: String,
    pub expected_yield: Money,
    pub average_buy_price: Money,
    pub current_instrument_price: Money,
    pub quantity: Decimal,
    /// Expences to get current amount of paper, i.e. average position price multiplied to quantity
    pub balance_value: Money,
    /// Current position value, i.e. current position price multiplied to quantity
    pub current_value: Money,
    /// Taxes and fees
    pub taxes_and_fees: Money,
    /// Dividents and coupons
    pub dividents_and_coupons: Money,
}

pub struct Portfolio {
    pub bonds: Asset,
    pub shares: Asset,
    pub etfs: Asset,
    pub currencies: Asset,
}

pub struct Asset {
    name: String,
    papers: Vec<Paper>,
}

impl Money {
    pub fn new(value: Decimal, currency: String) -> Option<Self> {
        Currency::from_code(&currency.to_ascii_uppercase()).map(|currency| Self { value, currency })
    }

    pub fn from_value(value: Decimal, currency: Currency) -> Self {
        Self { value, currency }
    }
    pub fn zero(currency: Currency) -> Self {
        Self {
            value: Decimal::from_i64(0).unwrap(),
            currency,
        }
    }
}

impl Income {
    pub fn new(current: Money, balance: Money) -> Self {
        let income = current.value - balance.value;
        let percent = (income / balance.value) * Decimal::from_i16(100).unwrap_or_default();
        Self {
            currency: current.currency,
            percent,
            income,
        }
    }
}

impl Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.value.round_dp(2), self.currency.symbol())
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
            self.income.round_dp(2),
            self.currency.symbol(),
            self.percent.round_dp(2)
        )
    }
}

impl NumberRange for Income {
    fn is_negative(&self) -> bool {
        self.income.is_sign_negative()
    }

    fn is_zero(&self) -> bool {
        self.income.is_zero()
    }
}

impl Portfolio {
    pub fn new() -> Self {
        Self {
            bonds: Asset::new("Bonds".to_owned()),
            shares: Asset::new("Stocks".to_owned()),
            etfs: Asset::new("Etfs".to_owned()),
            currencies: Asset::new("Currencies".to_owned()),
        }
    }
}

impl Default for Portfolio {
    fn default() -> Self {
        Self::new()
    }
}

impl Asset {
    pub fn new(name: String) -> Self {
        Self {
            papers: vec![],
            name,
        }
    }

    pub fn add_paper(&mut self, paper: Paper) {
        self.papers.push(paper);
    }
}

impl Asset {
    pub fn printstd(&self) {
        print!("\n{}:\n\n", self.name);
        for p in &self.papers {
            p.printstd();
            println!();
            println!();
        }
    }
}

impl Paper {
    pub fn printstd(&self) {
        let mut table = Table::new();

        let format = format::FormatBuilder::new()
            .column_separator(' ')
            .borders(' ')
            .separators(
                &[format::LinePosition::Title],
                format::LineSeparator::new('-', ' ', ' ', ' '),
            )
            .indent(1)
            .padding(0, 0)
            .build();
        table.set_format(format);

        let currency = self.balance_value.currency.code().to_owned();
        let title = format!(
            "{} ({} | {} | {})",
            self.name, self.ticker, self.figi, currency
        );
        table.set_titles(row![bFH2 => title]);
        table.add_row(row!["Average buy price", self.average_buy_price]);
        table.add_row(row!["Last instrument price", self.current_instrument_price]);
        table.add_row(row!["Current items count", self.quantity.round_dp(2)]);
        table.add_row(row!["Expenses", self.balance_value]);
        table.add_row(row!["Current position price", self.current_value]);
        table.add_empty_row();

        let income = Income::new(self.current_value, self.balance_value);
        let expected_yield = Self::colored_cell(income);
        table.add_row(Row::new(vec![cell!("Income"), expected_yield]));

        let dividents_and_coupons = Self::colored_cell(self.dividents_and_coupons);
        table.add_row(Row::new(vec![cell!("Dividends"), dividents_and_coupons]));

        let taxes_and_fees = Self::colored_cell(self.taxes_and_fees);
        table.add_row(Row::new(vec![cell!("Taxes and fees"), taxes_and_fees]));

        table.printstd();
    }

    fn colored_cell<T: NumberRange + ToString>(value: T) -> Cell {
        if value.is_negative() {
            cell!(Fr->value)
        } else if value.is_zero() {
            cell!(value)
        } else {
            cell!(Fg->value)
        }
    }
}
