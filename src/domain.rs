use std::fmt::Display;

use comfy_table::{Attribute, Cell, TableComponent};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ux::{self, format_decimal};

const HUNDRED: Decimal = dec!(100);
const TOTAL_INCOME: &str = "Total income";

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Money {
    pub value: Decimal,
    pub currency: Currency,
}

pub struct Instrument {
    pub name: String,
    pub ticker: String,
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
    /// Expences (the amount of money thea really spent), i.e. average position price multiplied to quantity
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
    verbose: bool,
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
            value: Decimal::default(),
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
        let income = self.income();
        let percent = if self.balance.is_zero() {
            Decimal::default()
        } else {
            (income / self.balance) * HUNDRED
        };

        write!(
            f,
            "{} {} ({}%)",
            format_decimal(income)?,
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
    pub fn new(verbose: bool) -> Self {
        Self {
            bonds: Asset::new("Bonds".to_owned(), verbose),
            shares: Asset::new("Shares".to_owned(), verbose),
            etfs: Asset::new("Etfs".to_owned(), verbose),
            currencies: Asset::new("Currencies".to_owned(), verbose),
        }
    }
}

impl Default for Portfolio {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Asset {
    pub fn new(name: String, verbose: bool) -> Self {
        Self {
            papers: vec![],
            name,
            verbose,
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

    pub fn balance(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc.add(&p.balance_value);
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
        let currency = if !self.papers.is_empty() {
            self.papers[0].current_value.currency
        } else {
            iso_currency::Currency::RUB
        };
        self.papers.iter().fold(init(currency), f)
    }
}

impl Display for Asset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut asset_table = ux::new_table();
        asset_table.set_header(vec![Cell::new(&self.name)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        asset_table.set_style(TableComponent::HeaderLines, ' ');

        if self.verbose {
            for p in &self.papers {
                asset_table.add_row(vec![Cell::new(p)]);
            }
        }

        let balance_income = self.income();
        let dividents = self.dividents();
        let balance_value = self.balance();
        let current_value = self.current();
        let mut total_income = Income::new(dividents, Money::zero(dividents.currency));
        total_income.add(&balance_income);

        let balance_income = ux::colored_cell(balance_income);
        let total_income = ux::colored_cell(total_income);

        let mut table = ux::new_table();

        let title = format!("{} totals:", self.name);
        let title = Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header(vec![title, Cell::new("")]);

        table.add_row(vec![Cell::new("Balance value"), Cell::new(balance_value)]);
        table.add_row(vec![Cell::new("Current value"), Cell::new(current_value)]);
        table.add_row(vec![Cell::new("Balance income"), balance_income]);
        table.add_row(vec![Cell::new(TOTAL_INCOME), total_income]);
        table.add_row(vec![
            Cell::new("Dividents or coupons"),
            ux::colored_cell(dividents),
        ]);
        table.add_row(vec![
            Cell::new("Instruments count"),
            Cell::new(self.papers.len()),
        ]);
        asset_table.add_row(vec![Cell::new(table)]);
        write!(f, "{asset_table}")
    }
}

impl Display for Paper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();

        let currency = self.balance_value.currency.code().to_owned();
        let title = format!(
            "{} ({} | {} | {})",
            self.name, self.ticker, self.figi, currency
        );

        table.set_header(vec![
            Cell::new(title).add_attribute(Attribute::Bold),
            Cell::new(""),
        ]);

        table.add_row(vec![
            Cell::new("Average buy price"),
            Cell::new(self.average_buy_price),
        ]);
        table.add_row(vec![
            Cell::new("Last instrument price"),
            Cell::new(self.current_instrument_price),
        ]);
        table.add_row(vec![
            Cell::new("Current items count"),
            Cell::new(self.quantity.round_dp(2)),
        ]);
        table.add_row(vec![
            Cell::new("Balance value"),
            Cell::new(self.balance_value),
        ]);
        table.add_row(vec![
            Cell::new("Current value"),
            Cell::new(self.current_value),
        ]);
        table.add_row(vec!["", ""]);

        let income = Income::new(self.current_value, self.balance_value);
        let mut total_income = Income::new(
            self.dividents_and_coupons,
            Money::zero(self.dividents_and_coupons.currency),
        );
        total_income.add(&income);

        let expected_yield = ux::colored_cell(income);
        table.add_row(vec![Cell::new("Income"), expected_yield]);

        let dividents_and_coupons = ux::colored_cell(self.dividents_and_coupons);
        table.add_row(vec![Cell::new("Dividends"), dividents_and_coupons]);

        let total_income = ux::colored_cell(total_income);
        table.add_row(vec![Cell::new(TOTAL_INCOME), total_income]);

        let taxes_and_fees = ux::colored_cell(self.taxes_and_fees);
        table.add_row(vec![Cell::new("Taxes and fees"), taxes_and_fees]);

        write!(f, "{table}")
    }
}

impl Display for Portfolio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.etfs)?;
        write!(f, "{}", self.bonds)?;
        write!(f, "{}", self.shares)?;
        write!(f, "{}", self.currencies)?;

        let mut income = self.bonds.income();
        income.add(&self.shares.income());
        income.add(&self.currencies.income());

        let mut balance = self.bonds.balance();
        balance.add(&self.shares.balance());
        balance.add(&self.currencies.balance());

        let mut dividents = self.bonds.dividents();
        dividents.add(&self.shares.dividents());

        let mut total_income = Income::new(dividents, Money::zero(dividents.currency));
        total_income.add(&income);

        let mut current = self.bonds.current();
        current.add(&self.shares.current());
        current.add(&self.currencies.current());

        let income = ux::colored_cell(income);
        let total_income = ux::colored_cell(total_income);
        let mut table = ux::new_table();

        let title = Cell::new("Portfolio totals:")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkRed);
        table.set_header(vec![title, Cell::new("")]);

        table.add_row(vec![Cell::new("Balance income"), income]);
        table.add_row(vec![Cell::new(TOTAL_INCOME), total_income]);
        table.add_row(vec![
            Cell::new("Dividents and coupons"),
            ux::colored_cell(dividents),
        ]);
        table.add_row(vec![Cell::new("Balance value"), Cell::new(balance)]);
        table.add_row(vec![Cell::new("Current value"), Cell::new(current)]);

        writeln!(f)?;
        writeln!(f, "{table}")
    }
}
