use std::{
    fmt::Display,
    ops::{self, AddAssign, DivAssign, MulAssign, SubAssign},
};

use comfy_table::{Attribute, Cell, TableComponent};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ux::{self, format_decimal};

const HUNDRED: Decimal = dec!(100);
const TOTAL_INCOME: &str = "Total income";
const INCOME: &str = "Income";
const CURRENT_VALUE: &str = "Current value";
const BALANCE_VALUE: &str = "Balance value";
const BALANCE_INCOME: &str = "Balance income";

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Money {
    pub value: Decimal,
    pub currency: Currency,
}

pub struct Instrument {
    pub name: String,
    pub ticker: String,
}

#[derive(Clone, Copy)]
pub struct Income {
    currency: Currency,
    current: Decimal,
    balance: Decimal,
}

#[derive(Clone, Copy)]
pub struct Position {
    pub currency: Currency,
    pub average_buy_price: Money,
    pub current_instrument_price: Money,
    pub quantity: Decimal,
}

pub trait NumberRange {
    fn is_negative(&self) -> bool;
    fn is_zero(&self) -> bool;
}

/// Paper represents things like share, bond, currency, etf etc.
pub struct Paper<P: Profit> {
    pub name: String,
    pub ticker: String,
    pub figi: String,
    pub position: Position,
    pub totals: Totals,
    pub profit: P,
}

/// Portfolio is an Asset's container
/// Asset is a Paper's container
pub struct Portfolio {
    pub bonds: Asset<CouponProfit>,
    pub shares: Asset<DivdentProfit>,
    pub etfs: Asset<NoneProfit>,
    pub currencies: Asset<NoneProfit>,
    pub futures: Asset<NoneProfit>,
}

/// Asset is a Paper's container
pub struct Asset<P: Profit> {
    name: &'static str,
    papers: Vec<Paper<P>>,
    pub profit: P,
    /// Whether to include asset's papers into output
    /// If true papers will be displyed
    /// If false they only accounted during calculations (balance, income etc,)
    output_papers: bool,
}

pub struct Totals {
    /// Dividents and coupons
    pub dividents: Money,
    /// Taxes and fees
    pub fees: Money,
}

pub trait Profit: Copy {
    fn has_profit() -> bool;
    fn profit_name() -> &'static str;
}

#[derive(Clone, Copy)]
pub struct DivdentProfit;
#[derive(Clone, Copy)]
pub struct CouponProfit;
#[derive(Clone, Copy)]
pub struct NoneProfit;

impl Profit for DivdentProfit {
    fn has_profit() -> bool {
        true
    }

    fn profit_name() -> &'static str {
        "Dividents"
    }
}

impl Profit for CouponProfit {
    fn has_profit() -> bool {
        true
    }

    fn profit_name() -> &'static str {
        "Coupons"
    }
}

impl Profit for NoneProfit {
    fn has_profit() -> bool {
        false
    }

    fn profit_name() -> &'static str {
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
        self.income() + Income::new(self.dividents(), Money::zero(self.currency()))
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
    pub fn dividents(&self) -> Money {
        self.totals.dividents
    }

    /// Taxes and fees
    #[must_use]
    pub fn fees(&self) -> Money {
        self.totals.fees
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
}

impl ops::Add<Money> for Money {
    type Output = Money;

    fn add(self, rhs: Money) -> Money {
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
        self.value /= other.value;
    }
}

impl DivAssign<Decimal> for Money {
    fn div_assign(&mut self, other: Decimal) {
        self.value /= other;
    }
}

impl Income {
    #[must_use]
    pub fn new(current: Money, balance: Money) -> Self {
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

impl ops::Add<Income> for Income {
    type Output = Income;

    fn add(self, rhs: Income) -> Income {
        Income {
            current: self.current + rhs.current,
            balance: self.balance + rhs.balance,
            currency: self.currency,
        }
    }
}

impl AddAssign for Income {
    fn add_assign(&mut self, other: Self) {
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

impl Portfolio {
    #[must_use]
    pub fn new(output_papers: bool) -> Self {
        Self {
            bonds: Asset::new("Bonds", CouponProfit, output_papers),
            shares: Asset::new("Shares", DivdentProfit, output_papers),
            etfs: Asset::new("Etfs", NoneProfit, output_papers),
            currencies: Asset::new("Currencies", NoneProfit, output_papers),
            futures: Asset::new("Futures", NoneProfit, output_papers),
        }
    }

    #[must_use]
    pub fn income(&self) -> Income {
        self.bonds.income()
            + self.shares.income()
            + self.currencies.income()
            + self.etfs.income()
            + self.futures.income()
    }

    #[must_use]
    pub fn total_income(&self) -> Income {
        self.bonds.total_income()
            + self.shares.total_income()
            + self.currencies.total_income()
            + self.etfs.total_income()
            + self.futures.total_income()
    }

    #[must_use]
    pub fn balance(&self) -> Money {
        self.bonds.balance()
            + self.shares.balance()
            + self.currencies.balance()
            + self.etfs.balance()
            + self.futures.balance()
    }

    #[must_use]
    pub fn current(&self) -> Money {
        self.bonds.current()
            + self.shares.current()
            + self.currencies.current()
            + self.etfs.current()
            + self.futures.current()
    }

    #[must_use]
    pub fn dividents(&self) -> Money {
        self.bonds.dividents()
            + self.shares.dividents()
            + self.etfs.dividents()
            + self.futures.dividents()
    }
}

impl Default for Portfolio {
    fn default() -> Self {
        Self::new(true)
    }
}

impl<P: Profit> Asset<P> {
    #[must_use]
    pub fn new(name: &'static str, profit: P, output_papers: bool) -> Self {
        Self {
            papers: vec![],
            name,
            output_papers,
            profit,
        }
    }

    pub fn add_paper(&mut self, paper: Paper<P>) {
        self.papers.push(paper);
    }

    pub fn income(&self) -> Income {
        self.fold(Income::zero, |mut acc, p| {
            acc += p.income();
            acc
        })
    }

    pub fn total_income(&self) -> Income {
        self.fold(Income::zero, |mut acc, p| {
            acc += p.total_income();
            acc
        })
    }

    pub fn current(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc += p.current();
            acc
        })
    }

    pub fn balance(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc += p.balance();
            acc
        })
    }

    pub fn dividents(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc += p.dividents();
            acc
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.papers.is_empty()
    }

    fn fold<B, IF, F>(&self, mut init: IF, f: F) -> B
    where
        IF: FnMut(Currency) -> B,
        F: FnMut(B, &Paper<P>) -> B,
    {
        let currency = self.currency();
        self.papers.iter().fold(init(currency), f)
    }

    fn currency(&self) -> Currency {
        if self.papers.is_empty() {
            iso_currency::Currency::RUB
        } else {
            self.papers[0].currency()
        }
    }
}

impl<P: Profit> Display for Asset<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut asset_table = ux::new_table();
        asset_table.set_header(vec![Cell::new(self.name)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        asset_table.set_style(TableComponent::HeaderLines, ' ');

        if self.output_papers {
            for p in &self.papers {
                asset_table.add_row(vec![Cell::new(p)]);
            }
        }

        let balance_income = ux::colored_cell(self.income());
        let total_income = ux::colored_cell(self.total_income());

        let mut table = ux::new_table();

        let title = format!("{} totals:", self.name);
        let title = Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header(vec![title, Cell::new("")]);

        table.add_row(vec![Cell::new(BALANCE_VALUE), Cell::new(self.balance())]);
        table.add_row(vec![Cell::new(CURRENT_VALUE), Cell::new(self.current())]);
        table.add_row(vec![Cell::new(BALANCE_INCOME), balance_income]);

        if P::has_profit() {
            table.add_row(vec![Cell::new(TOTAL_INCOME), total_income]);
            table.add_row(vec![
                Cell::new(P::profit_name()),
                ux::colored_cell(self.dividents()),
            ]);
        }

        table.add_row(vec![
            Cell::new("Instruments count"),
            Cell::new(self.papers.len()),
        ]);
        asset_table.add_row(vec![Cell::new(table)]);

        if self.is_empty() {
            Ok(())
        } else {
            write!(f, "{asset_table}")
        }
    }
}

impl<P: Profit> Display for Paper<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();

        let currency = self.currency().code().to_owned();
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
            Cell::new(self.average_buy_price()),
        ]);
        table.add_row(vec![
            Cell::new("Last instrument price"),
            Cell::new(self.current_instrument_price()),
        ]);
        table.add_row(vec![
            Cell::new("Current items count"),
            Cell::new(self.quantity().round_dp(2)),
        ]);
        table.add_row(vec![Cell::new(BALANCE_VALUE), Cell::new(self.balance())]);
        table.add_row(vec![Cell::new(CURRENT_VALUE), Cell::new(self.current())]);
        table.add_row(vec!["", ""]);

        table.add_row(vec![Cell::new(INCOME), ux::colored_cell(self.income())]);

        if P::has_profit() {
            table.add_row(vec![
                Cell::new(P::profit_name()),
                ux::colored_cell(self.dividents()),
            ]);

            table.add_row(vec![
                Cell::new(TOTAL_INCOME),
                ux::colored_cell(self.total_income()),
            ]);
        }

        table.add_row(vec![
            Cell::new("Taxes and fees"),
            ux::colored_cell(self.fees()),
        ]);

        write!(f, "{table}")
    }
}

impl Display for Portfolio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.etfs.fmt(f)?;
        self.futures.fmt(f)?;
        self.bonds.fmt(f)?;
        self.shares.fmt(f)?;
        self.currencies.fmt(f)?;

        let mut table = ux::new_table();

        let title = Cell::new("Portfolio totals:")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkRed);
        table.set_header(vec![title, Cell::new("")]);

        table.add_row(vec![
            Cell::new(BALANCE_INCOME),
            ux::colored_cell(self.income()),
        ]);
        table.add_row(vec![
            Cell::new(TOTAL_INCOME),
            ux::colored_cell(self.total_income()),
        ]);
        table.add_row(vec![
            Cell::new("Dividents and coupons"),
            ux::colored_cell(self.dividents()),
        ]);
        table.add_row(vec![Cell::new(BALANCE_VALUE), Cell::new(self.balance())]);
        table.add_row(vec![Cell::new(CURRENT_VALUE), Cell::new(self.current())]);

        writeln!(f)?;
        writeln!(f, "{table}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};

    #[rstest]
    fn portfolio_balance(test_portfolio: Portfolio) {
        assert_eq!(dec!(1500), test_portfolio.balance().value);
    }

    #[rstest]
    fn portfolio_current(test_portfolio: Portfolio) {
        assert_eq!(dec!(1700), test_portfolio.current().value);
    }

    #[rstest]
    fn portfolio_income(test_portfolio: Portfolio) {
        assert_eq!(dec!(1500), test_portfolio.income().balance);
        assert_eq!(dec!(1700), test_portfolio.income().current);
        assert_eq!(dec!(13.33), test_portfolio.income().percent().round_dp(2));
    }

    #[rstest]
    fn portfolio_dividents(test_portfolio: Portfolio) {
        assert_eq!(dec!(150), test_portfolio.dividents().value);
    }

    #[rstest]
    fn portfolio_total_income(test_portfolio: Portfolio) {
        assert_eq!(dec!(1850), test_portfolio.total_income().current);
    }

    #[fixture]
    fn test_portfolio() -> Portfolio {
        let currency = Currency::RUB;
        let mut bonds = Asset::new("Bonds", CouponProfit, true);
        bonds.add_paper(Paper {
            name: "1".to_string(),
            ticker: "1t".to_string(),
            figi: "1f".to_string(),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(10), currency),
                current_instrument_price: Money::from_value(dec!(11), currency),
                quantity: dec!(100),
            },
            totals: Totals {
                dividents: Money::from_value(dec!(100), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: CouponProfit,
        });
        let mut shares = Asset::new("Shares", DivdentProfit, true);
        shares.add_paper(Paper {
            name: "2".to_string(),
            ticker: "2t".to_string(),
            figi: "2f".to_string(),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(5), currency),
                current_instrument_price: Money::from_value(dec!(6), currency),
                quantity: dec!(100),
            },
            totals: Totals {
                dividents: Money::from_value(dec!(50), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: DivdentProfit,
        });

        let etfs = Asset::new("Etfs", NoneProfit, true);
        let currencies = Asset::new("Currencies", NoneProfit, true);
        let futures = Asset::new("Futures", NoneProfit, true);
        Portfolio {
            bonds,
            shares,
            etfs,
            currencies,
            futures,
        }
    }
}
