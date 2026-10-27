//! Tax lots of securities by FIFO and the tax due if they were sold now.
//!
//! The broker matches sales with the earliest buys of the same security within an account,
//! so the cost of a sale is the cost of the lots it closes. Lots held for more than three
//! years since 2014 are exempt by the long-term holding benefit (ЛДВ) up to its limit.

use chrono::{DateTime, Datelike, Months, NaiveDate, TimeDelta, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::money::Money;
use super::moscow_date;

/// Years a lot must be held over to be exempt.
const LDV_YEARS: u32 = 3;
/// Lots bought before this year are not exempt.
const LDV_FIRST_YEAR: i32 = 2014;
/// Exempt result per full year of holding.
const LDV_LIMIT_PER_YEAR: Decimal = dec!(3_000_000);

/// Investment income of a year up to which [`BASE_RATE`] applies.
const HIGH_RATE_FROM: Decimal = dec!(2_400_000);
const BASE_RATE: Decimal = dec!(0.13);
const HIGH_RATE: Decimal = dec!(0.15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeKind {
    Buy,
    Sell,
    /// Partial redemption of a bond nominal that returns a part of the cost of every lot
    Repayment,
}

/// Trade of a security in RUB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trade {
    pub date: DateTime<Utc>,
    pub kind: TradeKind,
    /// Units bought or sold; not used by a repayment
    pub quantity: Decimal,
    /// Cost of a buy, commission and accrued interest included; proceeds of a sale
    /// net of commission; money received by a repayment. Positive.
    pub amount: Money,
}

/// Units bought at once that are not sold yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lot {
    pub bought: DateTime<Utc>,
    pub quantity: Decimal,
    /// Cost of the units of the lot, in RUB
    pub cost: Money,
}

/// Sale with the parts of lots it closed, priced at its proceeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RealizedSale {
    pub date: DateTime<Utc>,
    pub lots: Vec<LotSale>,
}

/// Lots of a security left after its trades are matched by FIFO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LotBook {
    /// Open lots, the earliest first
    pub lots: Vec<Lot>,
    pub realized: Vec<RealizedSale>,
    /// `false` when a sale had no lots to close, e.g. of units transferred from another broker
    pub complete: bool,
}

impl LotBook {
    /// Matches `trades` in date order: a sale closes the earliest lots first.
    #[must_use]
    pub fn new(trades: impl IntoIterator<Item = Trade>) -> Self {
        let mut trades: Vec<Trade> = trades.into_iter().collect();
        trades.sort_by_key(|t| t.date);
        let mut book = Self {
            lots: Vec::new(),
            realized: Vec::new(),
            complete: true,
        };
        for trade in trades {
            match trade.kind {
                TradeKind::Buy => book.lots.push(Lot {
                    bought: trade.date,
                    quantity: trade.quantity,
                    cost: trade.amount,
                }),
                TradeKind::Sell => book.sell(&trade),
                TradeKind::Repayment => book.repay(trade.amount),
            }
        }
        book
    }

    /// Units in open lots.
    #[must_use]
    pub fn quantity(&self) -> Decimal {
        self.lots.iter().map(|l| l.quantity).sum()
    }

    fn sell(&mut self, trade: &Trade) {
        let mut left = trade.quantity;
        let mut closed = Vec::new();
        while left > Decimal::ZERO {
            let Some(lot) = self.lots.first_mut() else {
                self.complete = false;
                break;
            };
            let taken = left.min(lot.quantity);
            let taken_cost = lot.cost * (taken / lot.quantity);
            closed.push(Lot {
                bought: lot.bought,
                quantity: taken,
                cost: taken_cost,
            });
            lot.cost -= taken_cost;
            lot.quantity -= taken;
            left -= taken;
            if lot.quantity.is_zero() {
                self.lots.remove(0);
            }
        }
        if closed.is_empty() {
            return;
        }
        let price = trade.amount / trade.quantity;
        let sold = moscow_date(trade.date);
        self.realized.push(RealizedSale {
            date: trade.date,
            lots: closed
                .iter()
                .map(|l| LotSale::new(l, price, sold))
                .collect(),
        });
    }

    /// Lowers the cost of every lot by its share of `amount`.
    fn repay(&mut self, amount: Money) {
        let quantity = self.quantity();
        if quantity.is_zero() {
            return;
        }
        for lot in &mut self.lots {
            lot.cost -= amount * (lot.quantity / quantity);
        }
    }
}

/// Lot priced as if it were sold on `sold` Moscow date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LotSale {
    /// Moscow date of the buy
    pub bought: NaiveDate,
    pub quantity: Decimal,
    pub cost: Money,
    pub value: Money,
    /// Full years the lot is held for
    pub years_held: u32,
    /// First date the lot is exempt on; `None` when it is never, being bought before 2014
    pub ldv_from: Option<NaiveDate>,
    /// Whether the lot is exempt on the sale date
    pub ldv: bool,
}

impl LotSale {
    /// Prices `lot` at `price` per unit on Moscow date `sold`.
    #[must_use]
    pub fn new(lot: &Lot, price: Money, sold: NaiveDate) -> Self {
        let bought = moscow_date(lot.bought);
        let ldv_from = ldv_from(bought);
        Self {
            bought,
            quantity: lot.quantity,
            cost: lot.cost,
            value: price * lot.quantity,
            years_held: full_years(bought, sold),
            ldv_from,
            ldv: ldv_from.is_some_and(|d| d <= sold),
        }
    }

    #[must_use]
    pub fn result(&self) -> Money {
        self.value - self.cost
    }

    /// The first `quantity` units of the lot, with cost and value in proportion.
    #[must_use]
    pub fn part(&self, quantity: Decimal) -> Self {
        let share = quantity / self.quantity;
        Self {
            quantity,
            cost: self.cost * share,
            value: self.value * share,
            ..*self
        }
    }
}

/// Day after the third anniversary of a buy, since a lot must be held over three years;
/// `None` for buys before 2014.
fn ldv_from(bought: NaiveDate) -> Option<NaiveDate> {
    if bought.year() < LDV_FIRST_YEAR {
        return None;
    }
    bought
        .checked_add_months(Months::new(12 * LDV_YEARS))?
        .checked_add_signed(TimeDelta::days(1))
}

/// Full years from `from` to `to`.
fn full_years(from: NaiveDate, to: NaiveDate) -> u32 {
    let years = to.year() - from.year();
    let anniversary_passed = (to.month(), to.day()) >= (from.month(), from.day());
    let years = if anniversary_passed { years } else { years - 1 };
    u32::try_from(years).unwrap_or(0)
}

/// Result of `lots` exempt by the long-term holding benefit.
///
/// The limit is 3 million RUB times the full years of holding averaged over
/// the exempt lots weighted by their value; the benefit cannot exceed their gain.
#[must_use]
pub fn ldv_exemption<'a>(lots: impl IntoIterator<Item = &'a LotSale>) -> Money {
    let mut value = Decimal::ZERO;
    let mut value_years = Decimal::ZERO;
    let mut gain = Decimal::ZERO;
    for lot in lots.into_iter().filter(|l| l.ldv) {
        value += lot.value.value;
        value_years += lot.value.value * Decimal::from(lot.years_held);
        gain += lot.result().value;
    }
    if value <= Decimal::ZERO || gain <= Decimal::ZERO {
        return rub(Decimal::ZERO);
    }
    let limit = LDV_LIMIT_PER_YEAR * value_years / value;
    rub(gain.min(limit))
}

fn sum_lots<'a>(
    lots: impl IntoIterator<Item = &'a LotSale>,
    amount: impl Fn(&LotSale) -> Money,
) -> Money {
    lots.into_iter()
        .fold(rub(Decimal::ZERO), |acc, l| acc + amount(l))
}

/// Sales of a security this year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecuritySales {
    pub ticker: String,
    /// Parts of lots the sales closed
    pub lots: Vec<LotSale>,
    /// `false` when some sales had no lots to close, so the cost of the others may be wrong
    pub complete: bool,
}

impl SecuritySales {
    #[must_use]
    pub fn quantity(&self) -> Decimal {
        self.lots.iter().map(|l| l.quantity).sum()
    }

    #[must_use]
    pub fn proceeds(&self) -> Money {
        sum_lots(&self.lots, |l| l.value)
    }

    #[must_use]
    pub fn cost(&self) -> Money {
        sum_lots(&self.lots, |l| l.cost)
    }

    #[must_use]
    pub fn result(&self) -> Money {
        sum_lots(&self.lots, LotSale::result)
    }

    /// Units sold from exempt lots.
    #[must_use]
    pub fn ldv_quantity(&self) -> Decimal {
        self.lots.iter().filter(|l| l.ldv).map(|l| l.quantity).sum()
    }
}

/// Investment income of the current year, in RUB.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YearIncome {
    pub sales: Vec<SecuritySales>,
    /// Dividends and coupons
    pub payments: Money,
    /// Tax withheld other than from dividends and coupons, mostly from sales
    pub withheld: Money,
}

impl YearIncome {
    #[must_use]
    pub fn zero() -> Self {
        Self {
            sales: Vec::new(),
            payments: rub(Decimal::ZERO),
            withheld: rub(Decimal::ZERO),
        }
    }

    fn sold(&self) -> impl Iterator<Item = &LotSale> {
        self.sales.iter().flat_map(|s| &s.lots)
    }

    /// Result of sales; a loss offsets only gains of securities.
    #[must_use]
    pub fn securities(&self) -> Money {
        sum_lots(self.sold(), LotSale::result)
    }

    /// Result of sales exempt by the long-term holding benefit.
    #[must_use]
    pub fn ldv_exemption(&self) -> Money {
        ldv_exemption(self.sold())
    }

    /// Tax on the result of sales.
    #[must_use]
    pub fn securities_tax(&self) -> Money {
        rub(self.tax_with(&[]) - income_tax(self.payments.value))
    }

    /// Tax the year grows by when `lots` are sold too; negative when their loss
    /// offsets gains of the sales.
    #[must_use]
    pub fn sale_tax(&self, lots: &[LotSale]) -> Money {
        rub(self.tax_with(lots) - self.tax_with(&[]))
    }

    /// Exemption the year grows by when `lots` are sold too.
    #[must_use]
    pub fn sale_ldv_exemption(&self, lots: &[LotSale]) -> Money {
        ldv_exemption(self.sold().chain(lots)) - self.ldv_exemption()
    }

    /// Tax on investment income of the year with `lots` sold too.
    fn tax_with(&self, lots: &[LotSale]) -> Decimal {
        let sold = || self.sold().chain(lots);
        let taxable = sum_lots(sold(), LotSale::result) - ldv_exemption(sold());
        income_tax(self.payments.value + taxable.value.max(Decimal::ZERO))
    }
}

/// Tax on investment income of a year: 13% up to 2.4 million RUB and 15% over it.
fn income_tax(base: Decimal) -> Decimal {
    let base = base.max(Decimal::ZERO);
    let high = (base - HIGH_RATE_FROM).max(Decimal::ZERO);
    (base - high) * BASE_RATE + high * HIGH_RATE
}

fn rub(value: Decimal) -> Money {
    Money::from_value(value, Currency::RUB)
}

/// Cost basis of a position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Basis {
    Lots(Vec<LotSale>),
    /// Trades on the account do not add up to the position, e.g. units were transferred
    /// from another broker or split
    Unknown {
        /// Units in open lots
        in_lots: Decimal,
    },
}

/// Portfolio position priced as if it were sold now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionSale {
    pub figi: String,
    pub name: String,
    pub ticker: String,
    pub quantity: Decimal,
    pub basis: Basis,
}

impl PositionSale {
    /// Prices open lots of `book` at `price` per unit on Moscow date `sold`;
    /// the basis is unknown when they do not add up to `quantity`.
    #[must_use]
    pub fn new(
        figi: String,
        name: String,
        ticker: String,
        quantity: Decimal,
        book: &LotBook,
        price: Money,
        sold: NaiveDate,
    ) -> Self {
        let in_lots = book.quantity();
        let basis = if book.complete && in_lots == quantity {
            Basis::Lots(
                book.lots
                    .iter()
                    .map(|l| LotSale::new(l, price, sold))
                    .collect(),
            )
        } else {
            Basis::Unknown { in_lots }
        };
        Self {
            figi,
            name,
            ticker,
            quantity,
            basis,
        }
    }

    /// Lots with known cost; empty when the basis is unknown.
    #[must_use]
    pub fn lots(&self) -> &[LotSale] {
        match &self.basis {
            Basis::Lots(lots) => lots,
            Basis::Unknown { .. } => &[],
        }
    }

    #[must_use]
    pub fn cost(&self) -> Money {
        self.sum(|l| l.cost)
    }

    #[must_use]
    pub fn value(&self) -> Money {
        self.sum(|l| l.value)
    }

    #[must_use]
    pub fn result(&self) -> Money {
        self.sum(LotSale::result)
    }

    /// Units in exempt lots.
    #[must_use]
    pub fn ldv_quantity(&self) -> Decimal {
        self.lots()
            .iter()
            .filter(|l| l.ldv)
            .map(|l| l.quantity)
            .sum()
    }

    /// Earliest date a lot that is not exempt yet becomes exempt and the units it adds.
    #[must_use]
    pub fn next_ldv(&self) -> Option<(NaiveDate, Decimal)> {
        let date = self
            .lots()
            .iter()
            .filter(|l| !l.ldv)
            .filter_map(|l| l.ldv_from)
            .min()?;
        let quantity = self
            .lots()
            .iter()
            .filter(|l| l.ldv_from == Some(date))
            .map(|l| l.quantity)
            .sum();
        Some((date, quantity))
    }

    fn sum(&self, amount: impl Fn(&LotSale) -> Money) -> Money {
        sum_lots(self.lots(), amount)
    }
}

/// Sale of the earliest lots of a position that saves the most tax by offsetting
/// gains of this year's sales.
pub struct Harvest<'a> {
    pub position: &'a PositionSale,
    pub quantity: Decimal,
    pub result: Money,
    /// Tax saved, positive
    pub saving: Money,
}

/// Tax due if portfolio securities were sold now.
pub struct SaleTaxReport {
    pub positions: Vec<PositionSale>,
    /// Investment income of the current year before the sale
    pub year_income: YearIncome,
    /// `false` for an individual investment account, which is not taxed on sales
    pub taxed: bool,
}

impl SaleTaxReport {
    /// Positions with known cost.
    pub fn known(&self) -> impl Iterator<Item = &PositionSale> {
        self.positions
            .iter()
            .filter(|p| matches!(p.basis, Basis::Lots(_)))
    }

    /// Positions with unknown cost.
    pub fn unknown(&self) -> impl Iterator<Item = &PositionSale> {
        self.positions
            .iter()
            .filter(|p| matches!(p.basis, Basis::Unknown { .. }))
    }

    #[must_use]
    pub fn cost(&self) -> Money {
        self.sum(PositionSale::cost)
    }

    #[must_use]
    pub fn value(&self) -> Money {
        self.sum(PositionSale::value)
    }

    #[must_use]
    pub fn result(&self) -> Money {
        self.sum(PositionSale::result)
    }

    /// Lots of all positions with known cost.
    fn known_lots(&self) -> Vec<LotSale> {
        self.known().flat_map(PositionSale::lots).copied().collect()
    }

    /// Result exempt in addition to this year's sales when all positions are sold.
    #[must_use]
    pub fn ldv_exemption(&self) -> Money {
        self.year_income.sale_ldv_exemption(&self.known_lots())
    }

    /// Tax due when all positions with known cost are sold together.
    #[must_use]
    pub fn tax(&self) -> Money {
        self.tax_of(&self.known_lots())
    }

    /// Tax due when only `position` is sold.
    #[must_use]
    pub fn position_tax(&self, position: &PositionSale) -> Money {
        self.tax_of(position.lots())
    }

    /// Tax on this year's sales; zero when sales are not taxed.
    #[must_use]
    pub fn year_tax(&self) -> Money {
        if self.taxed {
            self.year_income.securities_tax()
        } else {
            rub(Decimal::ZERO)
        }
    }

    /// Sales of positions that save tax by offsetting gains of this year's sales,
    /// the biggest saving first. Savings do not add up, since they offset the same gains.
    ///
    /// The broker sells the earliest lots first, so of every position the number of
    /// its earliest lots is picked whose sale saves the most.
    #[must_use]
    pub fn harvest(&self) -> Vec<Harvest<'_>> {
        let mut harvest: Vec<Harvest> = self
            .known()
            .filter_map(|position| {
                let lots = position.lots();
                let (count, tax) = (1..=lots.len())
                    .map(|count| (count, self.tax_of(&lots[..count])))
                    .min_by_key(|(_, tax)| tax.value)?;
                let sold = &lots[..count];
                (tax.value < Decimal::ZERO).then(|| Harvest {
                    position,
                    quantity: sold.iter().map(|l| l.quantity).sum(),
                    result: sum_lots(sold, LotSale::result),
                    saving: rub(-tax.value),
                })
            })
            .collect();
        harvest.sort_by_key(|h| std::cmp::Reverse(h.saving.value));
        harvest
    }

    fn tax_of(&self, lots: &[LotSale]) -> Money {
        if self.taxed {
            self.year_income.sale_tax(lots)
        } else {
            rub(Decimal::ZERO)
        }
    }

    fn sum(&self, amount: impl Fn(&PositionSale) -> Money) -> Money {
        self.known()
            .fold(rub(Decimal::ZERO), |acc, p| acc + amount(p))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn at(date: &str) -> DateTime<Utc> {
        format!("{date}T09:00:00Z").parse().unwrap()
    }

    fn day(date: &str) -> NaiveDate {
        date.parse().unwrap()
    }

    fn trade(date: &str, kind: TradeKind, quantity: i64, amount: i64) -> Trade {
        Trade {
            date: at(date),
            kind,
            quantity: Decimal::from(quantity),
            amount: rub(Decimal::from(amount)),
        }
    }

    fn lot_sale(bought: &str, cost: i64, value: i64, sold: &str) -> LotSale {
        let lot = Lot {
            bought: at(bought),
            quantity: Decimal::ONE,
            cost: rub(Decimal::from(cost)),
        };
        LotSale::new(&lot, rub(Decimal::from(value)), day(sold))
    }

    fn realized_result(sale: &RealizedSale) -> Decimal {
        sum_lots(&sale.lots, LotSale::result).value
    }

    #[test]
    fn lot_book_sale_closes_earliest_lots_first() {
        // Arrange
        let trades = [
            trade("2024-03-01", TradeKind::Buy, 10, 2000),
            trade("2023-01-10", TradeKind::Buy, 10, 1000),
            trade("2025-05-01", TradeKind::Sell, 15, 4500),
        ];

        // Act
        let book = LotBook::new(trades);

        // Assert
        assert!(book.complete);
        assert_eq!(book.lots.len(), 1);
        assert_eq!(book.lots[0].bought, at("2024-03-01"));
        assert_eq!(book.lots[0].quantity, dec!(5));
        assert_eq!(book.lots[0].cost.value, dec!(1000));
        assert_eq!(realized_result(&book.realized[0]), dec!(2500));
    }

    #[test]
    fn lot_book_sale_without_lots_is_incomplete() {
        // Arrange
        let trades = [
            trade("2024-01-10", TradeKind::Buy, 5, 500),
            trade("2025-01-10", TradeKind::Sell, 8, 1600),
        ];

        // Act
        let book = LotBook::new(trades);

        // Assert
        assert!(!book.complete);
        assert_eq!(book.quantity(), Decimal::ZERO);
        assert_eq!(realized_result(&book.realized[0]), dec!(500));
    }

    #[test]
    fn lot_book_repayment_lowers_cost_by_quantity_share() {
        // Arrange
        let trades = [
            trade("2024-01-10", TradeKind::Buy, 3, 3000),
            trade("2024-02-10", TradeKind::Buy, 1, 1000),
            trade("2024-06-10", TradeKind::Repayment, 0, 1000),
        ];

        // Act
        let book = LotBook::new(trades);

        // Assert
        let costs: Vec<Decimal> = book.lots.iter().map(|l| l.cost.value).collect();
        assert_eq!(costs, vec![dec!(2250), dec!(750)]);
    }

    #[test]
    fn lot_book_repayment_without_lots_is_ignored() {
        // Arrange
        let trades = [trade("2024-06-10", TradeKind::Repayment, 0, 1000)];

        // Act
        let book = LotBook::new(trades);

        // Assert
        assert_eq!(book.lots, Vec::new());
        assert!(book.complete);
    }

    #[rstest]
    #[case::third_anniversary("2022-03-15", "2025-03-15", false, 3)]
    #[case::day_after_third_anniversary("2022-03-15", "2025-03-16", true, 3)]
    #[case::day_before_anniversary("2020-03-15", "2025-03-14", true, 4)]
    #[case::recent("2025-01-01", "2025-10-01", false, 0)]
    #[case::before_2014("2013-12-31", "2025-10-01", false, 11)]
    #[case::first_day_of_2014("2014-01-01", "2025-10-01", true, 11)]
    fn lot_sale_ldv_after_three_years_since_2014(
        #[case] bought: &str,
        #[case] sold: &str,
        #[case] ldv: bool,
        #[case] years: u32,
    ) {
        // Act
        let sale = lot_sale(bought, 100, 150, sold);

        // Assert
        assert_eq!((sale.ldv, sale.years_held), (ldv, years));
    }

    #[test]
    fn lot_sale_ldv_from_leap_day() {
        // Act
        let sale = lot_sale("2024-02-29", 100, 150, "2025-10-01");

        // Assert
        assert_eq!(sale.ldv_from, Some(day("2027-03-01")));
    }

    #[test]
    fn ldv_exemption_is_gain_within_limit() {
        // Arrange
        let lots = [
            lot_sale("2020-01-10", 100, 300, "2025-10-01"),
            lot_sale("2025-01-10", 100, 50, "2025-10-01"),
        ];

        // Act
        let exempt = ldv_exemption(&lots);

        // Assert
        assert_eq!(exempt.value, dec!(200));
    }

    #[test]
    fn ldv_exemption_is_limited_by_weighted_years() {
        // Arrange: 5 and 3 full years weighted by equal values give a 12 million limit
        let lots = [
            lot_sale("2020-01-10", 1_000_000, 10_000_000, "2025-10-01"),
            lot_sale("2022-01-10", 1_000_000, 10_000_000, "2025-10-01"),
        ];

        // Act
        let exempt = ldv_exemption(&lots);

        // Assert
        assert_eq!(exempt.value, dec!(12_000_000));
    }

    #[test]
    fn ldv_exemption_of_loss_is_zero() {
        // Arrange
        let lots = [lot_sale("2020-01-10", 300, 100, "2025-10-01")];

        // Act
        let exempt = ldv_exemption(&lots);

        // Assert
        assert_eq!(exempt.value, Decimal::ZERO);
    }

    #[rstest]
    #[case::zero(dec!(0), dec!(0))]
    #[case::negative(dec!(-100), dec!(0))]
    #[case::base_rate(dec!(1_000_000), dec!(130_000))]
    #[case::threshold(dec!(2_400_000), dec!(312_000))]
    #[case::high_rate(dec!(3_400_000), dec!(462_000))]
    fn income_tax_is_progressive(#[case] base: Decimal, #[case] expected: Decimal) {
        // Act
        let tax = income_tax(base);

        // Assert
        assert_eq!(tax, expected);
    }

    /// Lot bought this year, not exempt, sold with `result`.
    fn recent(result: Decimal) -> LotSale {
        LotSale {
            value: rub(dec!(10_000) + result),
            ..lot_sale("2025-01-10", 10_000, 0, "2025-10-01")
        }
    }

    fn year(sold: &[LotSale], payments: Decimal) -> YearIncome {
        YearIncome {
            sales: vec![SecuritySales {
                ticker: "SBER".to_string(),
                lots: sold.to_vec(),
                complete: true,
            }],
            payments: rub(payments),
            withheld: rub(Decimal::ZERO),
        }
    }

    #[rstest]
    #[case::no_income(dec!(0), dec!(0), dec!(1000), dec!(130))]
    #[case::crosses_threshold(dec!(0), dec!(2_399_000), dec!(2000), dec!(280))]
    #[case::loss_offsets_gain(dec!(1000), dec!(0), dec!(-400), dec!(-52))]
    #[case::loss_beyond_gain(dec!(1000), dec!(500), dec!(-3000), dec!(-130))]
    #[case::loss_without_gain(dec!(0), dec!(500), dec!(-3000), dec!(0))]
    fn year_income_sale_tax(
        #[case] securities: Decimal,
        #[case] payments: Decimal,
        #[case] taxable: Decimal,
        #[case] expected: Decimal,
    ) {
        // Arrange
        let income = year(&[recent(securities)], payments);

        // Act
        let tax = income.sale_tax(&[recent(taxable)]);

        // Assert
        assert_eq!(tax.value, expected);
    }

    #[test]
    fn year_income_loss_does_not_offset_exempt_gain() {
        // Arrange
        let exempt = lot_sale("2020-01-10", 100, 1100, "2025-10-01");
        let income = year(&[exempt], dec!(0));

        // Act
        let tax = income.sale_tax(&[recent(dec!(-400))]);

        // Assert
        assert_eq!(income.ldv_exemption().value, dec!(1000));
        assert_eq!(tax.value, Decimal::ZERO);
    }

    #[test]
    fn year_income_shares_ldv_exemption_with_sale() {
        // Arrange
        let income = year(&[lot_sale("2020-01-10", 100, 1100, "2025-10-01")], dec!(0));
        let sale = [lot_sale("2021-01-10", 100, 600, "2025-10-01")];

        // Act
        let exemption = income.sale_ldv_exemption(&sale);

        // Assert
        assert_eq!(exemption.value, dec!(500));
    }

    #[test]
    fn year_income_securities_tax_is_over_payments() {
        // Arrange
        let income = year(&[recent(dec!(1000))], dec!(2_400_000));

        // Act
        let tax = income.securities_tax();

        // Assert
        assert_eq!(income.securities().value, dec!(1000));
        assert_eq!(tax.value, dec!(150));
    }

    fn position(trades: &[Trade], quantity: i64, price: i64) -> PositionSale {
        PositionSale::new(
            "FIGI".to_string(),
            "Sber".to_string(),
            "SBER".to_string(),
            Decimal::from(quantity),
            &LotBook::new(trades.iter().copied()),
            rub(Decimal::from(price)),
            day("2025-10-01"),
        )
    }

    #[test]
    fn position_sale_with_lots_matching_quantity() {
        // Arrange
        let trades = [
            trade("2020-01-10", TradeKind::Buy, 10, 1000),
            trade("2024-01-10", TradeKind::Buy, 10, 3000),
        ];

        // Act
        let sale = position(&trades, 20, 200);

        // Assert
        assert_eq!(sale.cost().value, dec!(4000));
        assert_eq!(sale.value().value, dec!(4000));
        assert_eq!(sale.result().value, dec!(0));
        assert_eq!(sale.ldv_quantity(), dec!(10));
        assert_eq!(sale.next_ldv(), Some((day("2027-01-11"), dec!(10))));
    }

    #[test]
    fn position_sale_basis_unknown_when_lots_differ_from_quantity() {
        // Arrange
        let trades = [trade("2024-01-10", TradeKind::Buy, 10, 1000)];

        // Act
        let sale = position(&trades, 30, 200);

        // Assert
        assert_eq!(sale.basis, Basis::Unknown { in_lots: dec!(10) });
        assert_eq!(sale.result().value, Decimal::ZERO);
        assert_eq!(sale.next_ldv(), None);
    }

    fn report(positions: Vec<PositionSale>, taxed: bool) -> SaleTaxReport {
        SaleTaxReport {
            positions,
            year_income: YearIncome::zero(),
            taxed,
        }
    }

    #[test]
    fn sale_tax_report_sums_known_positions_and_exempts_ldv() {
        // Arrange
        let positions = vec![
            position(&[trade("2020-01-10", TradeKind::Buy, 1, 100)], 1, 1100),
            position(&[trade("2025-01-10", TradeKind::Buy, 1, 100)], 1, 600),
            position(&[trade("2025-01-10", TradeKind::Buy, 1, 100)], 5, 600),
        ];

        // Act
        let report = report(positions, true);

        // Assert
        assert_eq!(report.result().value, dec!(1500));
        assert_eq!(report.ldv_exemption().value, dec!(1000));
        assert_eq!(report.tax().value, dec!(65));
        assert_eq!(report.unknown().count(), 1);
    }

    #[test]
    fn sale_tax_report_of_iis_is_not_taxed() {
        // Arrange
        let positions = vec![position(
            &[trade("2025-01-10", TradeKind::Buy, 1, 100)],
            1,
            600,
        )];

        // Act
        let report = report(positions, false);

        // Assert
        assert_eq!(report.result().value, dec!(500));
        assert_eq!(report.tax().value, Decimal::ZERO);
        assert_eq!(
            report.position_tax(&report.positions[0]).value,
            Decimal::ZERO
        );
    }

    #[test]
    fn sale_tax_report_position_tax_of_loss_is_saving() {
        // Arrange
        let positions = vec![position(
            &[trade("2025-01-10", TradeKind::Buy, 1, 1000)],
            1,
            600,
        )];
        let mut report = report(positions, true);
        report.year_income = year(&[recent(dec!(1000))], dec!(0));

        // Act
        let tax = report.position_tax(&report.positions[0]);

        // Assert
        assert_eq!(tax.value, dec!(-52));
    }

    fn harvest_report(taxed: bool) -> SaleTaxReport {
        let losing = position(
            &[
                trade("2025-01-10", TradeKind::Buy, 1, 1300),
                trade("2025-02-10", TradeKind::Buy, 1, 500),
                trade("2025-03-10", TradeKind::Buy, 1, 3000),
            ],
            3,
            1000,
        );
        let gaining = position(&[trade("2025-01-10", TradeKind::Buy, 1, 100)], 1, 1000);
        let mut report = report(vec![losing, gaining], taxed);
        report.year_income = year(&[recent(dec!(1000))], dec!(0));
        report
    }

    #[test]
    fn harvest_picks_earliest_lots_saving_most() {
        // Arrange
        let report = harvest_report(true);

        // Act
        let harvest = report.harvest();

        // Assert
        assert_eq!(harvest.len(), 1);
        assert_eq!(harvest[0].quantity, dec!(3));
        assert_eq!(harvest[0].result.value, dec!(-1800));
        assert_eq!(harvest[0].saving.value, dec!(130));
    }

    #[test]
    fn harvest_of_untaxed_account_is_empty() {
        // Arrange
        let report = harvest_report(false);

        // Act
        let harvest = report.harvest();

        // Assert
        assert!(harvest.is_empty());
    }
}
