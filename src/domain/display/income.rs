//! Display implementation for the passive income forecast.

use std::fmt::Display;

use comfy_table::{Attribute, Cell, Table};

use super::super::income::{IncomeForecast, IncomeTotals};
use super::super::xirr::AnnualRate;
use crate::ux;

/// Label of a month, e.g. `2026 September`.
fn month_label(year: i32, month: u32) -> String {
    let name = u8::try_from(month)
        .ok()
        .and_then(|m| chrono::Month::try_from(m).ok())
        .map_or("Unknown", |m| m.name());
    format!("{year} {name}")
}

fn income_cells(income: &IncomeTotals) -> [Cell; 4] {
    [
        Cell::new(income.coupons),
        Cell::new(income.dividends),
        Cell::new(income.estimated_dividends),
        Cell::new(income.total()),
    ]
}

fn create_months_table(forecast: &IncomeForecast) -> Table {
    let mut table = ux::new_table();
    table.set_header([Cell::new("Passive Income Forecast")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue)]);
    table.add_row(
        ["Month", "Coupons", "Dividends", "Est. dividends", "Total"]
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
    );
    for month in &forecast.months {
        let mut row = vec![Cell::new(month_label(month.year, month.month))];
        row.extend(income_cells(&month.income));
        table.add_row(row);
    }
    let mut total = vec![Cell::new("Total")];
    total.extend(income_cells(&forecast.total()));
    table.add_row(total.into_iter().map(|c| c.add_attribute(Attribute::Bold)));
    table
}

fn create_summary_table(forecast: &IncomeForecast) -> Table {
    let mut table = ux::new_table();
    table.set_header([
        Cell::new("Income Summary")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue),
        Cell::new(""),
    ]);
    ux::add_row(&mut table, "Year total", forecast.total().total());
    ux::add_row(&mut table, "Monthly average", forecast.monthly_average());
    ux::add_row(&mut table, "Portfolio value", forecast.portfolio_value);
    match forecast.current_yield() {
        Some(rate) => ux::add_row_colorized(&mut table, "Current yield", AnnualRate(rate)),
        None => ux::add_row(&mut table, "Current yield", "n/a"),
    }
    table
}

impl Display for IncomeForecast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "\n{}", create_months_table(self))?;
        writeln!(f, "\n{}", create_summary_table(self))?;
        writeln!(
            f,
            "Amounts are before taxes. Estimated dividends repeat the ones paid a year before."
        )
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::january(2027, 1, "2027 January")]
    #[case::december(2026, 12, "2026 December")]
    #[case::invalid(2026, 13, "2026 Unknown")]
    fn month_label_names_month(#[case] year: i32, #[case] month: u32, #[case] expected: &str) {
        // Act
        let label = month_label(year, month);

        // Assert
        assert_eq!(label, expected);
    }
}
