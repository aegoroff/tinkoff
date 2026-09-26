use std::collections::HashMap;
use std::str::FromStr;

use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::money::Money;
use super::paper::Ticker;
use super::portfolio::Portfolio;
use crate::domain::LoadedPaper;

/// Risk analysis results for a portfolio
#[derive(Debug, Clone)]
pub struct RiskAnalysis {
    /// Asset allocation by type (bonds, shares, etfs, etc.)
    pub asset_allocation: AssetAllocation,
    /// Currency diversification analysis
    pub currency_allocation: CurrencyAllocation,
    /// Sector diversification analysis
    pub sector_allocation: SectorAllocation,
    /// Position concentration (top holdings)
    pub position_concentration: PositionConcentration,
    /// Risk metrics summary
    pub risk_metrics: RiskMetrics,
}

/// Asset allocation breakdown by instrument type
#[derive(Debug, Clone)]
pub struct AssetAllocation {
    pub bonds: AllocationItem,
    pub shares: AllocationItem,
    pub etfs: AllocationItem,
    pub currencies: AllocationItem,
    pub futures: AllocationItem,
    pub total_value: Money,
}

/// Single allocation item with value and percentage
#[derive(Debug, Clone)]
pub struct AllocationItem {
    pub name: &'static str,
    pub value: Money,
    pub percentage: Decimal,
}

/// Currency diversification analysis
#[derive(Debug, Clone)]
pub struct CurrencyAllocation {
    pub allocations: Vec<CurrencyItem>,
    pub total_value: Money,
    /// Number of different currencies
    pub currency_count: usize,
    /// Herfindahl-Hirschman Index for currency concentration (0-1, lower is better diversified)
    pub hhi: Decimal,
}

/// Single currency allocation item
#[derive(Debug, Clone)]
pub struct CurrencyItem {
    pub currency: Currency,
    /// Value of papers exposed to `currency`, in RUB
    pub value: Money,
    pub percentage: Decimal,
}

/// Sector diversification analysis of all papers except currencies
#[derive(Debug, Clone)]
pub struct SectorAllocation {
    /// Sorted by value descending
    pub allocations: Vec<SectorItem>,
    pub total_value: Money,
    /// Herfindahl-Hirschman Index for sector concentration (0-1, lower is better diversified)
    pub hhi: Decimal,
}

/// Single sector allocation item
#[derive(Debug, Clone)]
pub struct SectorItem {
    /// Sector code from the API; `None` for papers of unknown sector
    pub sector: Option<String>,
    /// Value of papers of the sector, in RUB
    pub value: Money,
    pub percentage: Decimal,
}

/// Position concentration analysis
#[derive(Debug, Clone)]
pub struct PositionConcentration {
    /// Top 5 positions by value
    pub top_positions: Vec<PositionItem>,
    /// Top 5 positions percentage of total portfolio
    pub top_5_percentage: Decimal,
    /// Top 10 positions percentage of total portfolio
    pub top_10_percentage: Decimal,
    /// Herfindahl-Hirschman Index for position concentration (0-1, lower is better diversified)
    pub hhi: Decimal,
    pub total_positions: usize,
    pub total_value: Money,
}

/// Single position in concentration analysis
#[derive(Debug, Clone)]
pub struct PositionItem {
    pub name: String,
    pub ticker: Ticker,
    pub instrument_type: &'static str,
    /// Current value in RUB
    pub value: Money,
    pub percentage: Decimal,
}

/// Summary risk metrics
#[derive(Debug, Clone)]
pub struct RiskMetrics {
    /// Overall diversification score (0-100, higher is better)
    pub diversification_score: Decimal,
    /// Currency risk level (0-100, lower is better)
    pub currency_risk: Decimal,
    /// Concentration risk level (0-100, lower is better)
    pub concentration_risk: Decimal,
    /// Asset type concentration risk (0-100, lower is better)
    pub asset_concentration_risk: Decimal,
    /// Sector concentration risk (0-100, lower is better); `None` without papers having sectors
    pub sector_risk: Option<Decimal>,
    /// Risk level assessment from allocation / HHI metrics only
    pub risk_level: RiskLevel,
}

/// Risk level assessment
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    VeryHigh,
}

/// Deviation from the target, in percentage points, that triggers a buy or sell recommendation.
pub const REBALANCE_THRESHOLD: Decimal = dec!(5);

/// Decimal places deviations are rounded to, the same as they are displayed with.
const DEVIATION_DECIMAL_PLACES: u32 = 2;

/// Asset type names accepted in a target allocation string.
pub const TARGET_ASSET_TYPES: &str = "bonds, shares, etfs, currencies, futures";

/// Target allocation for portfolio rebalancing
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAllocation {
    /// Target percentage for bonds (0-100)
    pub bonds: Decimal,
    /// Target percentage for shares (0-100)
    pub shares: Decimal,
    /// Target percentage for ETFs (0-100)
    pub etfs: Decimal,
    /// Target percentage for currencies (0-100)
    pub currencies: Decimal,
    /// Target percentage for futures (0-100)
    pub futures: Decimal,
}

/// Named target allocations accepted by `--target` instead of explicit percents.
pub const TARGET_PRESETS: [(&str, TargetAllocation); 2] = [
    (
        "conservative",
        TargetAllocation {
            bonds: dec!(60),
            shares: dec!(30),
            etfs: dec!(5),
            currencies: dec!(5),
            futures: dec!(0),
        },
    ),
    (
        "balanced",
        TargetAllocation {
            bonds: dec!(40),
            shares: dec!(40),
            etfs: dec!(10),
            currencies: dec!(5),
            futures: dec!(5),
        },
    ),
];

impl TargetAllocation {
    /// Describes [`TARGET_PRESETS`] for CLI help, e.g. `balanced (bonds 40, shares 40, ...)`.
    #[must_use]
    pub fn presets_help() -> String {
        TARGET_PRESETS
            .iter()
            .map(|(name, t)| {
                format!(
                    "{name} (bonds {}, shares {}, etfs {}, currencies {}, futures {})",
                    t.bonds, t.shares, t.etfs, t.currencies, t.futures
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl FromStr for TargetAllocation {
    type Err = String;

    /// Parses a preset name from [`TARGET_PRESETS`] or `bonds=60,shares=30,etfs=10` style target.
    ///
    /// Names and keys are case-insensitive, keys accept singular forms; omitted asset types
    /// get 0%. Percentages must be within 0..=100 and sum up to exactly 100.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((_, preset)) = TARGET_PRESETS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s.trim()))
        {
            return Ok(preset.clone());
        }

        let mut target = Self {
            bonds: Decimal::ZERO,
            shares: Decimal::ZERO,
            etfs: Decimal::ZERO,
            currencies: Decimal::ZERO,
            futures: Decimal::ZERO,
        };
        let mut seen: Vec<&'static str> = Vec::new();

        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                let presets = TARGET_PRESETS.map(|(name, _)| name).join(", ");
                format!("'{part}' is neither a preset ({presets}) nor asset=percent, e.g. bonds=60")
            })?;
            let (name, slot) = match key.trim().to_ascii_lowercase().as_str() {
                "bonds" | "bond" => ("bonds", &mut target.bonds),
                "shares" | "share" => ("shares", &mut target.shares),
                "etfs" | "etf" => ("etfs", &mut target.etfs),
                "currencies" | "currency" => ("currencies", &mut target.currencies),
                "futures" | "future" => ("futures", &mut target.futures),
                other => {
                    return Err(format!(
                        "unknown asset type '{other}'; expected one of: {TARGET_ASSET_TYPES}"
                    ));
                }
            };
            if seen.contains(&name) {
                return Err(format!("{name} is set more than once"));
            }
            seen.push(name);

            let percent = Decimal::from_str(value.trim())
                .map_err(|_| format!("'{}' is not a number in '{part}'", value.trim()))?;
            if percent < Decimal::ZERO || percent > dec!(100) {
                return Err(format!("{name}={percent}% must be within 0..100"));
            }
            *slot = percent;
        }

        let sum = target.bonds + target.shares + target.etfs + target.currencies + target.futures;
        if sum != dec!(100) {
            return Err(format!(
                "target percentages sum up to {sum}%, expected 100%"
            ));
        }
        Ok(target)
    }
}

/// Rebalancing recommendation for a single asset
#[derive(Debug, Clone)]
pub struct RebalanceRecommendation {
    /// Asset type name
    pub asset_type: &'static str,
    /// Current percentage in portfolio
    pub current_percentage: Decimal,
    /// Target percentage
    pub target_percentage: Decimal,
    /// Deviation from target (positive = overweight, negative = underweight)
    pub deviation: Decimal,
    /// Recommended action
    pub action: RebalanceAction,
    /// Value to buy/sell to rebalance
    pub rebalance_value: Money,
}

/// Action to take for rebalancing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebalanceAction {
    /// Buy to increase position
    Buy,
    /// Sell to decrease position
    Sell,
    /// No action needed
    Hold,
}

/// Portfolio rebalancing analysis
#[derive(Debug, Clone)]
pub struct RebalancingAnalysis {
    /// Total portfolio value
    pub total_value: Money,
    /// Recommendations for each asset type
    pub recommendations: Vec<RebalanceRecommendation>,
    /// Maximum deviation from target (absolute value)
    pub max_deviation: Decimal,
    /// Total value to rebalance
    pub total_rebalance_value: Money,
    /// Rebalancing priority score (0-100, higher = more urgent)
    pub priority_score: Decimal,
}

impl RebalancingAnalysis {
    /// Analyze portfolio and generate rebalancing recommendations
    #[must_use]
    pub fn analyze(asset_allocation: &AssetAllocation, target: &TargetAllocation) -> Self {
        let total_value = asset_allocation.total_value;
        let currency = total_value.currency;

        // Calculate recommendations for each asset type
        let mut recommendations = Vec::with_capacity(5);
        let mut max_deviation = dec!(0);
        let mut total_rebalance_value = dec!(0);

        let assets = [
            ("Bonds", asset_allocation.bonds.percentage, target.bonds),
            ("Shares", asset_allocation.shares.percentage, target.shares),
            ("ETFs", asset_allocation.etfs.percentage, target.etfs),
            (
                "Currencies",
                asset_allocation.currencies.percentage,
                target.currencies,
            ),
            (
                "Futures",
                asset_allocation.futures.percentage,
                target.futures,
            ),
        ];

        for (asset_type, current_pct, target_pct) in assets {
            // Rounded as displayed, so the action always matches the shown deviation.
            let deviation = (current_pct - target_pct).round_dp(DEVIATION_DECIMAL_PLACES);
            let abs_deviation = deviation.abs();

            if abs_deviation > max_deviation {
                max_deviation = abs_deviation;
            }

            // Calculate the value to rebalance
            let target_value = (target_pct / dec!(100)) * total_value.value;
            let current_value = (current_pct / dec!(100)) * total_value.value;
            let rebalance_amount = (target_value - current_value).abs();

            let (action, rebalance_value) = if abs_deviation < REBALANCE_THRESHOLD {
                (RebalanceAction::Hold, Money::zero(currency))
            } else if deviation > dec!(0) {
                (
                    RebalanceAction::Sell,
                    Money::from_value(rebalance_amount, currency),
                )
            } else {
                (
                    RebalanceAction::Buy,
                    Money::from_value(rebalance_amount, currency),
                )
            };

            total_rebalance_value += rebalance_value.value;

            recommendations.push(RebalanceRecommendation {
                asset_type,
                current_percentage: current_pct,
                target_percentage: target_pct,
                deviation,
                action,
                rebalance_value,
            });
        }

        // Calculate priority score based on max deviation
        // 0-5% = 0-25, 5-10% = 25-50, 10-15% = 50-75, 15%+ = 75-100
        let priority_score = (max_deviation * dec!(5)).min(dec!(100));

        Self {
            total_value,
            recommendations,
            max_deviation,
            total_rebalance_value: Money::from_value(total_rebalance_value, currency),
            priority_score,
        }
    }
}

impl RiskAnalysis {
    /// Analyze portfolio risk metrics
    #[must_use]
    pub fn analyze(portfolio: &Portfolio) -> Self {
        let papers: Vec<LoadedPaper> = portfolio.papers().collect();
        let asset_allocation = AssetAllocation::from_portfolio(portfolio);
        let currency_allocation = CurrencyAllocation::from_papers(&papers);
        let sector_allocation = SectorAllocation::from_papers(&papers);
        let position_concentration = PositionConcentration::from_papers(&papers);
        let risk_metrics = RiskMetrics::calculate(
            &asset_allocation,
            &currency_allocation,
            &sector_allocation,
            &position_concentration,
        );

        Self {
            asset_allocation,
            currency_allocation,
            sector_allocation,
            position_concentration,
            risk_metrics,
        }
    }
}

impl AssetAllocation {
    #[must_use]
    fn from_portfolio(portfolio: &Portfolio) -> Self {
        let bonds_value = portfolio.bonds.current();
        let shares_value = portfolio.shares.current();
        let etfs_value = portfolio.etfs.current();
        let currencies_value = portfolio.currencies.current();
        let futures_value = portfolio.futures.current();

        let total_value =
            bonds_value + shares_value + etfs_value + currencies_value + futures_value;

        let calc_item = |name: &'static str, value: Money| -> AllocationItem {
            let percentage = if total_value.value.is_zero() {
                dec!(0)
            } else {
                (value.value / total_value.value) * dec!(100)
            };
            AllocationItem {
                name,
                value,
                percentage,
            }
        };

        Self {
            bonds: calc_item("Bonds", bonds_value),
            shares: calc_item("Shares", shares_value),
            etfs: calc_item("ETFs", etfs_value),
            currencies: calc_item("Currencies", currencies_value),
            futures: calc_item("Futures", futures_value),
            total_value,
        }
    }
}

impl CurrencyAllocation {
    #[must_use]
    fn from_papers(papers: &[LoadedPaper]) -> Self {
        let mut currency_map: HashMap<Currency, Decimal> = HashMap::new();
        let mut total_value = Decimal::ZERO;

        for paper in papers {
            let (value, currency) = paper.current_value_and_nominal_currency();
            *currency_map.entry(currency).or_default() += value;
            total_value += value;
        }

        let mut allocations: Vec<CurrencyItem> = currency_map
            .into_iter()
            .map(|(currency, value)| {
                let percentage = if total_value.is_zero() {
                    dec!(0)
                } else {
                    (value / total_value) * dec!(100)
                };
                CurrencyItem {
                    currency,
                    // Paper values are converted to RUB, the currency is only their exposure.
                    value: Money::from_value(value, Currency::RUB),
                    percentage,
                }
            })
            .collect();

        // Sort by value descending
        allocations.sort_by_key(|b| std::cmp::Reverse(b.value.value));

        let currency_count = allocations.len();

        // Calculate HHI (Herfindahl-Hirschman Index)
        let hhi = allocations.iter().fold(dec!(0), |acc, item| {
            let share = item.percentage / dec!(100);
            acc + share * share
        });

        let total_money = Money::from_value(total_value, Currency::RUB);

        Self {
            allocations,
            total_value: total_money,
            currency_count,
            hhi,
        }
    }
}

impl SectorAllocation {
    #[must_use]
    fn from_papers(papers: &[LoadedPaper]) -> Self {
        let mut sector_map: HashMap<Option<&str>, Decimal> = HashMap::new();
        let mut total_value = Decimal::ZERO;

        // Currencies are cash, not an exposure to any sector.
        for paper in papers
            .iter()
            .filter(|p| !matches!(p, LoadedPaper::Currency(_)))
        {
            let (value, _) = paper.current_value_and_nominal_currency();
            *sector_map.entry(paper.sector()).or_default() += value;
            total_value += value;
        }

        let mut allocations: Vec<SectorItem> = sector_map
            .into_iter()
            .map(|(sector, value)| SectorItem {
                sector: sector.map(str::to_string),
                value: Money::from_value(value, Currency::RUB),
                percentage: percentage_of(value, total_value),
            })
            .collect();
        allocations.sort_by(|a, b| {
            b.value
                .value
                .cmp(&a.value.value)
                .then_with(|| a.sector.cmp(&b.sector))
        });

        let hhi = allocations.iter().fold(dec!(0), |acc, item| {
            let share = item.percentage / dec!(100);
            acc + share * share
        });

        Self {
            allocations,
            total_value: Money::from_value(total_value, Currency::RUB),
            hhi,
        }
    }
}

/// Percentage of `value` in `total`; zero for zero `total`.
fn percentage_of(value: Decimal, total: Decimal) -> Decimal {
    if total.is_zero() {
        dec!(0)
    } else {
        (value / total) * dec!(100)
    }
}

impl PositionConcentration {
    #[must_use]
    fn from_papers(papers: &[LoadedPaper]) -> Self {
        let mut position_values: Vec<(String, &Ticker, &'static str, Decimal)> = papers
            .iter()
            .map(|paper| match paper {
                LoadedPaper::Bond(p) => (p.name.clone(), &p.ticker, "Bond", p.current().value),
                LoadedPaper::Share(p) => (p.name.clone(), &p.ticker, "Share", p.current().value),
                LoadedPaper::Etf(p) => (p.name.clone(), &p.ticker, "ETF", p.current().value),
                LoadedPaper::Currency(p) => {
                    (p.name.clone(), &p.ticker, "Currency", p.current().value)
                }
                LoadedPaper::Future(p) => (p.name.clone(), &p.ticker, "Future", p.current().value),
            })
            .collect();

        let total_value: Decimal = position_values.iter().map(|(_, _, _, v)| v).sum();
        let total_positions = position_values.len();

        // Sort by value descending
        position_values.sort_by_key(|b| std::cmp::Reverse(b.3));

        // Calculate percentages and create PositionItem list
        let mut items: Vec<PositionItem> = position_values
            .iter()
            .map(|(name, ticker, instrument_type, value)| {
                let percentage = if total_value.is_zero() {
                    dec!(0)
                } else {
                    (*value / total_value) * dec!(100)
                };
                PositionItem {
                    name: name.clone(),
                    ticker: (*ticker).clone(),
                    instrument_type,
                    value: Money::from_value(*value, Currency::RUB),
                    percentage,
                }
            })
            .collect();

        // Calculate top 5 and top 10 percentages
        let top_5_percentage: Decimal = items.iter().take(5).map(|i| i.percentage).sum();
        let top_10_percentage: Decimal = items.iter().take(10).map(|i| i.percentage).sum();

        // Calculate HHI
        let hhi = items.iter().fold(dec!(0), |acc, item| {
            let share = item.percentage / dec!(100);
            acc + share * share
        });

        // Keep only top 10 for display
        items.truncate(10);

        let total_money = Money::from_value(total_value, Currency::RUB);

        Self {
            top_positions: items,
            top_5_percentage,
            top_10_percentage,
            hhi,
            total_positions,
            total_value: total_money,
        }
    }
}

impl RiskMetrics {
    #[must_use]
    fn calculate(
        asset_alloc: &AssetAllocation,
        currency_alloc: &CurrencyAllocation,
        sector_alloc: &SectorAllocation,
        position_conc: &PositionConcentration,
    ) -> Self {
        let sector_risk = Self::calculate_sector_risk(sector_alloc);
        let diversification_score = Self::calculate_diversification_score(
            asset_alloc,
            currency_alloc,
            sector_risk,
            position_conc,
        );
        let currency_risk = Self::calculate_currency_risk(currency_alloc);
        let concentration_risk = Self::calculate_concentration_risk(position_conc);
        let asset_concentration_risk = Self::calculate_asset_concentration_risk(asset_alloc);
        let risk_level = Self::assess_risk_level(
            diversification_score,
            currency_risk,
            concentration_risk,
            asset_concentration_risk,
        );

        Self {
            diversification_score,
            currency_risk,
            concentration_risk,
            asset_concentration_risk,
            sector_risk,
            risk_level,
        }
    }

    fn calculate_diversification_score(
        asset_alloc: &AssetAllocation,
        currency_alloc: &CurrencyAllocation,
        sector_risk: Option<Decimal>,
        position_conc: &PositionConcentration,
    ) -> Decimal {
        // Weight factors for diversification calculation
        let asset_diversification =
            dec!(100) - Self::calculate_asset_concentration_risk(asset_alloc);
        let currency_diversification = dec!(100) - currency_alloc.hhi * dec!(100);
        let position_diversification = dec!(100) - position_conc.hhi * dec!(100);

        // Weighted average (positions matter most, then assets, then currency and sectors)
        let weighted = asset_diversification * dec!(3)
            + currency_diversification * dec!(2)
            + position_diversification * dec!(5);
        match sector_risk {
            Some(risk) => (weighted + (dec!(100) - risk) * dec!(2)) / dec!(12),
            None => weighted / dec!(10),
        }
    }

    fn calculate_sector_risk(sector_alloc: &SectorAllocation) -> Option<Decimal> {
        let known = sector_alloc.allocations.iter().any(|i| i.sector.is_some());
        known.then(|| sector_alloc.hhi * dec!(100))
    }

    fn calculate_currency_risk(currency_alloc: &CurrencyAllocation) -> Decimal {
        // Currency risk based on HHI and number of currencies
        let hhi_risk = currency_alloc.hhi * dec!(100);

        // Penalty for low currency count
        let count_penalty = if currency_alloc.currency_count == 0 {
            dec!(50)
        } else if currency_alloc.currency_count == 1 {
            dec!(30)
        } else if currency_alloc.currency_count == 2 {
            dec!(15)
        } else {
            dec!(0)
        };

        (hhi_risk + count_penalty).min(dec!(100))
    }

    fn calculate_concentration_risk(position_conc: &PositionConcentration) -> Decimal {
        // Concentration risk based on HHI and top holdings
        let hhi_risk = position_conc.hhi * dec!(100);
        let top_5_risk = position_conc.top_5_percentage;

        // Weighted average
        (hhi_risk * dec!(4) + top_5_risk) / dec!(5)
    }

    fn calculate_asset_concentration_risk(asset_alloc: &AssetAllocation) -> Decimal {
        // Calculate HHI for asset types
        let percentages = [
            asset_alloc.bonds.percentage,
            asset_alloc.shares.percentage,
            asset_alloc.etfs.percentage,
            asset_alloc.currencies.percentage,
            asset_alloc.futures.percentage,
        ];

        let hhi: Decimal = percentages.iter().fold(dec!(0), |acc, &p| {
            let share = p / dec!(100);
            acc + share * share
        });

        // Count non-zero asset types
        let non_zero_count = percentages.iter().filter(|&&p| !p.is_zero()).count();

        // Penalty for low asset type count
        let count_penalty = if non_zero_count <= 1 {
            dec!(40)
        } else if non_zero_count == 2 {
            dec!(20)
        } else {
            dec!(0)
        };

        (hhi * dec!(100) + count_penalty).min(dec!(100))
    }

    fn assess_risk_level(
        diversification_score: Decimal,
        currency_risk: Decimal,
        concentration_risk: Decimal,
        asset_concentration_risk: Decimal,
    ) -> RiskLevel {
        let avg_risk = (currency_risk + concentration_risk + asset_concentration_risk) / dec!(3);
        let diversification_bonus = diversification_score / dec!(10);

        let final_risk = (avg_risk - diversification_bonus)
            .max(dec!(0))
            .min(dec!(100));

        if final_risk < dec!(25) {
            RiskLevel::Low
        } else if final_risk < dec!(50) {
            RiskLevel::Medium
        } else if final_risk < dec!(75) {
            RiskLevel::High
        } else {
            RiskLevel::VeryHigh
        }
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::{
        CouponProfit, DividendProfit, Figi, LoadedPaper, NoneProfit, Paper, Position, Profit,
        Ticker, Totals,
    };

    #[test]
    fn test_asset_allocation_calculation() {
        let currency = Currency::RUB;
        let mut portfolio = Portfolio::new(false);

        // Add a bond with current value 500
        let mut bonds = portfolio.bonds;
        bonds.add_paper(Paper {
            name: "Bond 1".to_string(),
            ticker: Ticker::new("BOND1".to_string()),
            figi: Figi::new("bond1figi".to_string()),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(5), currency),
                current_instrument_price: Money::from_value(dec!(5), currency),
                accrued_interest: Money::zero(currency),
                quantity: dec!(100),
                daily_yield: Money::zero(currency),
                blocked: false,
                blocked_lots: dec!(0),
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
                cash_flows: vec![],
            },
            profit: CouponProfit,
            bond: None,
            sector: None,
        });
        portfolio.bonds = bonds;

        // Add a share with current value 500
        let mut shares = portfolio.shares;
        shares.add_paper(Paper {
            name: "Share 1".to_string(),
            ticker: Ticker::new("SHARE1".to_string()),
            figi: Figi::new("share1figi".to_string()),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(5), currency),
                current_instrument_price: Money::from_value(dec!(5), currency),
                accrued_interest: Money::zero(currency),
                quantity: dec!(100),
                daily_yield: Money::zero(currency),
                blocked: false,
                blocked_lots: dec!(0),
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
                cash_flows: vec![],
            },
            profit: DividendProfit,
            bond: None,
            sector: None,
        });
        portfolio.shares = shares;

        let allocation = AssetAllocation::from_portfolio(&portfolio);

        assert_eq!(allocation.bonds.percentage, dec!(50));
        assert_eq!(allocation.shares.percentage, dec!(50));
        assert_eq!(allocation.etfs.percentage, dec!(0));
        assert_eq!(allocation.currencies.percentage, dec!(0));
        assert_eq!(allocation.futures.percentage, dec!(0));
    }

    #[test]
    fn test_currency_allocation_single_currency() {
        let papers = vec![LoadedPaper::Share(Paper {
            name: "Share 1".to_string(),
            ticker: Ticker::new("SHARE1".to_string()),
            figi: Figi::new("share1figi".to_string()),
            position: Position {
                currency: Currency::RUB,
                average_buy_price: Money::from_value(dec!(100), Currency::RUB),
                current_instrument_price: Money::from_value(dec!(100), Currency::RUB),
                accrued_interest: Money::zero(Currency::RUB),
                quantity: dec!(10),
                daily_yield: Money::zero(Currency::RUB),
                blocked: false,
                blocked_lots: dec!(0),
            },
            totals: Totals {
                additional_profit: Money::zero(Currency::RUB),
                fees: Money::zero(Currency::RUB),
                cash_flows: vec![],
            },
            profit: DividendProfit,
            bond: None,
            sector: None,
        })];

        let allocation = CurrencyAllocation::from_papers(&papers);

        assert_eq!(allocation.currency_count, 1);
        assert_eq!(allocation.hhi, dec!(1)); // HHI = 1.0 for single currency
    }

    #[test]
    fn test_currency_allocation_diversified() {
        let papers = vec![
            LoadedPaper::Share(Paper {
                name: "Share 1".to_string(),
                ticker: Ticker::new("SHARE1".to_string()),
                figi: Figi::new("share1figi".to_string()),
                position: Position {
                    currency: Currency::RUB,
                    average_buy_price: Money::from_value(dec!(50), Currency::RUB),
                    current_instrument_price: Money::from_value(dec!(50), Currency::RUB),
                    accrued_interest: Money::zero(Currency::RUB),
                    quantity: dec!(10),
                    daily_yield: Money::zero(Currency::RUB),
                    blocked: false,
                    blocked_lots: dec!(0),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                    cash_flows: vec![],
                },
                profit: DividendProfit,
                bond: None,
                sector: None,
            }),
            LoadedPaper::Share(Paper {
                name: "Share 2".to_string(),
                ticker: Ticker::new("SHARE2".to_string()),
                figi: Figi::new("share2figi".to_string()),
                position: Position {
                    currency: Currency::USD,
                    average_buy_price: Money::from_value(dec!(50), Currency::USD),
                    current_instrument_price: Money::from_value(dec!(50), Currency::USD),
                    accrued_interest: Money::zero(Currency::USD),
                    quantity: dec!(10),
                    daily_yield: Money::zero(Currency::USD),
                    blocked: false,
                    blocked_lots: dec!(0),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::USD),
                    fees: Money::zero(Currency::USD),
                    cash_flows: vec![],
                },
                profit: DividendProfit,
                bond: None,
                sector: None,
            }),
        ];

        let allocation = CurrencyAllocation::from_papers(&papers);

        assert_eq!(allocation.currency_count, 2);
        // HHI = 0.5^2 + 0.5^2 = 0.5
        assert_eq!(allocation.hhi, dec!(0.5));
    }

    /// Share exposed to `currency` with prices converted to RUB, as loaded from the API.
    fn share_priced_in_rub(ticker: &str, currency: Currency, value: Decimal) -> LoadedPaper {
        LoadedPaper::Share(rub_paper(ticker, currency, value, DividendProfit))
    }

    /// Paper of `sector` worth `value` RUB.
    fn paper_in_sector<P: Profit>(sector: Option<&str>, value: Decimal, profit: P) -> Paper<P> {
        Paper {
            sector: sector.map(str::to_string),
            ..rub_paper("TICKER", Currency::RUB, value, profit)
        }
    }

    fn rub_paper<P: Profit>(
        ticker: &str,
        currency: Currency,
        value: Decimal,
        profit: P,
    ) -> Paper<P> {
        let rub = Currency::RUB;
        Paper {
            name: ticker.to_string(),
            ticker: Ticker::new(ticker),
            figi: Figi::new(ticker),
            position: Position {
                currency,
                average_buy_price: Money::from_value(value, rub),
                current_instrument_price: Money::from_value(value, rub),
                accrued_interest: Money::zero(rub),
                quantity: dec!(1),
                daily_yield: Money::zero(rub),
                blocked: false,
                blocked_lots: dec!(0),
            },
            totals: Totals {
                additional_profit: Money::zero(rub),
                fees: Money::zero(rub),
                cash_flows: vec![],
            },
            profit,
            bond: None,
            sector: None,
        }
    }

    #[test]
    fn currency_allocation_values_are_in_rub() {
        // Arrange
        let papers = vec![
            share_priced_in_rub("USDSHARE", Currency::USD, dec!(9000)),
            share_priced_in_rub("RUBSHARE", Currency::RUB, dec!(1000)),
        ];

        // Act
        let allocation = CurrencyAllocation::from_papers(&papers);

        // Assert
        let usd = &allocation.allocations[0];
        assert_eq!(usd.currency, Currency::USD);
        assert_eq!(usd.value, Money::from_value(dec!(9000), Currency::RUB));
        assert_eq!(usd.percentage, dec!(90));
    }

    #[test]
    fn sector_allocation_groups_papers_by_sector() {
        // Arrange
        let papers = vec![
            LoadedPaper::Share(paper_in_sector(
                Some("financial"),
                dec!(300),
                DividendProfit,
            )),
            LoadedPaper::Bond(paper_in_sector(Some("financial"), dec!(300), CouponProfit)),
            LoadedPaper::Share(paper_in_sector(Some("energy"), dec!(400), DividendProfit)),
        ];

        // Act
        let allocation = SectorAllocation::from_papers(&papers);

        // Assert
        let sectors = allocation
            .allocations
            .iter()
            .map(|i| (i.sector.as_deref(), i.percentage))
            .collect::<Vec<_>>();
        assert_eq!(
            sectors,
            vec![(Some("financial"), dec!(60)), (Some("energy"), dec!(40))]
        );
        assert_eq!(
            allocation.total_value,
            Money::from_value(dec!(1000), Currency::RUB)
        );
        // HHI = 0.6^2 + 0.4^2 = 0.52
        assert_eq!(allocation.hhi, dec!(0.52));
    }

    #[test]
    fn sector_allocation_skips_currencies_and_keeps_unknown_sectors() {
        // Arrange
        let papers = vec![
            LoadedPaper::Currency(paper_in_sector(None, dec!(5000), NoneProfit)),
            LoadedPaper::Share(paper_in_sector(Some("it"), dec!(500), DividendProfit)),
            LoadedPaper::Etf(paper_in_sector(None, dec!(500), DividendProfit)),
        ];

        // Act
        let allocation = SectorAllocation::from_papers(&papers);

        // Assert
        let sectors = allocation
            .allocations
            .iter()
            .map(|i| (i.sector.as_deref(), i.percentage))
            .collect::<Vec<_>>();
        assert_eq!(sectors, vec![(None, dec!(50)), (Some("it"), dec!(50))]);
        assert_eq!(
            allocation.total_value,
            Money::from_value(dec!(1000), Currency::RUB)
        );
    }

    #[test]
    fn sector_allocation_of_empty_portfolio_is_empty() {
        // Act
        let allocation = SectorAllocation::from_papers(&[]);

        // Assert
        assert!(allocation.allocations.is_empty());
        assert_eq!(allocation.hhi, dec!(0));
    }

    #[rstest]
    #[case::single_sector(vec![Some("it")], Some(dec!(100)))]
    #[case::two_equal_sectors(vec![Some("it"), Some("energy")], Some(dec!(50)))]
    #[case::only_unknown_sectors(vec![None], None)]
    #[case::no_papers(vec![], None)]
    fn sector_risk_follows_sector_hhi(
        #[case] sectors: Vec<Option<&str>>,
        #[case] expected: Option<Decimal>,
    ) {
        // Arrange
        let papers = sectors
            .into_iter()
            .map(|s| LoadedPaper::Share(paper_in_sector(s, dec!(100), DividendProfit)))
            .collect::<Vec<_>>();
        let allocation = SectorAllocation::from_papers(&papers);

        // Act
        let risk = RiskMetrics::calculate_sector_risk(&allocation);

        // Assert
        assert_eq!(risk, expected);
    }

    #[rstest]
    #[case::without_sectors(None, dec!(70))]
    #[case::diversified_sectors(Some(dec!(0)), dec!(75))]
    #[case::single_sector(Some(dec!(100)), dec!(58.33))]
    fn diversification_score_accounts_sector_risk(
        #[case] sector_risk: Option<Decimal>,
        #[case] expected: Decimal,
    ) {
        // Arrange: bonds only, no currencies and positions
        let asset_alloc = allocation_with_currencies(dec!(0));
        let currency_alloc = CurrencyAllocation::from_papers(&[]);
        let position_conc = PositionConcentration::from_papers(&[]);

        // Act
        let score = RiskMetrics::calculate_diversification_score(
            &asset_alloc,
            &currency_alloc,
            sector_risk,
            &position_conc,
        );

        // Assert
        assert_eq!(score.round_dp(2), expected);
    }

    #[test]
    fn position_concentration_values_are_in_rub() {
        // Arrange
        let papers = vec![share_priced_in_rub("USDSHARE", Currency::USD, dec!(9000))];

        // Act
        let concentration = PositionConcentration::from_papers(&papers);

        // Assert
        assert_eq!(
            concentration.top_positions[0].value,
            Money::from_value(dec!(9000), Currency::RUB)
        );
    }

    #[test]
    fn test_position_concentration() {
        let papers = vec![
            LoadedPaper::Share(Paper {
                name: "Large Position".to_string(),
                ticker: Ticker::new("LARGE".to_string()),
                figi: Figi::new("largefigi".to_string()),
                position: Position {
                    currency: Currency::RUB,
                    average_buy_price: Money::from_value(dec!(100), Currency::RUB),
                    current_instrument_price: Money::from_value(dec!(100), Currency::RUB),
                    accrued_interest: Money::zero(Currency::RUB),
                    quantity: dec!(10),
                    daily_yield: Money::zero(Currency::RUB),
                    blocked: false,
                    blocked_lots: dec!(0),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                    cash_flows: vec![],
                },
                profit: DividendProfit,
                bond: None,
                sector: None,
            }),
            LoadedPaper::Share(Paper {
                name: "Small Position".to_string(),
                ticker: Ticker::new("SMALL".to_string()),
                figi: Figi::new("smallfigi".to_string()),
                position: Position {
                    currency: Currency::RUB,
                    average_buy_price: Money::from_value(dec!(10), Currency::RUB),
                    current_instrument_price: Money::from_value(dec!(10), Currency::RUB),
                    accrued_interest: Money::zero(Currency::RUB),
                    quantity: dec!(10),
                    daily_yield: Money::zero(Currency::RUB),
                    blocked: false,
                    blocked_lots: dec!(0),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                    cash_flows: vec![],
                },
                profit: DividendProfit,
                bond: None,
                sector: None,
            }),
        ];

        let concentration = PositionConcentration::from_papers(&papers);

        assert_eq!(concentration.total_positions, 2);
        // Large position is 1000/1100 = 90.91%
        assert!(concentration.top_positions[0].percentage > dec!(90));
        // Small position is 100/1100 = 9.09%
        assert!(concentration.top_positions[1].percentage < dec!(10));
    }

    #[test]
    fn test_risk_level_assessment() {
        // Test low risk scenario
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(250), Currency::RUB),
                percentage: dec!(25),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(250), Currency::RUB),
                percentage: dec!(25),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(250), Currency::RUB),
                percentage: dec!(25),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::from_value(dec!(125), Currency::RUB),
                percentage: dec!(12.5),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::from_value(dec!(125), Currency::RUB),
                percentage: dec!(12.5),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let currency_alloc = CurrencyAllocation {
            allocations: vec![
                CurrencyItem {
                    currency: Currency::RUB,
                    value: Money::from_value(dec!(500), Currency::RUB),
                    percentage: dec!(50),
                },
                CurrencyItem {
                    currency: Currency::USD,
                    value: Money::from_value(dec!(500), Currency::RUB),
                    percentage: dec!(50),
                },
            ],
            total_value: Money::from_value(dec!(1000), Currency::RUB),
            currency_count: 2,
            hhi: dec!(0.5),
        };

        let position_conc = PositionConcentration {
            top_positions: vec![],
            top_5_percentage: dec!(50),
            top_10_percentage: dec!(80),
            hhi: dec!(0.1),
            total_positions: 20,
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let metrics = RiskMetrics::calculate(
            &asset_alloc,
            &currency_alloc,
            &SectorAllocation::from_papers(&[]),
            &position_conc,
        );

        // With diversified portfolio, risk should be relatively low
        assert!(metrics.diversification_score > dec!(50));
    }

    #[test]
    fn test_risk_level_enum_display() {
        assert_eq!(RiskLevel::Low.to_string(), "Low");
        assert_eq!(RiskLevel::Medium.to_string(), "Medium");
        assert_eq!(RiskLevel::High.to_string(), "High");
        assert_eq!(RiskLevel::VeryHigh.to_string(), "Very High");
    }

    #[test]
    fn test_risk_metrics_from_allocation_only() {
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(400), Currency::RUB),
                percentage: dec!(40),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(400), Currency::RUB),
                percentage: dec!(40),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(200), Currency::RUB),
                percentage: dec!(20),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let currency_alloc = CurrencyAllocation {
            allocations: vec![CurrencyItem {
                currency: Currency::RUB,
                value: Money::from_value(dec!(1000), Currency::RUB),
                percentage: dec!(100),
            }],
            total_value: Money::from_value(dec!(1000), Currency::RUB),
            currency_count: 1,
            hhi: dec!(1),
        };

        let position_conc = PositionConcentration {
            top_positions: vec![],
            top_5_percentage: dec!(60),
            top_10_percentage: dec!(90),
            hhi: dec!(0.2),
            total_positions: 10,
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let metrics = RiskMetrics::calculate(
            &asset_alloc,
            &currency_alloc,
            &SectorAllocation::from_papers(&[]),
            &position_conc,
        );
        assert!(metrics.diversification_score > dec!(0));
        assert!(metrics.currency_risk > dec!(0));
        assert!(metrics.concentration_risk > dec!(0));
        assert_eq!(metrics.currency_risk, dec!(100)); // hhi=1 → 100 + single-currency penalty, capped at 100
    }

    /// 60% bonds, 30% shares, 5% ETFs, 5% currencies.
    fn conservative_target() -> TargetAllocation {
        TargetAllocation {
            bonds: dec!(60),
            shares: dec!(30),
            etfs: dec!(5),
            currencies: dec!(5),
            futures: dec!(0),
        }
    }

    #[test]
    fn target_allocation_parses_all_asset_types() {
        // Arrange
        let input = "bonds=40,shares=40,etfs=10,currencies=5,futures=5";

        // Act
        let target = TargetAllocation::from_str(input).unwrap();

        // Assert
        assert_eq!(target.bonds, dec!(40));
        assert_eq!(target.shares, dec!(40));
        assert_eq!(target.etfs, dec!(10));
        assert_eq!(target.currencies, dec!(5));
        assert_eq!(target.futures, dec!(5));
    }

    #[test]
    fn target_allocation_omitted_types_are_zero() {
        // Arrange
        let input = " Bond = 62.5 , SHARE=37.5 ";

        // Act
        let target = TargetAllocation::from_str(input).unwrap();

        // Assert
        assert_eq!(target.bonds, dec!(62.5));
        assert_eq!(target.shares, dec!(37.5));
        assert!(target.etfs.is_zero());
        assert!(target.currencies.is_zero());
        assert!(target.futures.is_zero());
    }

    #[rstest]
    #[case::conservative("conservative", conservative_target())]
    #[case::balanced(" Balanced ", TargetAllocation {
        bonds: dec!(40),
        shares: dec!(40),
        etfs: dec!(10),
        currencies: dec!(5),
        futures: dec!(5),
    })]
    fn target_allocation_parses_preset(#[case] input: &str, #[case] expected: TargetAllocation) {
        // Arrange

        // Act
        let target = TargetAllocation::from_str(input).unwrap();

        // Assert
        assert_eq!(target, expected);
    }

    #[test]
    fn target_presets_sum_up_to_100() {
        // Arrange

        // Act
        let sums = TARGET_PRESETS
            .map(|(name, t)| (name, t.bonds + t.shares + t.etfs + t.currencies + t.futures));

        // Assert
        for (name, sum) in sums {
            assert_eq!(sum, dec!(100), "{name}");
        }
    }

    #[test]
    fn presets_help_lists_presets_with_percents() {
        // Arrange

        // Act
        let help = TargetAllocation::presets_help();

        // Assert
        assert_eq!(
            help,
            "conservative (bonds 60, shares 30, etfs 5, currencies 5, futures 0), \
             balanced (bonds 40, shares 40, etfs 10, currencies 5, futures 5)"
        );
    }

    #[rstest]
    #[case::sum_below_100("bonds=60", "sum up to 60%")]
    #[case::sum_above_100("bonds=60,shares=50", "sum up to 110%")]
    #[case::empty("", "sum up to 0%")]
    #[case::unknown_type("bonds=60,gold=40", "unknown asset type 'gold'")]
    #[case::not_a_number("bonds=sixty", "'sixty' is not a number")]
    #[case::no_equals_sign(
        "bonds60",
        "neither a preset (conservative, balanced) nor asset=percent"
    )]
    #[case::unknown_preset("aggressive", "neither a preset")]
    #[case::negative("bonds=120,shares=-20", "bonds=120% must be within 0..100")]
    #[case::duplicate("bonds=50,bond=50", "bonds is set more than once")]
    fn target_allocation_rejects_invalid_input(#[case] input: &str, #[case] expected: &str) {
        // Arrange

        // Act
        let error = TargetAllocation::from_str(input).unwrap_err();

        // Assert
        assert!(error.contains(expected), "{error}");
    }

    #[test]
    fn test_rebalancing_no_action_needed() {
        // Portfolio matches target exactly
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(600), Currency::RUB),
                percentage: dec!(60),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(300), Currency::RUB),
                percentage: dec!(30),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let target = conservative_target();
        let analysis = RebalancingAnalysis::analyze(&asset_alloc, &target);

        // All actions should be Hold since portfolio matches target
        for rec in &analysis.recommendations {
            assert_eq!(rec.action, RebalanceAction::Hold);
        }
        assert_eq!(analysis.max_deviation, dec!(0));
    }

    #[test]
    fn test_rebalancing_buy_and_sell() {
        // Portfolio is overweight in shares, underweight in bonds
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(400), Currency::RUB),
                percentage: dec!(40),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(500), Currency::RUB),
                percentage: dec!(50),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let target = conservative_target(); // 60% bonds, 30% shares
        let analysis = RebalancingAnalysis::analyze(&asset_alloc, &target);

        // Bonds should be BUY (currently 40%, target 60%)
        let bonds_rec = analysis
            .recommendations
            .iter()
            .find(|r| r.asset_type == "Bonds")
            .unwrap();
        assert_eq!(bonds_rec.action, RebalanceAction::Buy);
        assert!(bonds_rec.deviation < dec!(0)); // Underweight

        // Shares should be SELL (currently 50%, target 30%)
        let shares_rec = analysis
            .recommendations
            .iter()
            .find(|r| r.asset_type == "Shares")
            .unwrap();
        assert_eq!(shares_rec.action, RebalanceAction::Sell);
        assert!(shares_rec.deviation > dec!(0)); // Overweight
    }

    /// Allocation with the given currencies share, the rest in bonds.
    fn allocation_with_currencies(currencies_percentage: Decimal) -> AssetAllocation {
        let item = |name, percentage: Decimal| AllocationItem {
            name,
            value: Money::from_value(percentage * dec!(10), Currency::RUB),
            percentage,
        };
        AssetAllocation {
            bonds: item("Bonds", dec!(100) - currencies_percentage),
            shares: item("Shares", dec!(0)),
            etfs: item("ETFs", dec!(0)),
            currencies: item("Currencies", currencies_percentage),
            futures: item("Futures", dec!(0)),
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        }
    }

    #[rstest]
    #[case::rounds_to_threshold(dec!(0.0038), dec!(-5.00), RebalanceAction::Buy)]
    #[case::rounds_below_threshold(dec!(0.006), dec!(-4.99), RebalanceAction::Hold)]
    #[case::exactly_threshold(dec!(0), dec!(-5), RebalanceAction::Buy)]
    fn rebalancing_threshold_applies_to_displayed_deviation(
        #[case] currencies_percentage: Decimal,
        #[case] expected_deviation: Decimal,
        #[case] expected_action: RebalanceAction,
    ) {
        // Arrange
        let allocation = allocation_with_currencies(currencies_percentage);
        let target = TargetAllocation {
            bonds: dec!(95),
            shares: dec!(0),
            etfs: dec!(0),
            currencies: dec!(5),
            futures: dec!(0),
        };

        // Act
        let analysis = RebalancingAnalysis::analyze(&allocation, &target);

        // Assert
        let currencies = analysis
            .recommendations
            .iter()
            .find(|r| r.asset_type == "Currencies")
            .unwrap();
        assert_eq!(currencies.deviation, expected_deviation);
        assert_eq!(currencies.action, expected_action);
    }

    #[test]
    fn test_rebalancing_threshold() {
        // Small deviation within 5% threshold
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(580), Currency::RUB),
                percentage: dec!(58),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(320), Currency::RUB),
                percentage: dec!(32),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let target = conservative_target(); // 60% bonds, 30% shares
        let analysis = RebalancingAnalysis::analyze(&asset_alloc, &target);

        // Deviations are within 5% threshold, so all should be Hold
        for rec in &analysis.recommendations {
            assert_eq!(rec.action, RebalanceAction::Hold);
        }
    }

    #[test]
    fn test_rebalancing_priority_score() {
        // High deviation should result in high priority score
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(200), Currency::RUB),
                percentage: dec!(20),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(700), Currency::RUB),
                percentage: dec!(70),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            currencies: AllocationItem {
                name: "Currencies",
                value: Money::from_value(dec!(50), Currency::RUB),
                percentage: dec!(5),
            },
            futures: AllocationItem {
                name: "Futures",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            total_value: Money::from_value(dec!(1000), Currency::RUB),
        };

        let target = conservative_target(); // 60% bonds, 30% shares
        let analysis = RebalancingAnalysis::analyze(&asset_alloc, &target);

        // Max deviation is 40% (bonds: 20% vs 60% target)
        assert_eq!(analysis.max_deviation, dec!(40));
        // Priority score should be capped at 100 (40 * 5 = 200, but capped)
        assert_eq!(analysis.priority_score, dec!(100));
    }
}
