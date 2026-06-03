use std::{collections::HashMap, fmt::Display};

use chrono::{DateTime, Datelike, Utc};
use comfy_table::{Attribute, Cell};

use crate::ux;

use super::super::calendar::CalendarPayment;
use super::super::money::Money;
use super::super::{CouponCalendar, DividendCalendar};

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

/// Generic calendar Display implementation for any [`CalendarPayment`] type
pub(super) fn format_calendar<P: CalendarPayment>(upcoming: &[P]) -> String {
    let mut table = ux::new_table();

    let title = Cell::new(P::calendar_title())
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    let (payment_date_hdr, ex_date_hdr, company_hdr, per_unit_hdr, total_hdr) = P::column_headers();
    let payment_date = Cell::new(payment_date_hdr).add_attribute(Attribute::Bold);
    let ex_date = Cell::new(ex_date_hdr).add_attribute(Attribute::Bold);
    let company = Cell::new(company_hdr).add_attribute(Attribute::Bold);
    let per_unit = Cell::new(per_unit_hdr).add_attribute(Attribute::Bold);
    let total = Cell::new(total_hdr).add_attribute(Attribute::Bold);
    table.add_row([payment_date, ex_date, company, per_unit, total]);

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

    let mut grouped: HashMap<(i32, u32), Vec<&P>> = HashMap::new();
    for payment in upcoming {
        let date = payment.payment_date();
        grouped
            .entry((date.year(), date.month()))
            .or_default()
            .push(payment);
    }

    let mut keys: Vec<_> = grouped.keys().copied().collect();
    keys.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let mut grand_total = Money::zero(iso_currency::Currency::RUB);

    let mut by_year: HashMap<i32, Vec<u32>> = HashMap::new();
    for (year, month) in &keys {
        by_year.entry(*year).or_default().push(*month);
    }

    let mut year_keys: Vec<_> = by_year.keys().copied().collect();
    year_keys.sort_unstable();

    for year in year_keys {
        table.add_row([
            Cell::new(format!("{year}"))
                .add_attribute(Attribute::Bold)
                .fg(comfy_table::Color::DarkCyan),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
            Cell::new(""),
        ]);

        let mut year_total = Money::zero(iso_currency::Currency::RUB);

        if let Some(months) = by_year.get(&year) {
            for month in months {
                let month_name_str = month_name(*month);
                table.add_row([
                    Cell::new(month_name_str).add_attribute(Attribute::Bold),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(""),
                ]);

                let mut month_total = Money::zero(iso_currency::Currency::RUB);

                if let Some(payments) = grouped.get(&(year, *month)) {
                    for payment in payments {
                        table.add_row([
                            Cell::new(format_date(payment.payment_date())),
                            Cell::new(format_date(payment.ex_date())),
                            Cell::new(payment.name().to_string()),
                            Cell::new(payment.payment_per_unit().to_string()),
                            Cell::new(payment.total_payment().to_string()),
                        ]);
                        month_total += payment.total_payment();
                    }
                }

                table.add_row([
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(P::month_label(month_name_str)).add_attribute(Attribute::Bold),
                    Cell::new(""),
                    Cell::new(month_total.to_string()).add_attribute(Attribute::Bold),
                ]);

                year_total += month_total;
                grand_total += month_total;
            }
        }

        table.add_row([
            Cell::new(""),
            Cell::new(""),
            Cell::new(P::year_label(year))
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
}
