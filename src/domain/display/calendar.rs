use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::{DateTime, Datelike, Utc};
use comfy_table::{Attribute, Cell, Table};
use iso_currency::Currency;

use crate::ux;

use super::super::calendar::CalendarPayment;
use super::super::money::Money;
use super::super::{CouponCalendar, DividendCalendar};
use crate::domain::calendar::{Calendar, CombinedCalendar};

fn format_date(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d").to_string()
}

fn month_name(month: u32) -> &'static str {
    let Ok(month) = u8::try_from(month) else {
        return "Unknown";
    };
    if let Ok(m) = chrono::Month::try_from(month) {
        m.name()
    } else {
        "Unknown"
    }
}

/// Groups payments by year and month, then sorts chronologically
fn group_and_sort_payments<P: CalendarPayment>(
    upcoming: &[P],
) -> BTreeMap<i32, BTreeMap<u32, Vec<&P>>> {
    let mut grouped: BTreeMap<(i32, u32), Vec<&P>> = BTreeMap::new();
    for payment in upcoming {
        let date = payment.payment_date();
        grouped
            .entry((date.year(), date.month()))
            .or_default()
            .push(payment);
    }

    let mut by_year: BTreeMap<i32, BTreeMap<u32, Vec<&P>>> = BTreeMap::new();
    for ((year, month), payments) in grouped {
        by_year.entry(year).or_default().insert(month, payments);
    }
    by_year
}

/// Creates a year header row in the calendar table
fn add_year_header(table: &mut Table, year: i32) {
    table.add_row([
        Cell::new(format!("{year}"))
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkCyan),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
    ]);
}

/// Creates a month header row in the calendar table
fn add_month_header(table: &mut Table, month_name_str: &str) {
    table.add_row([
        Cell::new(month_name_str).add_attribute(Attribute::Bold),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
    ]);
}

/// Color of bond redemption rows: they return the nominal and are not income
const REDEMPTION_COLOR: comfy_table::Color = comfy_table::Color::DarkGrey;

/// Income and bond redemptions of a period, summed separately
struct PeriodTotals {
    income: Money,
    redemptions: Money,
}

impl PeriodTotals {
    fn new() -> Self {
        Self {
            income: Money::zero(Currency::RUB),
            redemptions: Money::zero(Currency::RUB),
        }
    }

    fn add<P: CalendarPayment>(&mut self, payment: &P) {
        if payment.is_redemption() {
            self.redemptions += payment.total_payment();
        } else {
            self.income += payment.total_payment();
        }
    }

    fn add_totals(&mut self, other: &Self) {
        self.income += other.income;
        self.redemptions += other.redemptions;
    }
}

/// Adds a payment row to the calendar table; redemptions are grey
fn add_payment_row<P: CalendarPayment>(table: &mut Table, payment: &P) {
    let cells = [
        format_date(payment.payment_date()),
        format_date(payment.ex_date()),
        payment.title(),
        payment.payment_per_unit().to_string(),
        payment.total_payment().to_string(),
    ]
    .map(|text| {
        let cell = Cell::new(text);
        if payment.is_redemption() {
            cell.fg(REDEMPTION_COLOR)
        } else {
            cell
        }
    });
    table.add_row(cells);
}

/// Adds a grey row with the redemptions of a period when there are any
fn add_redemptions_total(table: &mut Table, label: String, total: Money) {
    if total.value.is_zero() {
        return;
    }
    table.add_row([
        Cell::new(""),
        Cell::new(""),
        Cell::new(label).fg(REDEMPTION_COLOR),
        Cell::new(""),
        Cell::new(total.to_string()).fg(REDEMPTION_COLOR),
    ]);
}

/// Adds a month total row to the calendar table
fn add_month_total<P: CalendarPayment>(table: &mut Table, month_name_str: &str, total: Money) {
    table.add_row([
        Cell::new(""),
        Cell::new(""),
        Cell::new(P::month_label(month_name_str)).add_attribute(Attribute::Bold),
        Cell::new(""),
        Cell::new(total.to_string()).add_attribute(Attribute::Bold),
    ]);
}

/// Adds a year total row to the calendar table
fn add_year_total<P: CalendarPayment>(table: &mut Table, year: i32, total: Money) {
    table.add_row([
        Cell::new(""),
        Cell::new(""),
        Cell::new(P::year_label(year))
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow),
        Cell::new(""),
        Cell::new(total.to_string()).add_attribute(Attribute::Bold),
    ]);
}

/// Adds the grand total row to the calendar table
fn add_grand_total(table: &mut Table, total: Money) {
    table.add_row([
        Cell::new(""),
        Cell::new(""),
        Cell::new("Grand Total")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkRed),
        Cell::new(""),
        Cell::new(total.to_string())
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkGreen),
    ]);
}

/// Adds an empty separator row
fn add_separator_row(table: &mut Table) {
    table.add_row([
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
        Cell::new(""),
    ]);
}

/// Generic calendar Display implementation for any [`CalendarPayment`] type
pub(super) fn format_calendar<P: CalendarPayment>(upcoming: &[P]) -> String {
    let mut table = ux::new_table();

    // Add header
    let title = Cell::new(P::calendar_title())
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Add column headers
    let (payment_date_hdr, ex_date_hdr, company_hdr, per_unit_hdr, total_hdr) = P::column_headers();
    table.add_row([
        Cell::new(payment_date_hdr).add_attribute(Attribute::Bold),
        Cell::new(ex_date_hdr).add_attribute(Attribute::Bold),
        Cell::new(company_hdr).add_attribute(Attribute::Bold),
        Cell::new(per_unit_hdr).add_attribute(Attribute::Bold),
        Cell::new(total_hdr).add_attribute(Attribute::Bold),
    ]);

    if upcoming.is_empty() {
        table.add_row([
            Cell::new(P::empty_message()),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
        ]);
        return table.to_string();
    }

    let grouped = group_and_sort_payments(upcoming);

    let mut grand_total = PeriodTotals::new();

    for year in grouped.keys() {
        let Some(months) = grouped.get(year) else {
            continue;
        };

        add_year_header(&mut table, *year);

        let mut year_total = PeriodTotals::new();

        for month in months.keys() {
            let Some(payments) = months.get(month) else {
                continue;
            };

            let month_name_str = month_name(*month);
            add_month_header(&mut table, month_name_str);

            let mut month_total = PeriodTotals::new();

            for payment in payments {
                add_payment_row(&mut table, *payment);
                month_total.add(*payment);
            }

            add_month_total::<P>(&mut table, month_name_str, month_total.income);
            add_redemptions_total(
                &mut table,
                format!("Month {month_name_str} Redemptions:"),
                month_total.redemptions,
            );

            year_total.add_totals(&month_total);
        }

        add_year_total::<P>(&mut table, *year, year_total.income);
        add_redemptions_total(
            &mut table,
            format!("Year {year} Redemptions:"),
            year_total.redemptions,
        );
        add_separator_row(&mut table);
        grand_total.add_totals(&year_total);
    }

    add_grand_total(&mut table, grand_total.income);
    add_redemptions_total(
        &mut table,
        "Redemptions Total".to_string(),
        grand_total.redemptions,
    );

    table.to_string()
}

impl Display for DividendCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format_calendar(&self.upcoming))
    }
}

impl Display for CouponCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format_calendar(&self.upcoming))
    }
}

impl Display for CombinedCalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format_calendar(&self.upcoming))
    }
}

impl Display for Calendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dividends(c) => c.fmt(f),
            Self::Coupons(c) => c.fmt(f),
            Self::Combined(c) => c.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::super::super::DividendCalendar;
    use super::super::super::calendar::DividendPayment;
    use super::super::super::money::Money;

    use super::super::super::paper::{Figi, Ticker};

    #[test]
    fn calendar_grouping_sorts_correctly() {
        let payments = vec![
            DividendPayment {
                figi: Figi::new("1".to_string()),
                ticker: Ticker::new("A".to_string()),
                name: "A".to_string(),
                currency: Currency::RUB,
                dividend_per_share: Money::from_value(dec!(1), Currency::RUB),
                total_dividend: Money::from_value(dec!(10), Currency::RUB),
                quantity: dec!(10),
                ex_dividend_date: Utc.with_ymd_and_hms(2025, 12, 1, 0, 0, 0).unwrap(),
                payment_date: None,
                dividend_type: "type".to_string(),
            },
            DividendPayment {
                figi: Figi::new("2".to_string()),
                ticker: Ticker::new("B".to_string()),
                name: "B".to_string(),
                currency: Currency::RUB,
                dividend_per_share: Money::from_value(dec!(2), Currency::RUB),
                total_dividend: Money::from_value(dec!(20), Currency::RUB),
                quantity: dec!(10),
                ex_dividend_date: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                payment_date: None,
                dividend_type: "type".to_string(),
            },
        ];
        let calendar = DividendCalendar { upcoming: payments };
        let output = format!("{calendar}");
        let pos_2024 = output.find("2024").unwrap();
        let pos_2025 = output.find("2025").unwrap();
        assert!(pos_2024 < pos_2025);
    }

    #[test]
    fn calendar_displays_its_kind() {
        use crate::domain::calendar::{Calendar, CombinedCalendar, CouponCalendar};

        // Arrange
        let calendars = [
            (
                Calendar::Dividends(DividendCalendar { upcoming: vec![] }),
                "Dividend Calendar",
            ),
            (
                Calendar::Coupons(CouponCalendar { upcoming: vec![] }),
                "Bond Payments Calendar",
            ),
            (
                Calendar::Combined(CombinedCalendar { upcoming: vec![] }),
                "Payments Calendar",
            ),
        ];

        for (calendar, title) in calendars {
            // Act
            let output = calendar.to_string();

            // Assert
            assert!(output.contains(title), "{output}");
        }
    }

    fn bond_payment(
        kind: crate::domain::bond::BondEventKind,
        day: u32,
        total: rust_decimal::Decimal,
    ) -> crate::domain::calendar::CouponPayment {
        use super::super::super::money::Money;

        crate::domain::calendar::CouponPayment {
            figi: Figi::new("2".to_string()),
            ticker: Ticker::new("BOND".to_string()),
            name: "OFZ Bond".to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(total / dec!(10), Currency::RUB),
            total_coupon: Money::from_value(total, Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2027, 2, day, 0, 0, 0).unwrap(),
            kind,
        }
    }

    /// Text of the total column in the row whose name column is `label`.
    fn total_of(output: &str, label: &str) -> Option<String> {
        output
            .lines()
            .find(|line| line.contains(label))
            .and_then(|line| line.split_whitespace().rev().nth(1).map(str::to_string))
    }

    #[test]
    fn bond_calendar_totals_income_without_redemptions() {
        use crate::domain::bond::BondEventKind;

        // Arrange
        let calendar = crate::domain::CouponCalendar {
            upcoming: vec![
                bond_payment(BondEventKind::Coupon, 3, dec!(300)),
                bond_payment(BondEventKind::Amortization, 3, dec!(2500)),
                bond_payment(BondEventKind::Maturity, 3, dec!(7500)),
            ],
        };

        // Act
        let output = calendar.to_string();

        // Assert
        assert_eq!(total_of(&output, "Grand Total").as_deref(), Some("300"));
        assert_eq!(
            total_of(&output, "Month February Total:").as_deref(),
            Some("300")
        );
        assert!(output.contains("Month February Redemptions:"));
        assert!(output.contains("Year 2027 Redemptions:"));
        assert!(output.contains("Redemptions Total"));
        assert!(output.contains("OFZ Bond · Maturity"));
        assert!(output.contains("OFZ Bond · Amortization"));
    }

    #[test]
    fn bond_calendar_without_redemptions_has_no_redemption_totals() {
        use crate::domain::bond::BondEventKind;

        // Arrange
        let calendar = crate::domain::CouponCalendar {
            upcoming: vec![bond_payment(BondEventKind::Coupon, 3, dec!(300))],
        };

        // Act
        let output = calendar.to_string();

        // Assert
        assert!(!output.contains("Redemptions"));
    }

    #[test]
    fn combined_calendar_empty() {
        use crate::domain::calendar::CombinedCalendar;

        let calendar = CombinedCalendar { upcoming: vec![] };
        let output = format!("{calendar}");
        assert!(output.contains("Payments Calendar"));
        assert!(output.contains("No upcoming dividend or coupon payments"));
    }

    #[test]
    fn combined_calendar_merges_dividends_and_coupons() {
        use super::super::super::calendar::DividendPayment;
        use super::super::super::money::Money;
        use crate::domain::calendar::{CombinedCalendar, CombinedPayment, CouponPayment};

        let dividend = DividendPayment {
            figi: Figi::new("1".to_string()),
            ticker: Ticker::new("SBER".to_string()),
            name: "Sberbank".to_string(),
            currency: Currency::RUB,
            dividend_per_share: Money::from_value(dec!(10), Currency::RUB),
            total_dividend: Money::from_value(dec!(100), Currency::RUB),
            quantity: dec!(10),
            ex_dividend_date: Utc.with_ymd_and_hms(2025, 3, 15, 0, 0, 0).unwrap(),
            payment_date: None,
            dividend_type: "Regular".to_string(),
        };

        let coupon = CouponPayment {
            figi: Figi::new("2".to_string()),
            ticker: Ticker::new("BOND".to_string()),
            name: "OFZ Bond".to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(dec!(5), Currency::RUB),
            total_coupon: Money::from_value(dec!(50), Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
            kind: crate::domain::bond::BondEventKind::Coupon,
        };

        let calendar = CombinedCalendar {
            upcoming: vec![
                CombinedPayment::Dividend(dividend),
                CombinedPayment::Coupon(coupon),
            ],
        };

        let output = format!("{calendar}");

        // Check title
        assert!(output.contains("Payments Calendar"));

        // Check both payments are present
        assert!(output.contains("Sberbank"));
        assert!(output.contains("OFZ Bond"));

        // Check sorting (coupon date is earlier)
        let pos_coupon = output.find("2025-02-01").unwrap();
        let pos_dividend = output.find("2025-03-15").unwrap();
        assert!(pos_coupon < pos_dividend);
    }

    #[test]
    fn combined_calendar_sorts_by_payment_date() {
        use super::super::super::calendar::DividendPayment;
        use super::super::super::money::Money;
        use crate::domain::calendar::{CombinedCalendar, CombinedPayment, CouponPayment};

        // Dividend with earlier date
        let dividend = DividendPayment {
            figi: Figi::new("1".to_string()),
            ticker: Ticker::new("SBER".to_string()),
            name: "Sberbank".to_string(),
            currency: Currency::RUB,
            dividend_per_share: Money::from_value(dec!(10), Currency::RUB),
            total_dividend: Money::from_value(dec!(100), Currency::RUB),
            quantity: dec!(10),
            ex_dividend_date: Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap(),
            payment_date: None,
            dividend_type: "Regular".to_string(),
        };

        // Coupon with later date
        let coupon = CouponPayment {
            figi: Figi::new("2".to_string()),
            ticker: Ticker::new("BOND".to_string()),
            name: "OFZ Bond".to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(dec!(5), Currency::RUB),
            total_coupon: Money::from_value(dec!(50), Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap(),
            kind: crate::domain::bond::BondEventKind::Coupon,
        };

        let calendar = CombinedCalendar {
            upcoming: vec![
                CombinedPayment::Coupon(coupon.clone()),
                CombinedPayment::Dividend(dividend.clone()),
            ],
        };

        let output = format!("{calendar}");

        // Check that dividend (earlier date) appears before coupon (later date)
        let pos_dividend = output.find("2025-01-15").unwrap();
        let pos_coupon = output.find("2025-06-01").unwrap();
        assert!(pos_dividend < pos_coupon);
    }
}
