use std::fmt::Display;

use chrono::{DateTime, Utc};
use comfy_table::{Attribute, Cell, CellAlignment, Color, Table};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::analytics::{Analytics, Forecast, Fundamentals, Recommendation, ShareAnalytics};
use super::super::money::Money;
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
        Cell::new(forecast.analysts()).set_alignment(CellAlignment::Right),
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

fn date(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d").to_string()
}

/// Company-wide amount in billions or millions, e.g. `3 722.07 bn ₽`.
fn compact(value: Money) -> String {
    let (scale, unit) = if value.value.abs() >= dec!(1_000_000_000) {
        (dec!(1_000_000_000), "bn")
    } else if value.value.abs() >= dec!(1_000_000) {
        (dec!(1_000_000), "mn")
    } else {
        return value.to_string();
    };
    let amount = ux::format_decimal(value.value / scale).unwrap_or_default();
    format!("{amount} {unit} {}", value.currency.symbol())
}

fn money(value: Money) -> String {
    value.to_string()
}

fn house_forecasts_table(share: &ShareAnalytics, forecast: &Forecast) -> Table {
    let mut table = ux::new_table();
    table.set_header([
        title(&format!(
            "Analyst forecasts: {} ({})",
            share.name, share.ticker
        )),
        title("Consensus"),
        title("Date"),
        title("Target"),
        title("Upside"),
    ]);
    table.add_row([
        Cell::new("Current price"),
        Cell::new(""),
        Cell::new(""),
        Cell::new(forecast.current_price).set_alignment(CellAlignment::Right),
        Cell::new(""),
    ]);
    table.add_row([
        Cell::new("Consensus").add_attribute(Attribute::Bold),
        recommendation_cell(forecast.recommendation),
        Cell::new(""),
        Cell::new(forecast.target_price)
            .add_attribute(Attribute::Bold)
            .set_alignment(CellAlignment::Right),
        upside_cell(forecast.upside()).set_alignment(CellAlignment::Right),
    ]);
    table.add_row([
        Cell::new("Target range"),
        Cell::new(""),
        Cell::new(""),
        Cell::new(format!("{} – {}", forecast.min_target, forecast.max_target))
            .set_alignment(CellAlignment::Right),
        Cell::new(""),
    ]);
    for house in &forecast.targets {
        table.add_row([
            Cell::new(&house.company),
            recommendation_cell(house.recommendation),
            Cell::new(house.date.map_or_else(|| NOT_AVAILABLE.to_string(), date)),
            Cell::new(house.target_price).set_alignment(CellAlignment::Right),
            upside_cell(forecast.upside_to(house.target_price)).set_alignment(CellAlignment::Right),
        ]);
    }
    table
}

/// Metric rows of the detailed fundamentals, groups separated by `None`.
fn fundamentals_rows(f: &Fundamentals) -> Vec<Option<(&'static str, Cell)>> {
    let row = |name, cell| Some((name, cell));
    let range = f
        .low_52_weeks
        .zip(f.high_52_weeks)
        .map(|(low, high)| format!("{low} – {high}"));
    vec![
        row("Market capitalization", metric_money(f.market_cap, compact)),
        row(
            "Enterprise value",
            metric_money(f.enterprise_value, compact),
        ),
        row("P/E", metric_cell(f.pe, ratio)),
        row("P/S", metric_cell(f.ps, ratio)),
        row("P/B", metric_cell(f.pb, ratio)),
        row("P/FCF", metric_cell(f.price_to_fcf, ratio)),
        row("EV/EBITDA", metric_cell(f.ev_to_ebitda, ratio)),
        None,
        row("Revenue", metric_money(f.revenue, compact)),
        row("Revenue growth", metric_cell(f.revenue_growth, percent)),
        row("EBITDA", metric_money(f.ebitda, compact)),
        row("Net income", metric_money(f.net_income, compact)),
        row("Net margin", metric_cell(f.net_margin, percent)),
        row("Free cash flow", metric_money(f.free_cash_flow, compact)),
        row("EPS", metric_money(f.eps, money)),
        row("ROE", metric_cell(f.roe, percent)),
        row("ROA", metric_cell(f.roa, percent)),
        None,
        row("Total debt", metric_money(f.total_debt, compact)),
        row("Debt/Equity", metric_cell(f.debt_to_equity, percent)),
        row("Net debt/EBITDA", metric_cell(f.net_debt_to_ebitda, ratio)),
        None,
        row("Dividend yield", metric_cell(f.dividend_yield, percent)),
        row(
            "5 years dividend yield",
            metric_cell(f.five_year_dividend_yield, percent),
        ),
        row(
            "Dividends per share",
            metric_money(f.dividends_per_share, money),
        ),
        row("Payout ratio", metric_cell(f.payout_ratio, percent)),
        row(
            "Ex-dividend date",
            Cell::new(
                f.ex_dividend_date
                    .map_or_else(|| NOT_AVAILABLE.to_string(), date),
            )
            .set_alignment(CellAlignment::Right),
        ),
        None,
        row("Beta", metric_cell(f.beta, ratio)),
        row("Free float", metric_cell(f.free_float, percent)),
        row(
            "52 weeks range",
            Cell::new(range.unwrap_or_else(|| NOT_AVAILABLE.to_string()))
                .set_alignment(CellAlignment::Right),
        ),
    ]
}

fn metric_money(value: Option<Money>, format: fn(Money) -> String) -> Cell {
    Cell::new(value.map_or_else(|| NOT_AVAILABLE.to_string(), format))
        .set_alignment(CellAlignment::Right)
}

fn fundamentals_details_table(share: &ShareAnalytics) -> Table {
    let mut table = ux::new_table();
    table.set_header([
        title(&format!("Fundamentals: {} ({})", share.name, share.ticker)),
        title(""),
    ]);
    let fundamentals = share.fundamentals.unwrap_or_default();
    for row in fundamentals_rows(&fundamentals) {
        match row {
            Some((name, cell)) => table.add_row([Cell::new(name), cell]),
            None => table.add_row(["", ""]),
        };
    }
    table
}

/// Detailed analytics of one share: forecasts of every investment house and fundamentals.
impl Display for ShareAnalytics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.forecast {
            Some(forecast) => writeln!(f, "{}", house_forecasts_table(self, forecast))?,
            None => writeln!(
                f,
                "No analyst forecasts for {} ({})",
                self.name, self.ticker
            )?,
        }
        writeln!(f)?;
        writeln!(f, "{}", fundamentals_details_table(self))
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::trillions(dec!(3722074873464), "3\u{a0}722.07 bn ₽")]
    #[case::loss(dec!(-908369000000), "-908.37 bn ₽")]
    #[case::millions(dec!(4617000), "4.62 mn ₽")]
    #[case::small(dec!(1500), "1\u{a0}500 ₽")]
    fn compact_amount(#[case] value: Decimal, #[case] expected: &str) {
        // Act
        let text = compact(Money::from_value(value, Currency::RUB));

        // Assert
        assert_eq!(text, expected);
    }
}
