use std::{fmt::Error, process::Command};

use comfy_table::{Cell, ContentArrangement, Table, TableComponent, presets};
use num_format::{Locale, ToFormattedString};
use rust_decimal::{Decimal, prelude::ToPrimitive};

use crate::domain::NumberRange;

/// Converts Decimal to string.
///
/// # Errors
///
/// This function will return an error if failed to convert rounded decimal to i64.
pub fn format_decimal(v: Decimal) -> Result<String, Error> {
    let integer = v
        .round_dp(2)
        .to_i64()
        .ok_or(Error)?
        .to_formatted_string(&Locale::ru);

    let mut fract = v.fract().round_dp(2);
    fract.set_sign_positive(true);
    let fract: String = fract.to_string().chars().skip(1).collect();
    Ok(format!("{integer}{fract}"))
}

/// Creates new table
#[must_use]
pub fn new_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(presets::UTF8_FULL_CONDENSED)
        .set_style(TableComponent::BottomBorder, ' ')
        .set_style(TableComponent::BottomBorderIntersections, ' ')
        .set_style(TableComponent::TopBorder, ' ')
        .set_style(TableComponent::TopBorderIntersections, ' ')
        .set_style(TableComponent::HeaderLines, '-')
        .set_style(TableComponent::RightHeaderIntersection, ' ')
        .set_style(TableComponent::LeftHeaderIntersection, ' ')
        .set_style(TableComponent::MiddleHeaderIntersections, ' ')
        .set_style(TableComponent::LeftBorder, ' ')
        .set_style(TableComponent::RightBorder, ' ')
        .set_style(TableComponent::TopRightCorner, ' ')
        .set_style(TableComponent::TopLeftCorner, ' ')
        .set_style(TableComponent::BottomLeftCorner, ' ')
        .set_style(TableComponent::BottomRightCorner, ' ')
        .set_style(TableComponent::VerticalLines, ' ')
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

/// Adds row into a table
pub fn add_row<C1: ToString, C2: ToString>(table: &mut Table, first_column: C1, second_column: C2) {
    table.add_row([Cell::new(first_column), Cell::new(second_column)]);
}

/// Adds row with colorized `second_column` into table. Colorization based on numeric
/// value. Positives will be green, negatives will be red zero color won't be changed
pub fn add_row_colorized<C1: ToString, C2: ToString + NumberRange>(
    table: &mut Table,
    first_column: C1,
    second_column: C2,
) {
    table.add_row([Cell::new(first_column), colored_cell(second_column)]);
}

/// Creates colorized cell based on numeric value.
/// Positives will be green, negatives will be red zero color won't be changed
pub fn colored_cell<T: NumberRange + ToString>(value: T) -> Cell {
    if value.is_negative() {
        Cell::new(value).fg(comfy_table::Color::DarkRed)
    } else if value.is_zero() {
        Cell::new(value)
    } else {
        Cell::new(value).fg(comfy_table::Color::DarkGreen)
    }
}

#[cfg(target_os = "linux")]
pub fn clear_screen() {
    if let Ok(mut c) = Command::new("clear").spawn() {
        if let Err(e) = c.wait() {
            println!("{e}");
        }
    }
}

#[cfg(target_os = "windows")]
pub fn clear_screen() {
    if let Ok(mut c) = Command::new("cmd").arg("/c").arg("cls").spawn() {
        if let Err(e) = c.wait() {
            println!("{e}");
        }
    }
}

#[cfg(target_os = "macos")]
pub fn clear_screen() {
    if let Ok(mut c) = Command::new("clear").spawn() {
        if let Err(e) = c.wait() {
            println!("{e}");
        }
    }
    if let Ok(mut c) = Command::new("printf").arg("\x1b[3J").spawn() {
        if let Err(e) = c.wait() {
            println!("{e}");
        }
    }
}
