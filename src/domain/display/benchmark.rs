//! Display implementation for the benchmark comparison.

use std::fmt::Display;

use comfy_table::{Attribute, Cell, Color};
use rust_decimal::Decimal;

use super::super::benchmark::{BenchmarkComparison, BenchmarkRow};
use super::super::xirr::AnnualRate;
use crate::ux;

fn rate_cell(rate: Option<Decimal>) -> Cell {
    rate.map_or_else(|| Cell::new("n/a"), |r| Cell::new(AnnualRate(r)))
}

/// Index return colored green when the row return beats it and red otherwise.
fn benchmark_cell(row: &BenchmarkRow, index: Option<Decimal>) -> Cell {
    let cell = rate_cell(index);
    match (row.xirr, index) {
        (Some(own), Some(index)) if own >= index => cell.fg(Color::DarkGreen),
        (Some(_), Some(_)) => cell.fg(Color::DarkRed),
        _ => cell,
    }
}

impl Display for BenchmarkComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();
        table.set_header([Cell::new("Benchmark Comparison")
            .add_attribute(Attribute::Bold)
            .fg(Color::DarkBlue)]);
        let header = ["Assets", "XIRR"]
            .into_iter()
            .chain(self.indices.iter().map(|i| i.benchmark.ticker))
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold));
        table.add_row(header);
        for row in &self.rows {
            let cells = [Cell::new(row.name), rate_cell(row.xirr)]
                .into_iter()
                .chain(row.benchmarks.iter().map(|b| benchmark_cell(row, *b)));
            table.add_row(cells);
        }
        writeln!(f, "\n{table}")?;
        writeln!(
            f,
            "Index columns show the return of the same payments invested into the index.\n\
             Securities are all papers but currencies. Green means the assets beat the index;\n\
             n/a means the index has no data for some payment."
        )?;
        for index in &self.indices {
            let since = index.prices.keys().next().map_or_else(
                || "no data".to_string(),
                |d| format!("data since {}", d.format("%Y-%m-%d")),
            );
            writeln!(
                f,
                "  {}: {}, {since}",
                index.benchmark.ticker, index.benchmark.description
            )?;
        }
        Ok(())
    }
}
