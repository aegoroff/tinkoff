use std::{collections::HashMap, fmt::Display};

use chrono::{DateTime, Datelike, Utc};
use comfy_table::{Attribute, Cell, Table};
use iso_currency::Currency;

use crate::ux;

use super::super::calendar::CalendarPayment;
use super::super::money::Money;
use super::super::{CouponCalendar, DividendCalendar};
use crate::domain::calendar::CombinedCalendar;

fn format_date(dt: DateTime<Utc>) -> String {
    format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
}

fn month_name(month: u32) -> &'static str {
    match month {
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
    }
}

/// Groups payments by year and month, then sorts chronologically
fn group_and_sort_payments<P: CalendarPayment>(
    upcoming: &[P],
) -> HashMap<i32, HashMap<u32, Vec<&P>>> {
    let mut grouped: HashMap<(i32, u32), Vec<&P>> = HashMap::new();
    for payment in upcoming {
        let date = payment.payment_date();
        grouped
            .entry((date.year(), date.month()))
            .or_default()
            .push(payment);
    }

    let mut by_year: HashMap<i32, HashMap<u32, Vec<&P>>> = HashMap::new();
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

/// Adds a payment row to the calendar table
fn add_payment_row<P: CalendarPayment>(table: &mut Table, payment: &P) {
    table.add_row([
        Cell::new(format_date(payment.payment_date())),
        Cell::new(format_date(payment.ex_date())),
        Cell::new(payment.name().to_string()),
        Cell::new(payment.payment_per_unit().to_string()),
        Cell::new(payment.total_payment().to_string()),
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

    let mut grand_total = Money::zero(Currency::RUB);

    // Collect and sort year keys
    let mut year_keys: Vec<i32> = grouped.keys().copied().collect();
    year_keys.sort_unstable();

    for year in year_keys {
        let Some(months) = grouped.get(&year) else {
            continue;
        };

        add_year_header(&mut table, year);

        let mut year_total = Money::zero(Currency::RUB);

        // Collect and sort month keys
        let mut month_keys: Vec<u32> = months.keys().copied().collect();
        month_keys.sort_unstable();

        for month in month_keys {
            let Some(payments) = months.get(&month) else {
                continue;
            };

            let month_name_str = month_name(month);
            add_month_header(&mut table, month_name_str);

            let mut month_total = Money::zero(Currency::RUB);

            for payment in payments {
                add_payment_row(&mut table, *payment);
                month_total += payment.total_payment();
            }

            add_month_total::<P>(&mut table, month_name_str, month_total);

            year_total += month_total;
            grand_total += month_total;
        }

        add_year_total::<P>(&mut table, year, year_total);
        add_separator_row(&mut table);
    }

    add_grand_total(&mut table, grand_total);

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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::super::super::DividendCalendar;
    use super::super::super::calendar::DividendPayment;
    use super::super::super::money::Money;

    #[test]
    fn calendar_grouping_sorts_correctly() {
        let payments = vec![
            DividendPayment {
                figi: "1".to_string(),
                ticker: "A".to_string(),
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
                figi: "2".to_string(),
                ticker: "B".to_string(),
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
            figi: "1".to_string(),
            ticker: "SBER".to_string(),
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
            figi: "2".to_string(),
            ticker: "BOND".to_string(),
            name: "OFZ Bond".to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(dec!(5), Currency::RUB),
            total_coupon: Money::from_value(dec!(50), Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
            coupon_type: "Constant".to_string(),
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
            figi: "1".to_string(),
            ticker: "SBER".to_string(),
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
            figi: "2".to_string(),
            ticker: "BOND".to_string(),
            name: "OFZ Bond".to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(dec!(5), Currency::RUB),
            total_coupon: Money::from_value(dec!(50), Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap(),
            coupon_type: "Constant".to_string(),
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
