use std::fmt::Display;

use comfy_table::{Attribute, Cell, TableComponent};

use crate::ux;

use super::super::history::History;

impl Display for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut history_table = ux::new_table();

        let currency = self.currency.code().to_owned();
        let title = format!(
            "{} ({} | {} | {})",
            self.name, self.ticker, self.figi, currency
        );

        history_table.set_header([Cell::new(title)
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        history_table.set_style(TableComponent::HeaderLines, ' ');

        let mut items_table = ux::new_table();

        let date = Cell::new("Date").add_attribute(Attribute::Bold);
        let quantity = Cell::new("Quantity").add_attribute(Attribute::Bold);
        let price = Cell::new("Price").add_attribute(Attribute::Bold);
        let payment = Cell::new("Payment").add_attribute(Attribute::Bold);
        let description = Cell::new("Description").add_attribute(Attribute::Bold);
        let state = Cell::new("State").add_attribute(Attribute::Bold);
        items_table.set_header([date, quantity, price, payment, description, state]);

        for item in &self.items {
            items_table.add_row([
                Cell::new(item.datetime),
                Cell::new(item.quantity - item.quantity_rest),
                Cell::new(item.price),
                Cell::new(item.payment),
                Cell::new(&item.description),
                Cell::new(item.operation_state),
            ]);
        }

        history_table.add_row([Cell::new(items_table)]);
        write!(f, "{history_table}")?;

        let mut table = ux::new_table();

        let title = Cell::new("Totals")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkYellow);
        table.set_header([title, Cell::new("")]);

        ux::add_row_colorized(&mut table, "Expenses", self.expenses());
        ux::add_row_colorized(&mut table, "Profit", self.profit());
        ux::add_row_colorized(&mut table, "Balance", self.balance());
        write!(f, "{table}")
    }
}
