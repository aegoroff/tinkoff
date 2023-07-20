use std::fmt::Error;

use num_format::{Locale, ToFormattedString};
use prettytable::{
    cell,
    format::{self, TableFormat},
    Cell,
};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::domain::NumberRange;

pub fn format_decimal(v: Decimal) -> Result<String, Error> {
    let integer = v
        .round_dp(2)
        .to_i64()
        .ok_or(Error)?
        .to_formatted_string(&Locale::ru);

    let mut fract = v.fract().round_dp(2);
    fract.set_sign_positive(true);
    let fract : String = fract.to_string().chars().skip(1).collect();
    Ok(format!("{integer}{fract}"))
}

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
