use std::fmt::Display;

use comfy_table::{Attribute, Cell, TableComponent};

use crate::ux;

use super::super::paper::Paper;
use super::super::paper::Profit;
use super::super::portfolio::{Asset, Portfolio};
use super::labels::{BALANCE_INCOME, BALANCE_VALUE, CURRENT_VALUE, INCOME, TOTAL_INCOME};

impl<P: Profit> Display for Asset<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut asset_table = ux::new_table();
        asset_table.set_header([Cell::new(self.name)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        asset_table.set_style(TableComponent::HeaderLines, ' ');

        if self.output_papers {
            for p in self.papers() {
                asset_table.add_row([Cell::new(p)]);
            }
        }

        let mut table = ux::new_table();

        let title = format!("{} totals:", self.name);
        let title = Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header([title, Cell::new("")]);

        ux::add_row(&mut table, BALANCE_VALUE, self.balance());
        ux::add_row(&mut table, CURRENT_VALUE, self.current());
        ux::add_row_colorized(&mut table, BALANCE_INCOME, self.income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, P::name(), self.dividents());
        }

        ux::add_row(&mut table, "Instruments count", self.papers().len());
        asset_table.add_row([Cell::new(table)]);

        if self.is_empty() {
            Ok(())
        } else {
            write!(f, "{asset_table}")
        }
    }
}

impl<P: Profit> Display for Paper<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();

        let currency = self.currency().code().to_owned();
        let title = format!(
            "{} ({} | {} | {})",
            self.name, self.ticker, self.figi, currency
        );

        table.set_header([
            Cell::new(title).add_attribute(Attribute::Bold),
            Cell::new(""),
        ]);

        ux::add_row(&mut table, "Average buy price", self.average_buy_price());
        ux::add_row(
            &mut table,
            "Last instrument price",
            self.current_instrument_price(),
        );
        ux::add_row(
            &mut table,
            "Current items count",
            self.quantity().round_dp(2),
        );
        ux::add_row(&mut table, BALANCE_VALUE, self.balance());
        ux::add_row(&mut table, CURRENT_VALUE, self.current());
        table.add_row(["", ""]);

        ux::add_row_colorized(&mut table, INCOME, self.income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, P::name(), self.dividents());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
        }

        ux::add_row_colorized(&mut table, "Taxes and fees", self.fees());

        write!(f, "{table}")
    }
}

impl Display for Portfolio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.etfs.fmt(f)?;
        self.futures.fmt(f)?;
        self.bonds.fmt(f)?;
        self.shares.fmt(f)?;
        self.currencies.fmt(f)?;

        if self.count_not_empty_assets() > 1 {
            let mut table = ux::new_table();

            let title = Cell::new("Portfolio totals:")
                .add_attribute(Attribute::Bold)
                .fg(comfy_table::Color::DarkRed);
            table.set_header([title, Cell::new("")]);

            ux::add_row_colorized(&mut table, BALANCE_INCOME, self.income());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, "Dividents and coupons", self.dividents());

            ux::add_row(&mut table, BALANCE_VALUE, self.balance());
            ux::add_row(&mut table, CURRENT_VALUE, self.current());

            writeln!(f)?;
            writeln!(f, "{table}")
        } else {
            writeln!(f)
        }
    }
}
