//! Display implementation for the tax report.

use std::fmt::Display;

use comfy_table::{Attribute, Cell};

use super::super::tax::{TaxReport, YearTaxes};
use crate::ux;

fn year_cells(label: String, year: &YearTaxes) -> [Cell; 7] {
    [
        Cell::new(label),
        Cell::new(year.dividends),
        Cell::new(year.coupons),
        Cell::new(year.dividend_tax),
        Cell::new(year.coupon_tax),
        Cell::new(year.other_tax),
        Cell::new(year.total_tax()),
    ]
}

impl Display for TaxReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.years.is_empty() {
            return writeln!(f, "No income or taxes");
        }
        let mut table = ux::new_table();
        table.set_header([Cell::new("Income and Taxes")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        table.add_row(
            [
                "Year",
                "Dividends",
                "Coupons",
                "Dividend tax",
                "Coupon tax",
                "Other tax",
                "Total tax",
            ]
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
        );
        for year in &self.years {
            table.add_row(year_cells(year.year.to_string(), year));
        }
        table.add_row(
            year_cells("Total".to_string(), &self.total())
                .map(|c| c.add_attribute(Attribute::Bold)),
        );
        writeln!(f, "\n{table}")?;
        writeln!(
            f,
            "Taxes withheld by the broker, corrections and refunds included.\n\
             Other tax is withheld at the year end or on withdrawal from sales and, \
             since the middle of 2023, from coupons too.\n\
             Tax withheld on the 1st of January is counted in the previous year."
        )
    }
}
