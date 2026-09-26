use std::fmt::Display;

use comfy_table::{Attribute, Cell, CellAlignment, Color, Table};
use rust_decimal::Decimal;

use super::super::analytics::{Analytics, Forecast, Fundamentals, Recommendation};
use crate::ux;

const NOT_AVAILABLE: &str = "n/a";

impl Display for Recommendation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "Buy"),
            Self::Hold => write!(f, "Hold"),
            Self::Sell => write!(f, "Sell"),
        }
    }
}

fn title(text: &str) -> Cell {
    Cell::new(text)
        .add_attribute(Attribute::Bold)
        .fg(Color::DarkBlue)
}

fn recommendation_cell(recommendation: Option<Recommendation>) -> Cell {
    match recommendation {
        Some(r @ Recommendation::Buy) => Cell::new(r).fg(Color::DarkGreen),
        Some(r @ Recommendation::Hold) => Cell::new(r).fg(Color::DarkYellow),
        Some(r @ Recommendation::Sell) => Cell::new(r).fg(Color::DarkRed),
        None => Cell::new(NOT_AVAILABLE),
    }
}

fn percent(value: Decimal) -> String {
    format!("{}%", value.round_dp(2))
}

fn upside_cell(upside: Option<Decimal>) -> Cell {
    let Some(upside) = upside else {
        return Cell::new(NOT_AVAILABLE);
    };
    let cell = Cell::new(percent(upside));
    if upside.is_sign_negative() && !upside.is_zero() {
        cell.fg(Color::DarkRed)
    } else if upside.is_zero() {
        cell
    } else {
        cell.fg(Color::DarkGreen)
    }
}

fn metric_cell(value: Option<Decimal>, format: fn(Decimal) -> String) -> Cell {
    Cell::new(value.map_or_else(|| NOT_AVAILABLE.to_string(), format))
        .set_alignment(CellAlignment::Right)
}

fn ratio(value: Decimal) -> String {
    value.round_dp(2).to_string()
}

fn forecast_cells(forecast: &Forecast) -> [Cell; 6] {
    [
        recommendation_cell(forecast.recommendation),
        Cell::new(forecast.current_price).set_alignment(CellAlignment::Right),
        Cell::new(forecast.target_price).set_alignment(CellAlignment::Right),
        upside_cell(forecast.upside()).set_alignment(CellAlignment::Right),
        Cell::new(format!("{} – {}", forecast.min_target, forecast.max_target)),
        Cell::new(forecast.analysts).set_alignment(CellAlignment::Right),
    ]
}

fn fundamentals_cells(fundamentals: &Fundamentals) -> [Cell; 7] {
    [
        metric_cell(fundamentals.pe, ratio),
        metric_cell(fundamentals.pb, ratio),
        metric_cell(fundamentals.ev_to_ebitda, ratio),
        metric_cell(fundamentals.net_debt_to_ebitda, ratio),
        metric_cell(fundamentals.roe, percent),
        metric_cell(fundamentals.dividend_yield, percent),
        metric_cell(fundamentals.beta, ratio),
    ]
}

fn forecasts_table(analytics: &Analytics) -> Table {
    let mut table = ux::new_table();
    table.set_header([
        title("Analyst forecasts"),
        title("Name"),
        title("Consensus"),
        title("Price"),
        title("Target"),
        title("Upside"),
        title("Target range"),
        title("Analysts"),
    ]);
    for share in analytics.shares() {
        let mut row = vec![Cell::new(&share.ticker), Cell::new(&share.name)];
        match &share.forecast {
            Some(forecast) => row.extend(forecast_cells(forecast)),
            None => row.push(Cell::new(NOT_AVAILABLE)),
        }
        table.add_row(row);
    }
    table
}

fn fundamentals_table(analytics: &Analytics) -> Table {
    let mut table = ux::new_table();
    table.set_header([
        title("Fundamentals"),
        title("P/E"),
        title("P/B"),
        title("EV/EBITDA"),
        title("Net debt/EBITDA"),
        title("ROE"),
        title("Dividend yield"),
        title("Beta"),
    ]);
    for share in analytics.shares() {
        let mut row = vec![Cell::new(&share.name)];
        row.extend(fundamentals_cells(&share.fundamentals.unwrap_or_default()));
        table.add_row(row);
    }
    table
}

impl Display for Analytics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.shares().is_empty() {
            return writeln!(f, "No shares in the portfolio");
        }
        writeln!(f, "{}", forecasts_table(self))?;
        writeln!(f)?;
        writeln!(f, "{}", fundamentals_table(self))
    }
}
