use std::fmt::Display;

use chrono::{DateTime, Utc};
use comfy_table::{Attribute, Cell, Table};
use rust_decimal::Decimal;

use crate::ux;

use super::super::paper::Paper;
use super::super::paper::Profit;
use super::super::portfolio::{Asset, Portfolio};
use super::super::xirr::AnnualRate;

const TOTAL_INCOME: &str = "Total income";
const INCOME: &str = "Income";
const CURRENT_VALUE: &str = "Current value";
const BALANCE_VALUE: &str = "Balance value";
const BALANCE_INCOME: &str = "Balance income";
const DAILY_CHANGE: &str = "Daily change";
const XIRR: &str = "Annual return (XIRR)";
const YTM: &str = "Yield to maturity";
const DURATION: &str = "Duration, years";

/// Adds XIRR row when the return can be calculated.
fn add_xirr_row(table: &mut Table, rate: Option<Decimal>) {
    if let Some(rate) = rate {
        ux::add_row_colorized(table, XIRR, AnnualRate(rate));
    }
}

/// Adds a duration row when the duration is known.
fn add_duration_row(table: &mut Table, title: &str, duration: Option<Decimal>) {
    if let Some(duration) = duration {
        ux::add_row(table, title, duration.round_dp(2));
    }
}

/// Adds a date row when the date is known.
fn add_date_row(table: &mut Table, title: &str, date: Option<DateTime<Utc>>) {
    if let Some(date) = date {
        ux::add_row(table, title, date.format("%Y-%m-%d"));
    }
}

impl<P: Profit> Display for Asset<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut asset_table = ux::new_table();
        asset_table.set_header([Cell::new(self.name)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        asset_table.style_mut().header_separator.fill = Some(' ');

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
        ux::add_row_colorized(&mut table, DAILY_CHANGE, self.daily_income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, P::name(), self.dividends());
        }
        add_xirr_row(&mut table, self.xirr(Utc::now()));

        if let Some(duration) = self.duration() {
            let count = self.duration_count();
            let total = self.papers().len();
            let title = if count == total {
                DURATION.to_string()
            } else {
                format!("{DURATION} ({count} of {total})")
            };
            add_duration_row(&mut table, &title, Some(duration));
        }
        if let Some(change) = self.rate_sensitivity() {
            ux::add_row_colorized(&mut table, "Value change at +1% yield", change);
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
        if !self.accrued_interest().value.is_zero() {
            ux::add_row(&mut table, "Accrued interest", self.accrued_interest());
        }
        ux::add_row(
            &mut table,
            "Current items count",
            self.quantity().round_dp(2),
        );
        ux::add_row(&mut table, BALANCE_VALUE, self.balance());
        ux::add_row(&mut table, CURRENT_VALUE, self.current());
        if !self.position.blocked_lots.is_zero() {
            ux::add_row(
                &mut table,
                "Blocked by orders",
                self.position.blocked_lots.round_dp(2),
            );
        }
        if self.position.blocked {
            ux::add_row(&mut table, "Blocked by exchange", "yes");
        }
        table.add_row(["", ""]);

        ux::add_row_colorized(&mut table, INCOME, self.income());
        ux::add_row_colorized(&mut table, DAILY_CHANGE, self.daily_income());

        if P::applicable() {
            ux::add_row_colorized(&mut table, P::name(), self.dividends());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
        }

        ux::add_row_colorized(&mut table, "Taxes and fees", self.fees());
        add_xirr_row(&mut table, self.xirr(Utc::now()));

        if let Some(bond) = &self.bond {
            table.add_row(["", ""]);
            add_date_row(&mut table, "Maturity date", bond.maturity_date);
            add_date_row(&mut table, "Next offer date", bond.next_offer_date);
            match bond.ytm {
                Some(ytm) => ux::add_row_colorized(&mut table, YTM, AnnualRate(ytm)),
                None => ux::add_row(&mut table, YTM, "n/a"),
            }
            if let Some(rate) = bond.yield_to_offer {
                ux::add_row_colorized(&mut table, "Yield to offer", AnnualRate(rate));
            }
            add_duration_row(&mut table, DURATION, bond.duration);
            add_duration_row(&mut table, "Modified duration", bond.modified_duration);
        }

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
            ux::add_row_colorized(&mut table, DAILY_CHANGE, self.daily_income());
            ux::add_row_colorized(&mut table, TOTAL_INCOME, self.total_income());
            ux::add_row_colorized(&mut table, "Dividends and coupons", self.dividends());
            add_xirr_row(&mut table, self.xirr(Utc::now()));

            ux::add_row(&mut table, BALANCE_VALUE, self.balance());
            ux::add_row(&mut table, CURRENT_VALUE, self.current());

            writeln!(f)?;
            writeln!(f, "{table}")
        } else {
            writeln!(f)
        }
    }
}
