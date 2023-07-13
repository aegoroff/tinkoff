use prettytable::{
    cell,
    format::{self, TableFormat},
    Cell,
};

use crate::domain::NumberRange;

pub fn new_table_format() -> TableFormat {
    format::FormatBuilder::new()
        .column_separator(' ')
        .borders(' ')
        .separators(
            &[format::LinePosition::Title],
            format::LineSeparator::new('-', ' ', ' ', ' '),
        )
        .indent(1)
        .padding(0, 0)
        .build()
}

pub fn colored_cell<T: NumberRange + ToString>(value: T) -> Cell {
    if value.is_negative() {
        cell!(Fr->value)
    } else if value.is_zero() {
        cell!(value)
    } else {
        cell!(Fg->value)
    }
}
