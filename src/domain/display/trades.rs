//! Display implementation for rebalancing orders.

use std::fmt::Display;

use comfy_table::{Attribute, Cell};
use rust_decimal::Decimal;

use super::super::trades::TradePlan;
use super::risk::action_cell;
use crate::ux;

fn quantity(value: Decimal) -> String {
    value.normalize().to_string()
}

impl Display for TradePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.assets.is_empty() {
            return Ok(());
        }
        let mut table = ux::new_table();
        table.set_header([Cell::new("Rebalancing Orders")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        table.add_row(
            ["Action", "Ticker", "Name", "Lots", "Units", "Amount"]
                .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
        );
        for asset in &self.assets {
            for order in &asset.orders {
                table.add_row([
                    action_cell(asset.action),
                    Cell::new(&order.ticker),
                    Cell::new(&order.name),
                    Cell::new(quantity(order.lots)),
                    Cell::new(quantity(order.units)),
                    Cell::new(order.amount),
                ]);
            }
            if !asset.left.value.is_zero() {
                let what = if asset.by_amount() {
                    format!("{} by amount", asset.asset_type)
                } else {
                    format!("{} left, less than a lot", asset.asset_type)
                };
                table.add_row([
                    action_cell(asset.action),
                    Cell::new(""),
                    Cell::new(what),
                    Cell::new(""),
                    Cell::new(""),
                    Cell::new(asset.left),
                ]);
            }
        }
        writeln!(f, "\n{table}")?;
        match self.tax {
            Some(tax) => writeln!(f, "Tax on the sells: {tax}")?,
            None => writeln!(
                f,
                "Tax on the sells: unknown, as lots of several accounts or of positions \
                 transferred from another broker are not known"
            )?,
        }
        writeln!(
            f,
            "Buys split an asset type by the current values of its instruments. Sells take \
             the lots adding the least tax first: losses, lots exempt by LDV, the smallest \
             gains, lots of unknown cost last. Currencies, futures and asset types without \
             instruments are rebalanced by amount."
        )
    }
}
