use std::{
    fmt::Display,
    ops::{self, AddAssign, DivAssign, MulAssign, SubAssign},
};

use chrono::{DateTime, Datelike, Utc};
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

#[derive(Clone)]
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

/// Portfolio is an [`Asset`]'s container
/// [`Asset`] is a [`Paper`]'s container
pub struct Portfolio {
    pub bonds: Asset<CouponProfit>,
    pub shares: Asset<DividentProfit>,
    pub etfs: Asset<NoneProfit>,
    pub currencies: Asset<NoneProfit>,
    pub futures: Asset<NoneProfit>,
}

/// Asset is a [`Paper`]'s container
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

/// Dividend payment information
#[derive(Clone)]
pub struct DividendPayment {
    pub figi: String,
    pub ticker: String,
    pub name: String,
    pub currency: Currency,
    pub dividend_per_share: Money,
    pub total_dividend: Money,
    pub quantity: Decimal,
    pub ex_dividend_date: DateTime<Utc>,
    pub payment_date: Option<DateTime<Utc>>,
    pub dividend_type: String,
}

/// Dividend calendar with upcoming payments
pub struct DividendCalendar {
    pub upcoming: Vec<DividendPayment>,
}

/// Coupon payment information
#[derive(Clone)]
pub struct CouponPayment {
    pub figi: String,
    pub ticker: String,
    pub name: String,
    pub currency: Currency,
    pub coupon_per_bond: Money,
    pub total_coupon: Money,
    pub quantity: Decimal,
    pub coupon_date: DateTime<Utc>,
    pub coupon_type: String,
}

/// Coupon calendar with upcoming payments
pub struct CouponCalendar {
    pub upcoming: Vec<CouponPayment>,
}

impl Display for CouponPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} | {} | {})",
            self.name,
            self.ticker,
            self.figi,
            self.currency.code()
        )
    }
}

impl Display for DividendPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} | {} | {})",
            self.name,
            self.ticker,
            self.figi,
            self.currency.code()
        )
    }
}

fn format_date(dt: DateTime<Utc>) -> String {
    format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
}

impl Display for DividendCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();

        let title = Cell::new("Dividend Calendar")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue);
        table.set_header([title]);

        // Main table headers
        let payment_date = Cell::new("Payment Date").add_attribute(Attribute::Bold);
        let ex_dividend_date = Cell::new("Ex-Dividend Date").add_attribute(Attribute::Bold);
        let company = Cell::new("Company").add_attribute(Attribute::Bold);
        let dividend_per_share = Cell::new("Dividend per Share").add_attribute(Attribute::Bold);
        let total_dividend = Cell::new("Total Dividend").add_attribute(Attribute::Bold);
        table.add_row([
            payment_date,
            ex_dividend_date,
            company,
            dividend_per_share,
            total_dividend,
        ]);

        // Upcoming payments
        if self.upcoming.is_empty() {
            table.add_row([
                Cell::new("No upcoming payments"),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
            ]);
        } else {
            // Group by year and month
            let mut grouped: std::collections::HashMap<(i32, u32), Vec<&DividendPayment>> =
                std::collections::HashMap::new();

            for payment in &self.upcoming {
                let year = payment.ex_dividend_date.year();
                let month = payment.ex_dividend_date.month();
                grouped.entry((year, month)).or_default().push(payment);
            }

            // Sort keys by year and month
            let mut keys: Vec<_> = grouped.keys().copied().collect();
            keys.sort_by(|a, b| {
                let year_cmp = a.0.cmp(&b.0);
                if year_cmp == std::cmp::Ordering::Equal {
                    a.1.cmp(&b.1)
                } else {
                    year_cmp
                }
            });

            let mut grand_total = Money::zero(iso_currency::Currency::RUB);

            // Group by year
            let mut by_year: std::collections::HashMap<i32, Vec<u32>> =
                std::collections::HashMap::new();
            for (year, month) in &keys {
                by_year.entry(*year).or_default().push(*month);
            }

            let mut year_keys: Vec<_> = by_year.keys().copied().collect();
            year_keys.sort_unstable();

            for year in year_keys {
                // Year header
                table.add_row([
                    Cell::new(format!("=== {year} ==="))
                        .add_attribute(Attribute::Bold)
                        .fg(comfy_table::Color::DarkCyan),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ]);

                let mut year_total = Money::zero(iso_currency::Currency::RUB);
                let months = by_year.get(&year).unwrap();

                for month in months {
                    // Month header
                    let month_name = match month {
                        1 => "January",
                        2 => "February",
                        3 => "March",
                        4 => "April",
                        5 => "May",
                        6 => "June",
                        7 => "July",
                        8 => "August",
                        9 => "September",
                        10 => "October",
                        11 => "November",
                        12 => "December",
                        _ => "Unknown",
                    };
                    table.add_row([
                        Cell::new(format!("--- {month_name} ---")),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                    ]);

                    let mut month_total = Money::zero(iso_currency::Currency::RUB);
                    let payments = grouped.get(&(year, *month)).unwrap();

                    for payment in payments {
                        let payment_date_str = payment
                            .payment_date
                            .map_or_else(|| "-".to_string(), format_date);

                        table.add_row([
                            Cell::new(payment_date_str),
                            Cell::new(format_date(payment.ex_dividend_date)),
                            Cell::new(payment.name.clone()),
                            Cell::new(payment.dividend_per_share.to_string()),
                            Cell::new(payment.total_dividend.to_string()),
                        ]);

                        month_total += payment.total_dividend;
                    }

                    // Month total
                    table.add_row([
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format!("Month {month_name} Total:"))
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(month_total.to_string()).add_attribute(Attribute::Bold),
                    ]);

                    year_total += month_total;
                    grand_total += month_total;
                }

                // Year total
                table.add_row([
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(format!("Year {year} Total:"))
                        .add_attribute(Attribute::Bold)
                        .fg(comfy_table::Color::DarkYellow),
                    Cell::new(""),
                    Cell::new(year_total.to_string()).add_attribute(Attribute::Bold),
                ]);

                table.add_row([
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ]);
            }

            // Grand total row
            table.add_row([
                Cell::new(""),
                Cell::new(""),
                Cell::new("Grand Total")
                    .add_attribute(Attribute::Bold)
                    .fg(comfy_table::Color::DarkRed),
                Cell::new(""),
                Cell::new(grand_total.to_string())
                    .add_attribute(Attribute::Bold)
                    .fg(comfy_table::Color::DarkGreen),
            ]);
        }

        write!(f, "{table}")
    }
}

impl Display for CouponCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();

        let title = Cell::new("Coupon Calendar")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue);
        table.set_header([title]);

        // Main table headers
        let payment_date = Cell::new("Payment Date").add_attribute(Attribute::Bold);
        let coupon_date = Cell::new("Coupon Date").add_attribute(Attribute::Bold);
        let company = Cell::new("Company").add_attribute(Attribute::Bold);
        let coupon_per_bond = Cell::new("Coupon per Bond").add_attribute(Attribute::Bold);
        let total_coupon = Cell::new("Total Coupon").add_attribute(Attribute::Bold);
        table.add_row([
            payment_date,
            coupon_date,
            company,
            coupon_per_bond,
            total_coupon,
        ]);

        // Upcoming payments
        if self.upcoming.is_empty() {
            table.add_row([
                Cell::new("No upcoming payments"),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
                Cell::new(""),
            ]);
        } else {
            // Group by year and month
            let mut grouped: std::collections::HashMap<(i32, u32), Vec<&CouponPayment>> =
                std::collections::HashMap::new();

            for payment in &self.upcoming {
                let year = payment.coupon_date.year();
                let month = payment.coupon_date.month();
                grouped.entry((year, month)).or_default().push(payment);
            }

            // Sort keys by year and month
            let mut keys: Vec<_> = grouped.keys().copied().collect();
            keys.sort_by(|a, b| {
                let year_cmp = a.0.cmp(&b.0);
                if year_cmp == std::cmp::Ordering::Equal {
                    a.1.cmp(&b.1)
                } else {
                    year_cmp
                }
            });

            let mut grand_total = Money::zero(iso_currency::Currency::RUB);

            // Group by year
            let mut by_year: std::collections::HashMap<i32, Vec<u32>> =
                std::collections::HashMap::new();
            for (year, month) in &keys {
                by_year.entry(*year).or_default().push(*month);
            }

            let mut year_keys: Vec<_> = by_year.keys().copied().collect();
            year_keys.sort_unstable();

            for year in year_keys {
                // Year header
                table.add_row([
                    Cell::new(format!("=== {year} ==="))
                        .add_attribute(Attribute::Bold)
                        .fg(comfy_table::Color::DarkCyan),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ]);

                let mut year_total = Money::zero(iso_currency::Currency::RUB);
                let months = by_year.get(&year).unwrap();

                for month in months {
                    // Month header
                    let month_name = match month {
                        1 => "January",
                        2 => "February",
                        3 => "March",
                        4 => "April",
                        5 => "May",
                        6 => "June",
                        7 => "July",
                        8 => "August",
                        9 => "September",
                        10 => "October",
                        11 => "November",
                        12 => "December",
                        _ => "Unknown",
                    };
                    table.add_row([
                        Cell::new(format!("--- {month_name} ---")),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(""),
                    ]);

                    let mut month_total = Money::zero(iso_currency::Currency::RUB);
                    let payments = grouped.get(&(year, *month)).unwrap();

                    for payment in payments {
                        let payment_date_str = format_date(payment.coupon_date);

                        table.add_row([
                            Cell::new(payment_date_str),
                            Cell::new(format_date(payment.coupon_date)),
                            Cell::new(payment.name.clone()),
                            Cell::new(payment.coupon_per_bond.to_string()),
                            Cell::new(payment.total_coupon.to_string()),
                        ]);

                        month_total += payment.total_coupon;
                    }

                    // Month total
                    table.add_row([
                        Cell::new(""),
                        Cell::new(""),
                        Cell::new(format!("Month {month_name} Total:"))
                            .add_attribute(Attribute::Bold),
                        Cell::new(""),
                        Cell::new(month_total.to_string()).add_attribute(Attribute::Bold),
                    ]);

                    year_total += month_total;
                    grand_total += month_total;
                }

                // Year total
                table.add_row([
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(format!("Year {year} Total:"))
                        .add_attribute(Attribute::Bold)
                        .fg(comfy_table::Color::DarkYellow),
                    Cell::new(""),
                    Cell::new(year_total.to_string()).add_attribute(Attribute::Bold),
                ]);

                table.add_row([
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ]);
            }

            // Grand total row
            table.add_row([
                Cell::new(""),
                Cell::new(""),
                Cell::new("Grand Total")
                    .add_attribute(Attribute::Bold)
                    .fg(comfy_table::Color::DarkRed),
                Cell::new(""),
                Cell::new(grand_total.to_string())
                    .add_attribute(Attribute::Bold)
                    .fg(comfy_table::Color::DarkGreen),
            ]);
        }

        write!(f, "{table}")
    }
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
            shares: Asset::new("Shares", DividentProfit, output_papers),
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

    #[must_use]
    pub fn count_not_empty_assets(&self) -> usize {
        [
            self.bonds.is_empty(),
            self.shares.is_empty(),
            self.currencies.is_empty(),
            self.etfs.is_empty(),
            self.futures.is_empty(),
        ]
        .into_iter()
        .filter(|x| !x)
        .count()
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
            // IMPORTANT: We need absolute divident value here but current is absolute + balance
            // so we have to subtract
            acc += p.dividents().current - p.dividents().balance;
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
        asset_table.set_header([Cell::new(self.name)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        asset_table.set_style(TableComponent::HeaderLines, ' ');

        if self.output_papers {
            for p in &self.papers {
                asset_table.add_row([Cell::new(p)]);
            }
        }

        let mut table = ux::new_table();

        let title = format!("{} totals:", self.name);
        let title = Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header([title, Cell::new("")]);

        ux::add_row(&mut table, BALANCE_VALUE, self.balance());
        ux::add_row(&mut table, CURRENT_VALUE, self.current());
        ux::add_row_colorized(&mut table, BALANCE_INCOME, self.income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, P::name(), self.dividents());
        }

        ux::add_row(&mut table, "Instruments count", self.papers.len());
        asset_table.add_row([Cell::new(table)]);

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

        table.set_header([
            Cell::new(title).add_attribute(Attribute::Bold),
            Cell::new(""),
        ]);

        ux::add_row(&mut table, "Average buy price", self.average_buy_price());
        ux::add_row(
            &mut table,
            "Last instrument price",
            self.current_instrument_price(),
        );
        ux::add_row(
            &mut table,
            "Current items count",
            self.quantity().round_dp(2),
        );
        ux::add_row(&mut table, BALANCE_VALUE, self.balance());
        ux::add_row(&mut table, CURRENT_VALUE, self.current());
        table.add_row(["", ""]);

        ux::add_row_colorized(&mut table, INCOME, self.income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, P::name(), self.dividents());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
        }

        ux::add_row_colorized(&mut table, "Taxes and fees", self.fees());

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

        if self.count_not_empty_assets() > 1 {
            let mut table = ux::new_table();

            let title = Cell::new("Portfolio totals:")
                .add_attribute(Attribute::Bold)
                .fg(comfy_table::Color::DarkRed);
            table.set_header([title, Cell::new("")]);

            ux::add_row_colorized(&mut table, BALANCE_INCOME, self.income());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, "Dividents and coupons", self.dividents());

            ux::add_row(&mut table, BALANCE_VALUE, self.balance());
            ux::add_row(&mut table, CURRENT_VALUE, self.current());

            writeln!(f)?;
            writeln!(f, "{table}")
        } else {
            writeln!(f)
        }
    }
}

impl Display for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut history_table = ux::new_table();

        let currency = self.currency.code().to_owned();
        let title = format!(
            "{} ({} | {} | {})",
            self.name, self.ticker, self.figi, currency
        );

        history_table.set_header([Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        history_table.set_style(TableComponent::HeaderLines, ' ');

        let mut items_table = ux::new_table();

        let date = Cell::new("Date").add_attribute(Attribute::Bold);
        let quantity = Cell::new("Quantity").add_attribute(Attribute::Bold);
        let price = Cell::new("Price").add_attribute(Attribute::Bold);
        let payment = Cell::new("Payment").add_attribute(Attribute::Bold);
        let description = Cell::new("Description").add_attribute(Attribute::Bold);
        let state = Cell::new("State").add_attribute(Attribute::Bold);
        items_table.set_header([date, quantity, price, payment, description, state]);

        for item in &self.items {
            items_table.add_row([
                Cell::new(item.datetime),
                Cell::new(item.quantity - item.quantity_rest),
                Cell::new(item.price),
                Cell::new(item.payment),
                Cell::new(&item.description),
                Cell::new(item.operation_state),
            ]);
        }

        history_table.add_row([Cell::new(items_table)]);
        write!(f, "{history_table}")?;

        let mut table = ux::new_table();

        let title = Cell::new("Totals")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header([title, Cell::new("")]);

        ux::add_row_colorized(&mut table, "Expenses", self.expenses());
        ux::add_row_colorized(&mut table, "Profit", self.profit());
        ux::add_row_colorized(&mut table, "Balance", self.balance());
        write!(f, "{table}")
    }
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
                additional_profit: Money::from_value(dec!(100), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: CouponProfit,
        });
        let mut shares = Asset::new("Shares", DividentProfit, true);
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
                additional_profit: Money::from_value(dec!(50), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: DividentProfit,
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
