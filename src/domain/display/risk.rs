//! Display implementations for risk analysis types.
//!
//! This module contains only the Display trait implementations and related
//! formatting functions for risk analysis types defined in `super::super::risk`.

use std::fmt::Display;

use comfy_table::{Attribute, Cell, Table};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::super::risk::{
    AssetAllocation, CurrencyAllocation, PositionConcentration, RebalanceAction,
    RebalancingAnalysis, RiskAnalysis, RiskLevel, RiskMetrics,
};
use crate::ux;

impl Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::Medium => write!(f, "Medium"),
            Self::High => write!(f, "High"),
            Self::VeryHigh => write!(f, "Very High"),
        }
    }
}

fn risk_level_color(level: &RiskLevel) -> comfy_table::Color {
    match level {
        RiskLevel::Low => comfy_table::Color::DarkGreen,
        RiskLevel::Medium => comfy_table::Color::DarkYellow,
        RiskLevel::High => comfy_table::Color::DarkRed,
        RiskLevel::VeryHigh => comfy_table::Color::Red,
    }
}

/// Formats a table row with label and colorized value based on risk (lower is better)
fn add_risk_row(table: &mut Table, label: &str, value: &str, risk_value: Decimal) {
    let mut cell = Cell::new(value);
    // Lower risk values are better (green), higher are worse (red)
    if risk_value < dec!(25) {
        cell = cell.fg(comfy_table::Color::DarkGreen);
    } else if risk_value < dec!(50) {
        cell = cell.fg(comfy_table::Color::DarkYellow);
    } else if risk_value < dec!(75) {
        cell = cell.fg(comfy_table::Color::Yellow);
    } else {
        cell = cell.fg(comfy_table::Color::DarkRed);
    }
    table.add_row([Cell::new(label), cell]);
}

/// Creates the risk summary table
fn create_risk_summary_table(metrics: &RiskMetrics) -> Table {
    let mut table = ux::new_table();

    // Header
    let title = Cell::new("Risk Summary")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Risk level with color
    let risk_level_cell =
        Cell::new(metrics.risk_level.to_string()).fg(risk_level_color(&metrics.risk_level));
    table.add_row([Cell::new("Risk Level"), risk_level_cell]);

    // Diversification score (higher is better)
    let div_score = ux::format_decimal(metrics.diversification_score).unwrap_or_default();
    let mut div_cell = Cell::new(format!("{div_score} / 100"));
    if metrics.diversification_score >= dec!(70) {
        div_cell = div_cell.fg(comfy_table::Color::DarkGreen);
    } else if metrics.diversification_score >= dec!(50) {
        div_cell = div_cell.fg(comfy_table::Color::DarkYellow);
    } else {
        div_cell = div_cell.fg(comfy_table::Color::DarkRed);
    }
    table.add_row([Cell::new("Diversification Score"), div_cell]);

    // Risk metrics (lower is better)
    add_risk_row(
        &mut table,
        "Currency Risk",
        &ux::format_decimal(metrics.currency_risk).unwrap_or_default(),
        metrics.currency_risk,
    );
    add_risk_row(
        &mut table,
        "Concentration Risk",
        &ux::format_decimal(metrics.concentration_risk).unwrap_or_default(),
        metrics.concentration_risk,
    );
    add_risk_row(
        &mut table,
        "Asset Concentration Risk",
        &ux::format_decimal(metrics.asset_concentration_risk).unwrap_or_default(),
        metrics.asset_concentration_risk,
    );

    // Additional risk metrics
    table.add_row([
        Cell::new("Volatility (Ann.)"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(metrics.volatility).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("Beta"),
        Cell::new(ux::format_decimal(metrics.beta).unwrap_or_default()),
    ]);
    // VaR section header
    let var_header = Cell::new("Value at Risk (95%)")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkCyan);
    table.add_row([var_header]);
    table.add_row([
        Cell::new("VaR 1d"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(metrics.var_95_1d).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("VaR 30d"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(metrics.var_95_30d).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("VaR Quarterly (90d)"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(metrics.var_95_quarterly).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("VaR Yearly (252d)"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(metrics.var_95_yearly).unwrap_or_default()
        )),
    ]);

    table
}

impl Display for RiskAnalysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let risk_summary = create_risk_summary_table(&self.risk_metrics);
        let asset_allocation = create_asset_allocation_table(&self.asset_allocation);
        let currency_diversification = create_currency_table(&self.currency_allocation);
        let position_concentration = create_position_table(&self.position_concentration);

        writeln!(f, "\n{risk_summary}")?;
        writeln!(f, "\n{asset_allocation}")?;
        writeln!(f, "\n{currency_diversification}")?;
        writeln!(f, "\n{position_concentration}")?;

        Ok(())
    }
}

impl Display for RebalancingAnalysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table = create_rebalancing_table(self);
        writeln!(f, "\n{table}")?;
        Ok(())
    }
}

/// Creates the asset allocation table
fn create_asset_allocation_table(allocation: &AssetAllocation) -> Table {
    let mut table = ux::new_table();

    // Header
    let title = Cell::new("Asset Allocation")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Column headers
    table.add_row([
        Cell::new("Asset Type").add_attribute(Attribute::Bold),
        Cell::new("Value").add_attribute(Attribute::Bold),
        Cell::new("%").add_attribute(Attribute::Bold),
    ]);

    // Helper to format percentage
    let fmt_pct = |p: Decimal| -> String { ux::format_decimal(p).unwrap_or_default() };

    // Add rows for each asset type
    let assets = [
        (
            &allocation.bonds.name,
            &allocation.bonds.value,
            allocation.bonds.percentage,
        ),
        (
            &allocation.shares.name,
            &allocation.shares.value,
            allocation.shares.percentage,
        ),
        (
            &allocation.etfs.name,
            &allocation.etfs.value,
            allocation.etfs.percentage,
        ),
        (
            &allocation.currencies.name,
            &allocation.currencies.value,
            allocation.currencies.percentage,
        ),
        (
            &allocation.futures.name,
            &allocation.futures.value,
            allocation.futures.percentage,
        ),
    ];

    for (name, value, pct) in assets {
        table.add_row([
            Cell::new(name),
            Cell::new(value.to_string()),
            Cell::new(format!("{}%", fmt_pct(pct))),
        ]);
    }

    // Total row
    table.add_row([
        Cell::new("Total").add_attribute(Attribute::Bold),
        Cell::new(allocation.total_value.to_string()).add_attribute(Attribute::Bold),
        Cell::new("100%").add_attribute(Attribute::Bold),
    ]);

    table
}

/// Creates the currency diversification table
fn create_currency_table(allocation: &CurrencyAllocation) -> Table {
    let mut table = ux::new_table();

    // Header
    let title = Cell::new("Currency Diversification")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Summary row
    let hhi_formatted = ux::format_decimal(allocation.hhi).unwrap_or_default();
    let mut hhi_cell = Cell::new(hhi_formatted);
    // Lower HHI is better (more diversified)
    if allocation.hhi < dec!(0.25) {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkGreen);
    } else if allocation.hhi < dec!(0.5) {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkYellow);
    } else {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkRed);
    }

    table.add_row([
        Cell::new("Currencies"),
        Cell::new(allocation.currency_count.to_string()),
    ]);
    table.add_row([Cell::new("HHI"), hhi_cell]);

    // Column headers for allocations
    table.add_row([
        Cell::new("Currency").add_attribute(Attribute::Bold),
        Cell::new("Value").add_attribute(Attribute::Bold),
        Cell::new("%").add_attribute(Attribute::Bold),
    ]);

    // Add currency rows
    for item in &allocation.allocations {
        table.add_row([
            Cell::new(item.currency.code()),
            Cell::new(item.value.to_string()),
            Cell::new(format!(
                "{}%",
                ux::format_decimal(item.percentage).unwrap_or_default()
            )),
        ]);
    }

    table
}

/// Creates the position concentration table
fn create_position_table(concentration: &PositionConcentration) -> Table {
    let mut table = ux::new_table();

    // Header
    let title = Cell::new("Position Concentration")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Summary row
    let hhi_formatted = ux::format_decimal(concentration.hhi).unwrap_or_default();
    let mut hhi_cell = Cell::new(hhi_formatted);
    if concentration.hhi < dec!(0.25) {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkGreen);
    } else if concentration.hhi < dec!(0.5) {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkYellow);
    } else {
        hhi_cell = hhi_cell.fg(comfy_table::Color::DarkRed);
    }

    table.add_row([
        Cell::new("Total Positions"),
        Cell::new(concentration.total_positions.to_string()),
    ]);
    table.add_row([
        Cell::new("Top 5"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(concentration.top_5_percentage).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("Top 10"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(concentration.top_10_percentage).unwrap_or_default()
        )),
    ]);
    table.add_row([Cell::new("HHI"), hhi_cell]);
    table.add_row([Cell::new("")]);

    // Column headers for positions
    table.add_row([
        Cell::new("#").add_attribute(Attribute::Bold),
        Cell::new("Name").add_attribute(Attribute::Bold),
        Cell::new("Ticker").add_attribute(Attribute::Bold),
        Cell::new("Type").add_attribute(Attribute::Bold),
        Cell::new("Value").add_attribute(Attribute::Bold),
        Cell::new("%").add_attribute(Attribute::Bold),
    ]);

    // Add position rows
    for (i, item) in concentration.top_positions.iter().enumerate() {
        table.add_row([
            Cell::new((i + 1).to_string()),
            Cell::new(&item.name),
            Cell::new(&item.ticker),
            Cell::new(item.instrument_type),
            Cell::new(item.value.to_string()),
            Cell::new(format!(
                "{}%",
                ux::format_decimal(item.percentage).unwrap_or_default()
            )),
        ]);
    }

    table
}

/// Creates the rebalancing recommendations table
fn create_rebalancing_table(analysis: &RebalancingAnalysis) -> Table {
    let mut table = ux::new_table();

    // Header
    let title = Cell::new("Rebalancing Recommendations")
        .add_attribute(Attribute::Bold)
        .fg(comfy_table::Color::DarkBlue);
    table.set_header([title]);

    // Summary
    table.add_row([
        Cell::new("Max Deviation"),
        Cell::new(format!(
            "{}%",
            ux::format_decimal(analysis.max_deviation).unwrap_or_default()
        )),
    ]);
    table.add_row([
        Cell::new("Total Rebalance Value"),
        Cell::new(analysis.total_rebalance_value.to_string()),
    ]);

    let priority_color = if analysis.priority_score < dec!(25) {
        comfy_table::Color::DarkGreen
    } else if analysis.priority_score < dec!(50) {
        comfy_table::Color::DarkYellow
    } else if analysis.priority_score < dec!(75) {
        comfy_table::Color::Yellow
    } else {
        comfy_table::Color::DarkRed
    };

    table.add_row([
        Cell::new("Priority Score"),
        Cell::new(format!(
            "{} / 100",
            ux::format_decimal(analysis.priority_score).unwrap_or_default()
        ))
        .fg(priority_color),
    ]);
    table.add_row([Cell::new("")]);

    // Column headers
    table.add_row([
        Cell::new("Asset").add_attribute(Attribute::Bold),
        Cell::new("Current %").add_attribute(Attribute::Bold),
        Cell::new("Target %").add_attribute(Attribute::Bold),
        Cell::new("Deviation").add_attribute(Attribute::Bold),
        Cell::new("Action").add_attribute(Attribute::Bold),
        Cell::new("Value").add_attribute(Attribute::Bold),
    ]);

    // Add rows for each recommendation
    for rec in &analysis.recommendations {
        let action_str = match &rec.action {
            RebalanceAction::Buy => "BUY",
            RebalanceAction::Sell => "SELL",
            RebalanceAction::Hold => "HOLD",
        };

        let mut action_cell = Cell::new(action_str);
        match &rec.action {
            RebalanceAction::Buy => action_cell = action_cell.fg(comfy_table::Color::DarkGreen),
            RebalanceAction::Sell => action_cell = action_cell.fg(comfy_table::Color::DarkRed),
            RebalanceAction::Hold => action_cell = action_cell.fg(comfy_table::Color::DarkGrey),
        }

        let deviation_str = format!(
            "{}{}%",
            if rec.deviation > dec!(0) { "+" } else { "" },
            ux::format_decimal(rec.deviation).unwrap_or_default()
        );

        let mut deviation_cell = Cell::new(deviation_str);
        if rec.deviation.abs() < dec!(5) {
            deviation_cell = deviation_cell.fg(comfy_table::Color::DarkGreen);
        } else if rec.deviation.abs() < dec!(10) {
            deviation_cell = deviation_cell.fg(comfy_table::Color::DarkYellow);
        } else {
            deviation_cell = deviation_cell.fg(comfy_table::Color::DarkRed);
        }

        table.add_row([
            Cell::new(rec.asset_type),
            Cell::new(format!(
                "{}%",
                ux::format_decimal(rec.current_percentage).unwrap_or_default()
            )),
            Cell::new(format!(
                "{}%",
                ux::format_decimal(rec.target_percentage).unwrap_or_default()
            )),
            deviation_cell,
            action_cell,
            Cell::new(rec.rebalance_value.to_string()),
        ]);
    }

    table
}
