//! Passive income forecast: expected coupons and dividends by month and the portfolio yield.

use chrono::{DateTime, Datelike, Months, NaiveDate, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::money::Money;

/// Kind of an expected income payment
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncomeKind {
    Coupon,
    /// Dividend declared by the issuer
    Dividend,
    /// Dividend expected as the one paid a year before
    EstimatedDividend,
}

/// Expected income payment of a position, in RUB
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IncomeItem {
    pub date: DateTime<Utc>,
    pub kind: IncomeKind,
    pub amount: Money,
}

/// Dividend per share, declared or already paid
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DividendRecord {
    pub date: DateTime<Utc>,
    pub per_share: Money,
}

/// Dividends of `quantity` shares expected from `from` until `until`, days inclusive.
///
/// Declared dividends are taken as is. Dividends paid within the year before `from`
/// are expected again a year later, except the earliest ones replaced by declared dividends:
/// an issuer usually declares its nearest payments only. Dividends of unknown amount are skipped.
#[must_use]
pub fn expected_dividends(
    records: &[DividendRecord],
    quantity: Decimal,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Vec<IncomeItem> {
    let (start, end) = (from.date_naive(), until.date_naive());
    let contains = |date: DateTime<Utc>| (start..=end).contains(&date.date_naive());
    let item = |date, kind, record: &DividendRecord| IncomeItem {
        date,
        kind,
        amount: record.per_share * quantity,
    };
    let known = records.iter().filter(|r| !r.per_share.value.is_zero());

    let declared: Vec<IncomeItem> = known
        .clone()
        .filter(|r| contains(r.date))
        .map(|r| item(r.date, IncomeKind::Dividend, r))
        .collect();

    let mut repeated: Vec<IncomeItem> = known
        .filter(|r| r.date.date_naive() < start)
        .filter_map(|r| {
            let date = r.date.checked_add_months(Months::new(12))?;
            contains(date).then(|| item(date, IncomeKind::EstimatedDividend, r))
        })
        .collect();
    repeated.sort_by_key(|i| i.date);

    let replaced = declared.len();
    declared
        .into_iter()
        .chain(repeated.into_iter().skip(replaced))
        .collect()
}

/// Income by its kinds, in RUB
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IncomeTotals {
    pub coupons: Money,
    pub dividends: Money,
    pub estimated_dividends: Money,
}

impl IncomeTotals {
    fn zero() -> Self {
        Self {
            coupons: Money::zero(Currency::RUB),
            dividends: Money::zero(Currency::RUB),
            estimated_dividends: Money::zero(Currency::RUB),
        }
    }

    fn add(&mut self, item: &IncomeItem) {
        match item.kind {
            IncomeKind::Coupon => self.coupons += item.amount,
            IncomeKind::Dividend => self.dividends += item.amount,
            IncomeKind::EstimatedDividend => self.estimated_dividends += item.amount,
        }
    }

    /// Sum of all kinds of income
    #[must_use]
    pub fn total(&self) -> Money {
        self.coupons + self.dividends + self.estimated_dividends
    }
}

/// Income expected within a calendar month
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonthIncome {
    pub year: i32,
    pub month: u32,
    pub income: IncomeTotals,
}

/// Passive income expected within a period, by month
pub struct IncomeForecast {
    /// Every month of the period, including months without payments
    pub months: Vec<MonthIncome>,
    /// Current value of the whole portfolio, in RUB
    pub portfolio_value: Money,
}

impl IncomeForecast {
    /// Groups `items` by month from `from` until `until`; items outside the months are ignored.
    #[must_use]
    pub fn new(
        from: DateTime<Utc>,
        until: DateTime<Utc>,
        items: &[IncomeItem],
        portfolio_value: Money,
    ) -> Self {
        let first = month_start(from.date_naive());
        let last = month_start(until.date_naive());
        let mut months: Vec<MonthIncome> =
            std::iter::successors(first, |m| m.checked_add_months(Months::new(1)))
                .take_while(|m| last.is_some_and(|last| *m <= last))
                .map(|m| MonthIncome {
                    year: m.year(),
                    month: m.month(),
                    income: IncomeTotals::zero(),
                })
                .collect();
        for item in items {
            let (year, month) = (item.date.year(), item.date.month());
            if let Some(m) = months
                .iter_mut()
                .find(|m| m.year == year && m.month == month)
            {
                m.income.add(item);
            }
        }
        Self {
            months,
            portfolio_value,
        }
    }

    /// Income of the whole period
    #[must_use]
    pub fn total(&self) -> IncomeTotals {
        self.months
            .iter()
            .fold(IncomeTotals::zero(), |acc, m| IncomeTotals {
                coupons: acc.coupons + m.income.coupons,
                dividends: acc.dividends + m.income.dividends,
                estimated_dividends: acc.estimated_dividends + m.income.estimated_dividends,
            })
    }

    /// Average income per month of a year
    #[must_use]
    pub fn monthly_average(&self) -> Money {
        self.total().total() / dec!(12)
    }

    /// Income of the period relative to the portfolio value; `None` for an empty portfolio.
    #[must_use]
    pub fn current_yield(&self) -> Option<Decimal> {
        self.total()
            .total()
            .value
            .checked_div(self.portfolio_value.value)
    }
}

fn month_start(date: NaiveDate) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::rstest;

    use super::*;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn rub(value: Decimal) -> Money {
        Money::from_value(value, Currency::RUB)
    }

    fn record(date: DateTime<Utc>, per_share: Decimal) -> DividendRecord {
        DividendRecord {
            date,
            per_share: rub(per_share),
        }
    }

    fn income(date: DateTime<Utc>, kind: IncomeKind, amount: Decimal) -> IncomeItem {
        IncomeItem {
            date,
            kind,
            amount: rub(amount),
        }
    }

    #[test]
    fn expected_dividends_repeats_last_year_ones() {
        // Arrange
        let records = [
            record(at(2024, 10, 1), dec!(1)),
            record(at(2025, 4, 1), dec!(3)),
            record(at(2025, 10, 1), dec!(2)),
        ];

        // Act
        let items = expected_dividends(&records, dec!(10), at(2026, 1, 1), at(2027, 1, 1));

        // Assert: October 2024 is older than a year before
        assert_eq!(
            items,
            vec![
                income(at(2026, 4, 1), IncomeKind::EstimatedDividend, dec!(30)),
                income(at(2026, 10, 1), IncomeKind::EstimatedDividend, dec!(20)),
            ]
        );
    }

    #[test]
    fn expected_dividends_replaces_earliest_repeated_by_declared() {
        // Arrange: paid twice last year, the next one is declared at another date
        let records = [
            record(at(2025, 5, 10), dec!(5)),
            record(at(2025, 11, 10), dec!(4)),
            record(at(2026, 5, 20), dec!(6)),
        ];

        // Act
        let items = expected_dividends(&records, dec!(1), at(2026, 1, 1), at(2027, 1, 1));

        // Assert
        assert_eq!(
            items,
            vec![
                income(at(2026, 5, 20), IncomeKind::Dividend, dec!(6)),
                income(at(2026, 11, 10), IncomeKind::EstimatedDividend, dec!(4)),
            ]
        );
    }

    #[rstest]
    #[case::unknown_amount(vec![record(at(2026, 5, 1), dec!(0))])]
    #[case::after_period(vec![record(at(2027, 2, 1), dec!(5))])]
    #[case::no_dividends(vec![])]
    fn expected_dividends_none(#[case] records: Vec<DividendRecord>) {
        // Act
        let items = expected_dividends(&records, dec!(1), at(2026, 1, 1), at(2027, 1, 1));

        // Assert
        assert!(items.is_empty());
    }

    #[test]
    fn expected_dividends_unknown_declared_amount_keeps_repeated() {
        // Arrange
        let records = [
            record(at(2025, 6, 1), dec!(5)),
            record(at(2026, 6, 1), dec!(0)),
        ];

        // Act
        let items = expected_dividends(&records, dec!(1), at(2026, 1, 1), at(2027, 1, 1));

        // Assert
        assert_eq!(
            items,
            vec![income(
                at(2026, 6, 1),
                IncomeKind::EstimatedDividend,
                dec!(5)
            )]
        );
    }

    #[test]
    fn forecast_groups_income_by_month() {
        // Arrange
        let items = [
            income(at(2026, 9, 30), IncomeKind::Coupon, dec!(10)),
            income(at(2026, 9, 26), IncomeKind::Dividend, dec!(20)),
            income(at(2027, 1, 15), IncomeKind::EstimatedDividend, dec!(30)),
            income(at(2028, 1, 15), IncomeKind::Coupon, dec!(1000)),
        ];

        // Act
        let forecast =
            IncomeForecast::new(at(2026, 9, 26), at(2027, 9, 26), &items, rub(dec!(600)));

        // Assert
        assert_eq!(forecast.months.len(), 13);
        let first = forecast.months[0];
        assert_eq!((first.year, first.month), (2026, 9));
        assert_eq!(first.income.total(), rub(dec!(30)));
        let january = forecast.months[4];
        assert_eq!((january.year, january.month), (2027, 1));
        assert_eq!(january.income.estimated_dividends, rub(dec!(30)));
        assert_eq!(forecast.months[1].income.total(), rub(dec!(0)));
    }

    #[test]
    fn forecast_totals_and_yield() {
        // Arrange
        let items = [
            income(at(2026, 10, 1), IncomeKind::Coupon, dec!(40)),
            income(at(2026, 11, 1), IncomeKind::Dividend, dec!(50)),
            income(at(2027, 3, 1), IncomeKind::EstimatedDividend, dec!(30)),
        ];

        // Act
        let forecast =
            IncomeForecast::new(at(2026, 9, 26), at(2027, 9, 26), &items, rub(dec!(1200)));

        // Assert
        let total = forecast.total();
        assert_eq!(total.coupons, rub(dec!(40)));
        assert_eq!(total.dividends, rub(dec!(50)));
        assert_eq!(total.estimated_dividends, rub(dec!(30)));
        assert_eq!(total.total(), rub(dec!(120)));
        assert_eq!(forecast.monthly_average(), rub(dec!(10)));
        assert_eq!(forecast.current_yield(), Some(dec!(0.1)));
    }

    #[test]
    fn forecast_of_empty_portfolio_has_no_yield() {
        // Act
        let forecast = IncomeForecast::new(at(2026, 9, 26), at(2027, 9, 26), &[], rub(dec!(0)));

        // Assert
        assert_eq!(forecast.current_yield(), None);
        assert_eq!(forecast.total().total(), rub(dec!(0)));
    }
}
