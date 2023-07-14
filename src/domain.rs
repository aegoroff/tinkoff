use std::fmt::Display;

use crossterm::style::{style, Color, Stylize};
use iso_currency::Currency;
use prettytable::{cell, row, Row, Table};
use rust_decimal::{prelude::FromPrimitive, Decimal};

use crate::ux;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Money {
    pub value: Decimal,
    pub currency: Currency,
}

pub struct Income {
    currency: Currency,
    current: Decimal,
    balance: Decimal,
}

pub trait NumberRange {
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

    pub fn add(&mut self, other: &Money) {
        self.value += other.value;
    }
}

impl Income {
    pub fn new(current: Money, balance: Money) -> Self {
        Self {
            currency: current.currency,
            current: current.value,
            balance: balance.value,
        }
    }

    pub fn zero(currency: Currency) -> Self {
        Self {
            currency,
            current: Decimal::default(),
            balance: Decimal::default(),
        }
    }

    pub fn add(&mut self, other: &Income) {
        self.current += other.current;
        self.balance += other.balance;
    }

    fn income(&self) -> Decimal {
        self.current - self.balance
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
        let income = self.income();
        let percent = (income / self.balance) * Decimal::from_i16(100).unwrap_or_default();

        write!(
            f,
            "{} {} ({}%)",
            income.round_dp(2),
            self.currency.symbol(),
            percent.round_dp(2)
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

    pub fn income(&self) -> Income {
        self.fold(Income::zero, |mut acc, p| {
            let income = Income::new(p.current_value, p.balance_value);
            acc.add(&income);
            acc
        })
    }

    pub fn current(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc.add(&p.current_value);
            acc
        })
    }

    pub fn dividents(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc.add(&p.dividents_and_coupons);
            acc
        })
    }

    fn fold<B, IF, F>(&self, mut init: IF, f: F) -> B
    where
        IF: FnMut(Currency) -> B,
        F: FnMut(B, &Paper) -> B,
    {
        let currency = self.papers[0].current_value.currency;
        self.papers.iter().fold(init(currency), f)
    }

    pub fn printstd(&self) {
        let name = style(&self.name).with(Color::Blue).bold();

        print!("\n {name}:\n\n");
        for p in &self.papers {
            p.printstd();
            println!();
            println!();
        }

        let balance_income = self.income();
        let dividents = self.dividents();
        let mut total_income = Income::new(dividents, Money::zero(dividents.currency));
        total_income.add(&balance_income);

        let balance_income = ux::colored_cell(balance_income);
        let total_income = ux::colored_cell(total_income);

        let mut table = Table::new();
        table.set_format(ux::new_table_format());

        let title = format!("{} totals:", self.name);
        table.set_titles(row![bFyH2 => title, ""]);
        table.add_row(Row::new(vec![cell!("Balance income"), balance_income]));
        table.add_row(Row::new(vec![cell!("Total income"), total_income]));
        table.add_row(row!["Dividents or coupons", Fg->dividents]);
        table.add_row(row!["Instruments count", self.papers.len()]);
        table.printstd();
    }
}

impl Paper {
    pub fn printstd(&self) {
        let mut table = Table::new();
        table.set_format(ux::new_table_format());

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
        let expected_yield = ux::colored_cell(income);
        table.add_row(Row::new(vec![cell!("Income"), expected_yield]));

        let dividents_and_coupons = ux::colored_cell(self.dividents_and_coupons);
        table.add_row(Row::new(vec![cell!("Dividends"), dividents_and_coupons]));

        let taxes_and_fees = ux::colored_cell(self.taxes_and_fees);
        table.add_row(Row::new(vec![cell!("Taxes and fees"), taxes_and_fees]));

        table.printstd();
    }
}
