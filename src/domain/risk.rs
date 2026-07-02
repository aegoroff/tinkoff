use std::collections::HashMap;

use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
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
    /// Portfolio volatility (annualized, as percentage 0-100)
    pub volatility: Decimal,
    /// Portfolio beta (sensitivity to market, typically 0-2)
    pub beta: Decimal,
    /// Value at Risk (95% confidence, 1-day horizon, as percentage 0-100)
    pub var_95_1d: Decimal,
    /// Value at Risk (95% confidence, 30-day horizon, as percentage 0-100)
    pub var_95_30d: Decimal,
    /// Value at Risk (95% confidence, quarterly horizon ~90 days, as percentage 0-100)
    pub var_95_quarterly: Decimal,
    /// Value at Risk (95% confidence, yearly horizon ~252 days, as percentage 0-100)
    pub var_95_yearly: Decimal,
    /// Risk level assessment
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

/// Target allocation for portfolio rebalancing
#[derive(Debug, Clone)]
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

impl Default for TargetAllocation {
    fn default() -> Self {
        // Conservative allocation: 60% bonds, 30% shares, 10% other
        Self {
            bonds: dec!(60),
            shares: dec!(30),
            etfs: dec!(5),
            currencies: dec!(5),
            futures: dec!(0),
        }
    }
}

impl TargetAllocation {
    /// Create a balanced allocation (40% bonds, 40% shares, 20% other)
    #[must_use]
    pub fn balanced() -> Self {
        Self {
            bonds: dec!(40),
            shares: dec!(40),
            etfs: dec!(10),
            currencies: dec!(5),
            futures: dec!(5),
        }
    }

    /// Create an aggressive allocation (20% bonds, 60% shares, 20% other)
    #[must_use]
    pub fn aggressive() -> Self {
        Self {
            bonds: dec!(20),
            shares: dec!(60),
            etfs: dec!(10),
            currencies: dec!(5),
            futures: dec!(5),
        }
    }

    /// Create a conservative allocation (70% bonds, 20% shares, 10% other)
    #[must_use]
    pub fn conservative() -> Self {
        Self {
            bonds: dec!(70),
            shares: dec!(20),
            etfs: dec!(5),
            currencies: dec!(5),
            futures: dec!(0),
        }
    }

    /// Validate that percentages sum to 100
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let sum = self.bonds + self.shares + self.etfs + self.currencies + self.futures;
        sum >= dec!(99) && sum <= dec!(101)
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
            let deviation = current_pct - target_pct;
            let abs_deviation = deviation.abs();

            if abs_deviation > max_deviation {
                max_deviation = abs_deviation;
            }

            // Calculate the value to rebalance
            let target_value = (target_pct / dec!(100)) * total_value.value;
            let current_value = (current_pct / dec!(100)) * total_value.value;
            let rebalance_amount = (target_value - current_value).abs();

            // Determine action based on deviation
            // Threshold of 5% deviation before recommending action
            let (action, rebalance_value) = if abs_deviation < dec!(5) {
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
    pub fn analyze(portfolio: &Portfolio, all_papers: &[LoadedPaper]) -> Self {
        let asset_allocation = AssetAllocation::from_portfolio(portfolio);
        let currency_allocation = CurrencyAllocation::from_papers(all_papers);
        let position_concentration = PositionConcentration::from_papers(all_papers);
        let risk_metrics = RiskMetrics::calculate(
            &asset_allocation,
            &currency_allocation,
            &position_concentration,
        );

        Self {
            asset_allocation,
            currency_allocation,
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
            let (value, currency) = match paper {
                LoadedPaper::Bond(p) => (p.current().value, p.currency()),
                LoadedPaper::Share(p) => (p.current().value, p.currency()),
                LoadedPaper::Etf(p) => (p.current().value, p.currency()),
                LoadedPaper::Currency(p) | LoadedPaper::Future(p) => {
                    (p.current().value, p.currency())
                }
            };
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
                    value: Money::from_value(value, currency),
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

impl PositionConcentration {
    #[must_use]
    fn from_papers(papers: &[LoadedPaper]) -> Self {
        let mut position_values: Vec<(String, &Ticker, &'static str, Decimal, Currency)> =
            Vec::new();

        for paper in papers {
            let (name, ticker, instrument_type, value, currency) = match paper {
                LoadedPaper::Bond(p) => (
                    p.name.clone(),
                    &p.ticker,
                    "Bond",
                    p.current().value,
                    p.currency(),
                ),
                LoadedPaper::Share(p) => (
                    p.name.clone(),
                    &p.ticker,
                    "Share",
                    p.current().value,
                    p.currency(),
                ),
                LoadedPaper::Etf(p) => (
                    p.name.clone(),
                    &p.ticker,
                    "ETF",
                    p.current().value,
                    p.currency(),
                ),
                LoadedPaper::Currency(p) | LoadedPaper::Future(p) => (
                    p.name.clone(),
                    &p.ticker,
                    match paper {
                        LoadedPaper::Etf(_) => "ETF",
                        LoadedPaper::Currency(_) => "Currency",
                        LoadedPaper::Future(_) => "Future",
                        _ => unreachable!(),
                    },
                    p.current().value,
                    p.currency(),
                ),
            };
            position_values.push((name, ticker, instrument_type, value, currency));
        }

        let total_value: Decimal = position_values.iter().map(|(_, _, _, v, _)| v).sum();
        let total_positions = position_values.len();

        // Sort by value descending
        position_values.sort_by_key(|b| std::cmp::Reverse(b.3));

        // Calculate percentages and create PositionItem list
        let mut items: Vec<PositionItem> = position_values
            .iter()
            .map(|(name, ticker, instrument_type, value, currency)| {
                let percentage = if total_value.is_zero() {
                    dec!(0)
                } else {
                    (*value / total_value) * dec!(100)
                };
                PositionItem {
                    name: name.clone(),
                    ticker: (*ticker).clone(),
                    instrument_type,
                    value: Money::from_value(*value, *currency),
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
        position_conc: &PositionConcentration,
    ) -> Self {
        let diversification_score =
            Self::calculate_diversification_score(asset_alloc, currency_alloc, position_conc);
        let currency_risk = Self::calculate_currency_risk(currency_alloc);
        let concentration_risk = Self::calculate_concentration_risk(position_conc);
        let asset_concentration_risk = Self::calculate_asset_concentration_risk(asset_alloc);
        let volatility = Self::calculate_volatility(asset_alloc);
        let beta = Self::calculate_beta(asset_alloc);
        // Calculate VaR for different horizons: 1 day, 30 days, quarterly (~90 days), yearly (~252 days)
        let var_95_1d = Self::calculate_var_95(volatility, 1);
        let var_95_30d = Self::calculate_var_95(volatility, 30);
        let var_95_quarterly = Self::calculate_var_95(volatility, 90);
        let var_95_yearly = Self::calculate_var_95(volatility, 252);

        let risk_level = Self::assess_risk_level(
            diversification_score,
            currency_risk,
            concentration_risk,
            asset_concentration_risk,
            volatility,
            beta,
        );

        Self {
            diversification_score,
            currency_risk,
            concentration_risk,
            asset_concentration_risk,
            volatility,
            beta,
            var_95_1d,
            var_95_30d,
            var_95_quarterly,
            var_95_yearly,
            risk_level,
        }
    }

    fn calculate_diversification_score(
        asset_alloc: &AssetAllocation,
        currency_alloc: &CurrencyAllocation,
        position_conc: &PositionConcentration,
    ) -> Decimal {
        // Weight factors for diversification calculation
        let asset_diversification =
            dec!(100) - Self::calculate_asset_concentration_risk(asset_alloc);
        let currency_diversification = dec!(100) - currency_alloc.hhi * dec!(100);
        let position_diversification = dec!(100) - position_conc.hhi * dec!(100);

        // Weighted average (positions matter most, then assets, then currency)
        (asset_diversification * dec!(3)
            + currency_diversification * dec!(2)
            + position_diversification * dec!(5))
            / dec!(10)
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
        volatility: Decimal,
        beta: Decimal,
    ) -> RiskLevel {
        let avg_risk = (currency_risk + concentration_risk + asset_concentration_risk) / dec!(3);
        let diversification_bonus = diversification_score / dec!(10);

        // Volatility penalty: annualised σ > 20% adds up to 20 points.
        // Typical equity market vol is ~15-20%; anything above signals elevated risk.
        let vol_penalty = (volatility - dec!(20)).max(dec!(0)).min(dec!(20));

        // Beta penalty: β > 1.5 adds up to 15 points.
        // β=1 is market-neutral; above 1.5 implies meaningful leverage to market swings.
        let beta_penalty = ((beta - dec!(1.5)) * dec!(15)).max(dec!(0)).min(dec!(15));

        let final_risk = (avg_risk - diversification_bonus + vol_penalty + beta_penalty)
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

    /// Calculate portfolio volatility based on asset allocation.
    ///
    /// Uses typical annualized volatility values for each asset type:
    /// - Bonds: 5%
    /// - Shares: 20%
    /// - ETFs: 15% (average, depends on underlying)
    /// - Currencies: 10%
    /// - Futures: 25% (leveraged instruments)
    ///
    /// Formula: `σ_portfolio` = sqrt(Σ wᵢ² · σᵢ²)
    ///
    /// This assumes zero cross-class correlation, which is conservative
    /// (i.e. gives a lower bound vs. the fully correlated linear sum, but
    /// higher than a perfectly negatively correlated portfolio). Linear
    /// weighting would be correct only if all correlations were 1, which
    /// systematically overstates risk.
    #[must_use]
    fn calculate_volatility(asset_alloc: &AssetAllocation) -> Decimal {
        // Typical annualized volatility percentages for each asset type
        let bond_vol = dec!(5);
        let share_vol = dec!(20);
        let etf_vol = dec!(15);
        let currency_vol = dec!(10);
        let future_vol = dec!(25);

        // Weights are percentages (0–100), so wᵢ = percentage / 100.
        // wᵢ² · σᵢ² = (percentage / 100)² · σᵢ²
        //            = percentage² · σᵢ² / 10_000
        let sq = |x: Decimal| x * x;
        let weighted_variance = sq(asset_alloc.bonds.percentage) * sq(bond_vol)
            + sq(asset_alloc.shares.percentage) * sq(share_vol)
            + sq(asset_alloc.etfs.percentage) * sq(etf_vol)
            + sq(asset_alloc.currencies.percentage) * sq(currency_vol)
            + sq(asset_alloc.futures.percentage) * sq(future_vol);

        // Divide by 10_000 to undo the two percentage→fraction conversions,
        // then take the square root to get σ_portfolio in percent.
        let variance_f64 = weighted_variance.to_f64().unwrap_or(0.0) / 10_000.0;

        Decimal::try_from(variance_f64.sqrt()).unwrap_or(dec!(0))
    }

    /// Calculate portfolio beta (sensitivity to market movements)
    /// Uses typical beta values for each asset type:
    /// - Bonds: 0.1 (low correlation with equity market)
    /// - Shares: 1.0 (market beta)
    /// - ETFs: 0.9 (slightly lower due to diversification)
    /// - Currencies: 0.0 (no market beta)
    /// - Futures: 1.2 (slightly leveraged)
    #[must_use]
    fn calculate_beta(asset_alloc: &AssetAllocation) -> Decimal {
        // Typical beta values for each asset type
        let bond_beta = dec!(0.1);
        let share_beta = dec!(1);
        let etf_beta = dec!(0.9);
        let currency_beta = dec!(0);
        let future_beta = dec!(1.2);

        // Weight beta by asset allocation percentage
        let weighted_beta = asset_alloc.bonds.percentage * bond_beta
            + asset_alloc.shares.percentage * share_beta
            + asset_alloc.etfs.percentage * etf_beta
            + asset_alloc.currencies.percentage * currency_beta
            + asset_alloc.futures.percentage * future_beta;

        // Divide by 100 to get weighted average
        weighted_beta / dec!(100)
    }

    /// Calculate Value at Risk (`VaR`) at 95% confidence level for a given horizon.
    ///
    /// Uses parametric `VaR`: `VaR(T) = 1.645 · σ_annual · sqrt(T / 252)`
    ///
    /// Assuming zero expected return (μ = 0), which is conservative for short horizons.
    /// The square-root-of-time rule scales annualised volatility to the desired horizon.
    ///
    /// `horizon_days` — trading days (252 per year). Pass `1` for the standard 1-day `VaR`.
    /// Result is expressed as a percentage of portfolio value (0–100).
    #[must_use]
    fn calculate_var_95(volatility: Decimal, horizon_days: u32) -> Decimal {
        // Z-score for 95% one-tailed confidence level
        let z_score = dec!(1.645);

        // Scale annual volatility to the requested horizon via sqrt-of-time rule.
        // sqrt(horizon / 252) computed in f64 to avoid needing the `maths` feature.
        let horizon_scale =
            Decimal::try_from((f64::from(horizon_days) / 252.0_f64).sqrt()).unwrap_or(dec!(1));

        (z_score * volatility * horizon_scale).min(dec!(100))
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::{
        CouponProfit, DividendProfit, Figi, LoadedPaper, Paper, Position, Ticker, Totals,
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
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
            },
            profit: CouponProfit,
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
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
            },
            profit: DividendProfit,
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
                quantity: dec!(10),
            },
            totals: Totals {
                additional_profit: Money::zero(Currency::RUB),
                fees: Money::zero(Currency::RUB),
            },
            profit: DividendProfit,
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
                    quantity: dec!(10),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                },
                profit: DividendProfit,
            }),
            LoadedPaper::Share(Paper {
                name: "Share 2".to_string(),
                ticker: Ticker::new("SHARE2".to_string()),
                figi: Figi::new("share2figi".to_string()),
                position: Position {
                    currency: Currency::USD,
                    average_buy_price: Money::from_value(dec!(50), Currency::USD),
                    current_instrument_price: Money::from_value(dec!(50), Currency::USD),
                    quantity: dec!(10),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::USD),
                    fees: Money::zero(Currency::USD),
                },
                profit: DividendProfit,
            }),
        ];

        let allocation = CurrencyAllocation::from_papers(&papers);

        assert_eq!(allocation.currency_count, 2);
        // HHI = 0.5^2 + 0.5^2 = 0.5
        assert_eq!(allocation.hhi, dec!(0.5));
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
                    quantity: dec!(10),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                },
                profit: DividendProfit,
            }),
            LoadedPaper::Share(Paper {
                name: "Small Position".to_string(),
                ticker: Ticker::new("SMALL".to_string()),
                figi: Figi::new("smallfigi".to_string()),
                position: Position {
                    currency: Currency::RUB,
                    average_buy_price: Money::from_value(dec!(10), Currency::RUB),
                    current_instrument_price: Money::from_value(dec!(10), Currency::RUB),
                    quantity: dec!(10),
                },
                totals: Totals {
                    additional_profit: Money::zero(Currency::RUB),
                    fees: Money::zero(Currency::RUB),
                },
                profit: DividendProfit,
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

        let metrics = RiskMetrics::calculate(&asset_alloc, &currency_alloc, &position_conc);

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
    fn test_volatility_calculation() {
        // Test 100% bonds - low volatility
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(1000), Currency::RUB),
                percentage: dec!(100),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
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

        let volatility = RiskMetrics::calculate_volatility(&asset_alloc);
        // sqrt((1.0 * 5)²) = 5% — single-asset case is unchanged vs. linear formula
        assert_eq!(volatility, dec!(5));
    }

    #[test]
    fn test_volatility_mixed_portfolio() {
        // Test 50% shares, 50% bonds
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(500), Currency::RUB),
                percentage: dec!(50),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(500), Currency::RUB),
                percentage: dec!(50),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
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

        let volatility = RiskMetrics::calculate_volatility(&asset_alloc);
        // sqrt((0.5*5)² + (0.5*20)²) = sqrt(6.25 + 100) = sqrt(106.25) ≈ 10.3078%
        // (linear sum 12.5% was wrong: assumes correlation = 1 between all asset classes)
        let expected = dec!(10.3077640640);
        let epsilon = dec!(0.000001);
        assert!(
            (volatility - expected).abs() < epsilon,
            "volatility {volatility} not close enough to {expected}"
        );
    }

    #[test]
    fn test_beta_calculation() {
        // Test 100% bonds - low beta
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(1000), Currency::RUB),
                percentage: dec!(100),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
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

        let beta = RiskMetrics::calculate_beta(&asset_alloc);
        // 100% bonds = 0.1 beta
        assert_eq!(beta, dec!(0.1));
    }

    #[test]
    fn test_beta_mixed_portfolio() {
        // Test 50% shares, 50% bonds
        let asset_alloc = AssetAllocation {
            bonds: AllocationItem {
                name: "Bonds",
                value: Money::from_value(dec!(500), Currency::RUB),
                percentage: dec!(50),
            },
            shares: AllocationItem {
                name: "Shares",
                value: Money::from_value(dec!(500), Currency::RUB),
                percentage: dec!(50),
            },
            etfs: AllocationItem {
                name: "ETFs",
                value: Money::zero(Currency::RUB),
                percentage: dec!(0),
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

        let beta = RiskMetrics::calculate_beta(&asset_alloc);
        // 50% bonds (0.1) + 50% shares (1.0) = 0.05 + 0.5 = 0.55
        assert_eq!(beta, dec!(0.55));
    }

    #[test]
    fn test_var_95_calculation() {
        // 1-day VaR from 10% annual volatility
        // VaR(1d) = 1.645 * 10 * sqrt(1/252) ≈ 1.0363%
        let volatility = dec!(10);
        let var = RiskMetrics::calculate_var_95(volatility, 1);
        let expected = Decimal::try_from(1.645 * 10.0 * (1.0_f64 / 252.0).sqrt()).unwrap();
        let epsilon = dec!(0.000001);
        assert!(
            (var - expected).abs() < epsilon,
            "1d VaR {var} not close enough to {expected}"
        );
    }

    #[test]
    fn test_var_95_10day() {
        // 10-day VaR from 10% annual volatility (Basel standard horizon)
        // VaR(10d) = 1.645 * 10 * sqrt(10/252) ≈ 3.2756%
        let volatility = dec!(10);
        let var = RiskMetrics::calculate_var_95(volatility, 10);
        let expected = Decimal::try_from(1.645 * 10.0 * (10.0_f64 / 252.0).sqrt()).unwrap();
        let epsilon = dec!(0.000001);
        assert!(
            (var - expected).abs() < epsilon,
            "10d VaR {var} not close enough to {expected}"
        );
    }

    #[test]
    fn test_var_95_high_volatility() {
        // Even very high vol should be capped at 100%
        let volatility = dec!(100);
        let var = RiskMetrics::calculate_var_95(volatility, 252);
        // VaR(252d) = 1.645 * 100 * sqrt(1) = 164.5 → capped at 100
        assert_eq!(var, dec!(100));
    }

    #[test]
    fn test_full_risk_metrics_calculation() {
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

        let metrics = RiskMetrics::calculate(&asset_alloc, &currency_alloc, &position_conc);

        let epsilon = dec!(0.000001);

        // Verify volatility: sqrt((0.4*5)² + (0.4*20)² + (0.2*15)²)
        //   = sqrt(4 + 64 + 9) = sqrt(77) ≈ 8.7750%
        // (old linear result 13% assumed correlation = 1, which overstated risk)
        let expected_vol = dec!(8.7749643874);
        assert!(
            (metrics.volatility - expected_vol).abs() < epsilon,
            "volatility {} not close enough to {expected_vol}",
            metrics.volatility
        );

        // Verify beta: 40%*0.1 + 40%*1 + 20%*0.9 = 0.04 + 0.4 + 0.18 = 0.62
        // (beta is a linear quantity — weighted average is correct here)
        assert_eq!(metrics.beta, dec!(0.62));

        // Verify VaR 1d: 1.645 * sqrt(77) * sqrt(1/252) ≈ 0.9093%  (1-day, 95% confidence)
        let expected_var_1d =
            Decimal::try_from(1.645 * 8.7749643874_f64 * (1.0_f64 / 252.0).sqrt()).unwrap();
        assert!(
            (metrics.var_95_1d - expected_var_1d).abs() < epsilon,
            "VaR 1d {} not close enough to {expected_var_1d}",
            metrics.var_95_1d
        );

        // Verify VaR 30d: 1.645 * sqrt(77) * sqrt(30/252) ≈ 4.980%
        let expected_var_30d =
            Decimal::try_from(1.645 * 8.7749643874_f64 * (30.0_f64 / 252.0).sqrt()).unwrap();
        assert!(
            (metrics.var_95_30d - expected_var_30d).abs() < epsilon,
            "VaR 30d {} not close enough to {expected_var_30d}",
            metrics.var_95_30d
        );

        // Verify VaR quarterly (90d): 1.645 * sqrt(77) * sqrt(90/252) ≈ 8.627%
        let expected_var_quarterly =
            Decimal::try_from(1.645 * 8.7749643874_f64 * (90.0_f64 / 252.0).sqrt()).unwrap();
        assert!(
            (metrics.var_95_quarterly - expected_var_quarterly).abs() < epsilon,
            "VaR quarterly {} not close enough to {expected_var_quarterly}",
            metrics.var_95_quarterly
        );

        // Verify VaR yearly (252d): 1.645 * sqrt(77) * sqrt(252/252) ≈ 14.435%
        let expected_var_yearly =
            Decimal::try_from(1.645 * 8.7749643874_f64 * (252.0_f64 / 252.0).sqrt()).unwrap();
        assert!(
            (metrics.var_95_yearly - expected_var_yearly).abs() < epsilon,
            "VaR yearly {} not close enough to {expected_var_yearly}",
            metrics.var_95_yearly
        );
    }

    #[test]
    fn test_target_allocation_default() {
        let target = TargetAllocation::default();
        assert_eq!(target.bonds, dec!(60));
        assert_eq!(target.shares, dec!(30));
        assert_eq!(target.etfs, dec!(5));
        assert_eq!(target.currencies, dec!(5));
        assert_eq!(target.futures, dec!(0));
        assert!(target.is_valid());
    }

    #[test]
    fn test_target_allocation_balanced() {
        let target = TargetAllocation::balanced();
        assert_eq!(target.bonds, dec!(40));
        assert_eq!(target.shares, dec!(40));
        assert_eq!(target.etfs, dec!(10));
        assert_eq!(target.currencies, dec!(5));
        assert_eq!(target.futures, dec!(5));
        assert!(target.is_valid());
    }

    #[test]
    fn test_target_allocation_aggressive() {
        let target = TargetAllocation::aggressive();
        assert_eq!(target.bonds, dec!(20));
        assert_eq!(target.shares, dec!(60));
        assert_eq!(target.etfs, dec!(10));
        assert_eq!(target.currencies, dec!(5));
        assert_eq!(target.futures, dec!(5));
        assert!(target.is_valid());
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

        let target = TargetAllocation::default();
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

        let target = TargetAllocation::default(); // 60% bonds, 30% shares
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

        let target = TargetAllocation::default(); // 60% bonds, 30% shares
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

        let target = TargetAllocation::default(); // 60% bonds, 30% shares
        let analysis = RebalancingAnalysis::analyze(&asset_alloc, &target);

        // Max deviation is 40% (bonds: 20% vs 60% target)
        assert_eq!(analysis.max_deviation, dec!(40));
        // Priority score should be capped at 100 (40 * 5 = 200, but capped)
        assert_eq!(analysis.priority_score, dec!(100));
    }
}
