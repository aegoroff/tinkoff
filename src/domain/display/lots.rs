//! Display implementation for the tax due on sale.

use std::fmt::Display;

use comfy_table::{Attribute, Cell, Table};
use rust_decimal::Decimal;

use super::super::lots::{Basis, PositionSale, SaleTaxReport};
use crate::ux;

fn title(text: String) -> Cell {
    Cell::new(text)
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue)
}

fn header<const N: usize>(table: &mut Table, names: [&str; N]) {
    table.add_row(names.map(|h| Cell::new(h).add_attribute(Attribute::Bold)));
}

fn quantity(value: Decimal) -> String {
    value.normalize().to_string()
}

fn position_row(report: &SaleTaxReport, position: &PositionSale) -> Vec<Cell> {
    let mut row = vec![
        Cell::new(&position.ticker),
        Cell::new(&position.name),
        Cell::new(quantity(position.quantity)),
    ];
    match position.basis {
        Basis::Lots(_) => {
            let next_ldv = position
                .next_ldv()
                .map(|(date, units)| format!("{date} ({})", quantity(units)))
                .unwrap_or_default();
            row.extend([
                Cell::new(position.cost()),
                Cell::new(position.value()),
                ux::colored_cell(position.result()),
                Cell::new(report.position_tax(position)),
                Cell::new(quantity(position.ldv_quantity())),
                Cell::new(next_ldv),
            ]);
        }
        Basis::Unknown { in_lots } if in_lots == position.quantity => {
            row.push(Cell::new("unknown: more units sold than bought"));
        }
        Basis::Unknown { in_lots } => row.push(Cell::new(format!(
            "unknown: trades cover {} units",
            quantity(in_lots)
        ))),
    }
    row
}

fn create_positions_table(report: &SaleTaxReport) -> Table {
    let mut table = ux::new_table();
    table.set_header([title("Tax If Sold Now".to_string())]);
    header(
        &mut table,
        [
            "Ticker", "Name", "Qty", "Cost", "Value", "Result", "Tax", "LDV qty", "Next LDV",
        ],
    );
    for position in &report.positions {
        table.add_row(position_row(report, position));
    }
    table
}

fn create_summary_table(report: &SaleTaxReport) -> Table {
    let mut table = ux::new_table();
    table.set_header([title("Summary".to_string()), Cell::new("")]);
    ux::add_row_colorized(
        &mut table,
        "Sales result this year",
        report.year_income.securities,
    );
    ux::add_row(
        &mut table,
        "Dividends and coupons this year",
        report.year_income.payments,
    );
    ux::add_row(&mut table, "Cost of positions", report.cost());
    ux::add_row(&mut table, "Value of positions", report.value());
    ux::add_row_colorized(&mut table, "Result if sold now", report.result());
    ux::add_row(&mut table, "Exempt by LDV", report.ldv_exemption());
    if report.taxed {
        ux::add_row(&mut table, "Tax if sold now", report.tax());
    } else {
        ux::add_row(&mut table, "Tax if sold now", "none on IIS");
    }
    table
}

impl Display for SaleTaxReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.positions.is_empty() {
            return writeln!(f, "No shares, bonds or ETFs");
        }
        writeln!(f, "\n{}", create_positions_table(self))?;
        writeln!(f, "\n{}", create_summary_table(self))?;
        writeln!(
            f,
            "Lots are matched by FIFO; cost includes commissions and accrued interest paid, \
             in RUB at trade dates by exchange rates rather than the Central Bank ones.\n\
             Tax of a position is the one due if it alone were sold, after this year's sales \
             and with 13% up to 2.4 million RUB of investment income and 15% over it; \
             a negative tax is a saving from offsetting this year's gains.\n\
             LDV exempts lots bought since 2014 and held over 3 years, up to 3 million RUB \
             per full year of holding.{}",
            if self.unknown().next().is_some() {
                "\nUnknown cost: units were transferred from another broker or split, \
                 so they are left out of the totals."
            } else {
                ""
            }
        )
    }
}

impl Display for PositionSale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();
        table.set_header([title(format!("Lots of {} ({})", self.ticker, self.name))]);
        header(
            &mut table,
            [
                "Bought", "Qty", "Cost", "Value", "Result", "Years", "LDV from",
            ],
        );
        for lot in self.lots() {
            let ldv_from = lot.ldv_from.map(|d| d.to_string()).unwrap_or_default();
            table.add_row([
                Cell::new(lot.bought),
                Cell::new(quantity(lot.quantity)),
                Cell::new(lot.cost),
                Cell::new(lot.value),
                ux::colored_cell(lot.result()),
                Cell::new(lot.years_held),
                if lot.ldv {
                    Cell::new(ldv_from).fg(comfy_table::Color::DarkGreen)
                } else {
                    Cell::new(ldv_from)
                },
            ]);
        }
        writeln!(f, "\n{table}")
    }
}
