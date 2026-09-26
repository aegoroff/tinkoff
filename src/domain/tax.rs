//! Taxes withheld by the broker and taxable income by year.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Utc};
use iso_currency::Currency;

use super::money::Money;

/// Kind of an operation that affects taxes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaxOperationKind {
    /// Dividend received
    Dividend,
    /// Coupon received
    Coupon,
    /// Tax withheld from dividends or its correction
    DividendTax,
    /// Tax withheld from coupons or its correction
    CouponTax,
    /// Tax withheld from sales, material benefit, REPO or its correction.
    /// The broker also withholds coupon tax this way at the year end since the middle of 2023
    OtherTax,
}

/// Operation that affects taxes; `amount` is its payment in RUB, negative when withheld
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaxOperation {
    pub date: DateTime<Utc>,
    pub kind: TaxOperationKind,
    pub amount: Money,
}

impl TaxOperation {
    /// Year the operation belongs to: other tax withheld on the 1st of January
    /// is the broker's annual tax calculation of the previous year.
    #[must_use]
    pub fn tax_year(&self) -> i32 {
        let year = self.date.year();
        let new_year = self.date.month() == 1 && self.date.day() == 1;
        if self.kind == TaxOperationKind::OtherTax && new_year {
            year - 1
        } else {
            year
        }
    }
}

/// Income and taxes of a year, in RUB; taxes are positive when withheld
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YearTaxes {
    pub year: i32,
    pub dividends: Money,
    pub coupons: Money,
    pub dividend_tax: Money,
    pub coupon_tax: Money,
    pub other_tax: Money,
}

impl YearTaxes {
    fn zero(year: i32) -> Self {
        let zero = Money::zero(Currency::RUB);
        Self {
            year,
            dividends: zero,
            coupons: zero,
            dividend_tax: zero,
            coupon_tax: zero,
            other_tax: zero,
        }
    }

    fn add(&mut self, operation: &TaxOperation) {
        match operation.kind {
            TaxOperationKind::Dividend => self.dividends += operation.amount,
            TaxOperationKind::Coupon => self.coupons += operation.amount,
            TaxOperationKind::DividendTax => self.dividend_tax -= operation.amount,
            TaxOperationKind::CouponTax => self.coupon_tax -= operation.amount,
            TaxOperationKind::OtherTax => self.other_tax -= operation.amount,
        }
    }

    fn add_year(&mut self, other: &Self) {
        self.dividends += other.dividends;
        self.coupons += other.coupons;
        self.dividend_tax += other.dividend_tax;
        self.coupon_tax += other.coupon_tax;
        self.other_tax += other.other_tax;
    }

    /// All taxes withheld
    #[must_use]
    pub fn total_tax(&self) -> Money {
        self.dividend_tax + self.coupon_tax + self.other_tax
    }
}

/// Income and taxes by year
pub struct TaxReport {
    /// Years with operations, ascending
    pub years: Vec<YearTaxes>,
}

impl TaxReport {
    /// Groups `operations` by their [`TaxOperation::tax_year`].
    #[must_use]
    pub fn new(operations: &[TaxOperation]) -> Self {
        let mut years: BTreeMap<i32, YearTaxes> = BTreeMap::new();
        for operation in operations {
            let year = operation.tax_year();
            years
                .entry(year)
                .or_insert_with(|| YearTaxes::zero(year))
                .add(operation);
        }
        Self {
            years: years.into_values().collect(),
        }
    }

    /// Sum of all years; its `year` is the latest one, or zero without operations
    #[must_use]
    pub fn total(&self) -> YearTaxes {
        let latest = self.years.last().map_or(0, |y| y.year);
        self.years
            .iter()
            .fold(YearTaxes::zero(latest), |mut acc, year| {
                acc.add_year(year);
                acc
            })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;

    fn rub(value: Decimal) -> Money {
        Money::from_value(value, Currency::RUB)
    }

    fn operation(year: i32, kind: TaxOperationKind, amount: Decimal) -> TaxOperation {
        operation_at(year, 6, 1, kind, amount)
    }

    fn operation_at(
        year: i32,
        month: u32,
        day: u32,
        kind: TaxOperationKind,
        amount: Decimal,
    ) -> TaxOperation {
        TaxOperation {
            date: Utc.with_ymd_and_hms(year, month, day, 3, 0, 0).unwrap(),
            kind,
            amount: rub(amount),
        }
    }

    #[rstest]
    #[case::annual_other_tax(1, 1, TaxOperationKind::OtherTax, 2025)]
    #[case::other_tax_in_january(1, 2, TaxOperationKind::OtherTax, 2026)]
    #[case::other_tax_on_new_year_eve(12, 31, TaxOperationKind::OtherTax, 2026)]
    #[case::coupon_tax_on_new_year(1, 1, TaxOperationKind::CouponTax, 2026)]
    #[case::dividend_on_new_year(1, 1, TaxOperationKind::Dividend, 2026)]
    fn tax_year_of_operation(
        #[case] month: u32,
        #[case] day: u32,
        #[case] kind: TaxOperationKind,
        #[case] expected: i32,
    ) {
        // Arrange
        let operation = operation_at(2026, month, day, kind, dec!(-100));

        // Act
        let year = operation.tax_year();

        // Assert
        assert_eq!(year, expected);
    }

    #[test]
    fn report_counts_annual_tax_in_previous_year() {
        // Arrange
        let operations = [
            operation(2025, TaxOperationKind::Coupon, dec!(1000)),
            operation_at(2026, 1, 1, TaxOperationKind::OtherTax, dec!(-130)),
        ];

        // Act
        let report = TaxReport::new(&operations);

        // Assert
        assert_eq!(report.years.len(), 1);
        assert_eq!(report.years[0].year, 2025);
        assert_eq!(report.years[0].other_tax, rub(dec!(130)));
    }

    #[test]
    fn report_groups_operations_by_year() {
        // Arrange
        let operations = [
            operation(2025, TaxOperationKind::Coupon, dec!(1000)),
            operation(2024, TaxOperationKind::Dividend, dec!(500)),
            operation(2025, TaxOperationKind::CouponTax, dec!(-130)),
        ];

        // Act
        let report = TaxReport::new(&operations);

        // Assert
        let years: Vec<i32> = report.years.iter().map(|y| y.year).collect();
        assert_eq!(years, vec![2024, 2025]);
        assert_eq!(report.years[0].dividends, rub(dec!(500)));
        assert_eq!(report.years[1].coupons, rub(dec!(1000)));
        assert_eq!(report.years[1].coupon_tax, rub(dec!(130)));
    }

    #[test]
    fn corrections_reduce_withheld_tax() {
        // Arrange
        let operations = [
            operation(2025, TaxOperationKind::OtherTax, dec!(-1000)),
            operation(2025, TaxOperationKind::OtherTax, dec!(300)),
        ];

        // Act
        let report = TaxReport::new(&operations);

        // Assert
        assert_eq!(report.years[0].other_tax, rub(dec!(700)));
    }

    #[test]
    fn year_total_tax() {
        // Arrange
        let operations = [
            operation(2025, TaxOperationKind::Dividend, dec!(870)),
            operation(2025, TaxOperationKind::DividendTax, dec!(-130)),
            operation(2025, TaxOperationKind::Coupon, dec!(1000)),
            operation(2025, TaxOperationKind::CouponTax, dec!(-130)),
            operation(2025, TaxOperationKind::OtherTax, dec!(-500)),
        ];

        // Act
        let year = TaxReport::new(&operations).years[0];

        // Assert
        assert_eq!(year.total_tax(), rub(dec!(760)));
    }

    #[test]
    fn total_sums_all_years() {
        // Arrange
        let operations = [
            operation(2024, TaxOperationKind::OtherTax, dec!(-100)),
            operation(2025, TaxOperationKind::OtherTax, dec!(-200)),
            operation(2025, TaxOperationKind::Coupon, dec!(50)),
        ];

        // Act
        let total = TaxReport::new(&operations).total();

        // Assert
        assert_eq!(total.year, 2025);
        assert_eq!(total.other_tax, rub(dec!(300)));
        assert_eq!(total.coupons, rub(dec!(50)));
    }

    #[test]
    fn empty_report() {
        // Act
        let report = TaxReport::new(&[]);

        // Assert
        assert!(report.years.is_empty());
        assert_eq!(report.total().total_tax(), rub(dec!(0)));
    }
}
