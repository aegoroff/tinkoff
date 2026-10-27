//! Display implementation for the bond screener.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use comfy_table::{Attribute, Cell};

use super::super::screener::{BondScreen, ScreenedBond, YieldHorizon};
use super::super::xirr::AnnualRate;
use crate::ux;

fn date(date: Option<DateTime<Utc>>) -> String {
    date.map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Next coupon with its annual rate of the nominal, e.g. `35.4 ₽ (7.08%)`.
fn coupon(bond: &ScreenedBond) -> String {
    match (bond.next_coupon, bond.coupon_rate()) {
        (Some(coupon), Some(rate)) => format!("{coupon} ({}%)", rate.round_dp(2)),
        (Some(coupon), None) => coupon.to_string(),
        (None, _) => String::new(),
    }
}

/// Features of a bond, e.g. `amortization, subordinated`.
fn notes(bond: &ScreenedBond) -> String {
    bond.bond
        .features
        .iter()
        .map(|f| f.name())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Display for BondScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut table = ux::new_table();
        table.set_header([Cell::new("Bond Screener")
            .add_attribute(Attribute::Bold)
            .fg(comfy_table::Color::DarkBlue)]);
        table.add_row(
            [
                "Ticker", "Name", "Price", "Yield", "To", "Offer", "Maturity", "Duration",
                "Coupon", "Coupons", "Risk", "Sector", "Notes",
            ]
            .map(|h| Cell::new(h).add_attribute(Attribute::Bold)),
        );
        for bond in &self.bonds {
            let (rate, to) = match bond.yield_to_worst() {
                Some((rate, YieldHorizon::Offer)) => (Some(rate), "offer"),
                Some((rate, YieldHorizon::Maturity)) => (Some(rate), "maturity"),
                None => (None, ""),
            };
            table.add_row([
                Cell::new(&bond.bond.ticker),
                Cell::new(&bond.bond.name),
                Cell::new(format!("{}%", bond.price_percent.normalize())),
                rate.map_or_else(|| Cell::new(""), |r| ux::colored_cell(AnnualRate(r))),
                Cell::new(to),
                Cell::new(date(bond.info.next_offer_date)),
                Cell::new(date(bond.info.maturity_date)),
                Cell::new(
                    bond.info
                        .duration
                        .map(|d| d.round_dp(2).to_string())
                        .unwrap_or_default(),
                ),
                Cell::new(coupon(bond)),
                Cell::new(bond.bond.coupons_per_year),
                Cell::new(bond.bond.risk.map_or("", |r| r.name())),
                Cell::new(&bond.bond.sector),
                Cell::new(notes(bond)),
            ]);
        }
        writeln!(f, "\n{table}")?;
        writeln!(
            f,
            "{} of {} bonds matching the listing criteria, {} with yields calculated.\n\
             Bonds already in the portfolio, with floating coupons and perpetual ones are \
             left out. Yield is effective annual to worst: the lower of the yields to the next \
             offer, which may be the issuer's call, and to maturity; duration is Macaulay's \
             in years. Coupon is the next \
             announced one with its annual rate of the nominal. Risk is the broker's risk \
             level, not a credit rating.",
            self.bonds.len(),
            self.listed,
            self.checked
        )
    }
}
