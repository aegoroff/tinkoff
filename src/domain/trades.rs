//! Orders that carry out rebalancing recommendations in whole lots.
//!
//! Buys are split between the instruments of an asset type by their current values.
//! Sells take the lots whose sale adds the least tax first: the broker sells the earliest
//! tax lots of an instrument first, so its next lot is taxed by the next tax lots in order.

use std::cmp::Reverse;

use iso_currency::Currency;
use rust_decimal::Decimal;

use super::lots::{LotSale, YearIncome};
use super::money::Money;
use super::paper::InstrumentKind;
use super::risk::{RebalanceAction, RebalancingAnalysis};

/// Position that can be bought or sold to rebalance the portfolio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tradable {
    pub kind: InstrumentKind,
    pub ticker: String,
    pub name: String,
    /// Units traded in one lot
    pub lot: Decimal,
    /// Price of a unit with accrued interest, in RUB
    pub price: Money,
    pub quantity: Decimal,
    /// Lots reserved by active orders
    pub blocked_lots: Decimal,
    /// Open tax lots priced now, the earliest first; `None` when their cost is unknown
    pub tax_lots: Option<Vec<LotSale>>,
}

impl Tradable {
    fn lot_price(&self) -> Money {
        self.price * self.lot
    }

    fn value(&self) -> Money {
        self.price * self.quantity
    }

    /// Whole lots not reserved by active orders.
    fn free_lots(&self) -> Decimal {
        ((self.quantity / self.lot).floor() - self.blocked_lots).max(Decimal::ZERO)
    }

    /// Parts of tax lots the first `lots` lots sold take, the earliest first.
    fn sold_tax_lots(&self, lots: Decimal) -> Option<Vec<LotSale>> {
        let mut left = lots * self.lot;
        let mut sold = Vec::new();
        for lot in self.tax_lots.as_ref()? {
            if left <= Decimal::ZERO {
                break;
            }
            let quantity = left.min(lot.quantity);
            sold.push(lot.part(quantity));
            left -= quantity;
        }
        Some(sold)
    }

    /// Taxable result per ruble of the next lot sold after `sold` lots;
    /// `None` when the cost is unknown. Exempt tax lots add no taxable result.
    fn tax_rate(&self, sold: Decimal) -> Option<Decimal> {
        let taxable = |lots: Vec<LotSale>| -> Decimal {
            lots.iter()
                .filter(|l| !l.ldv)
                .map(|l| l.result().value)
                .sum()
        };
        let before = taxable(self.sold_tax_lots(sold)?);
        let after = taxable(self.sold_tax_lots(sold + Decimal::ONE)?);
        let lot_price = self.lot_price().value;
        (!lot_price.is_zero()).then(|| (after - before) / lot_price)
    }
}

/// Whether sales are taxed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaleTaxation {
    /// Taxed after this year's investment income
    Taxed(YearIncome),
    /// Not taxed, as on an individual investment account
    Untaxed,
    /// Unknown, as for several accounts whose lots are matched separately
    Unknown,
}

/// Order of whole lots of an instrument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub ticker: String,
    pub name: String,
    pub lots: Decimal,
    pub units: Decimal,
    pub amount: Money,
}

/// Orders carrying out the recommendation for an asset type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetOrders {
    pub asset_type: &'static str,
    pub action: RebalanceAction,
    /// Amount to buy or sell
    pub target: Money,
    pub orders: Vec<Order>,
    /// Part of the target the orders do not carry out: less than a lot, or all of it
    /// for an asset type without instruments or not traded by lots
    pub left: Money,
}

impl AssetOrders {
    /// Whether the asset type is rebalanced by amount only: currencies and futures,
    /// or a type without instruments to buy.
    #[must_use]
    pub fn by_amount(&self) -> bool {
        self.orders.is_empty() && self.left == self.target
    }
}

/// Orders carrying out rebalancing recommendations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradePlan {
    pub assets: Vec<AssetOrders>,
    /// Tax due on the sells; `None` when unknown
    pub tax: Option<Money>,
}

impl TradePlan {
    /// Turns buy and sell recommendations of `analysis` into orders of `tradables`.
    #[must_use]
    pub fn new(
        analysis: &RebalancingAnalysis,
        tradables: &[Tradable],
        taxation: &SaleTaxation,
    ) -> Self {
        let mut sold_tax_lots = Some(Vec::new());
        let assets = analysis
            .recommendations
            .iter()
            .filter(|r| r.action != RebalanceAction::Hold)
            .map(|r| {
                let of_kind: Vec<&Tradable> = if traded_by_lots(r.kind) {
                    tradables
                        .iter()
                        .filter(|t| t.kind == r.kind && t.quantity > Decimal::ZERO)
                        .collect()
                } else {
                    Vec::new()
                };
                let (counts, left) = match r.action {
                    RebalanceAction::Sell => sell_lots(&of_kind, r.rebalance_value, taxation),
                    _ => buy_lots(&of_kind, r.rebalance_value),
                };
                if r.action == RebalanceAction::Sell {
                    for (t, lots) in of_kind.iter().zip(&counts) {
                        if lots.is_zero() {
                            continue;
                        }
                        match (&mut sold_tax_lots, t.sold_tax_lots(*lots)) {
                            (Some(all), Some(sold)) => all.extend(sold),
                            _ => sold_tax_lots = None,
                        }
                    }
                }
                AssetOrders {
                    asset_type: r.asset_type,
                    action: r.action,
                    target: r.rebalance_value,
                    orders: orders(&of_kind, &counts),
                    left,
                }
            })
            .collect();
        let tax = match taxation {
            SaleTaxation::Taxed(year) => sold_tax_lots.map(|lots| year.sale_tax(&lots)),
            SaleTaxation::Untaxed => Some(rub(Decimal::ZERO)),
            SaleTaxation::Unknown => None,
        };
        Self { assets, tax }
    }
}

/// Kinds of instruments traded by lots; currencies and futures are rebalanced by amount.
fn traded_by_lots(kind: InstrumentKind) -> bool {
    matches!(
        kind,
        InstrumentKind::Share | InstrumentKind::Bond | InstrumentKind::Etf
    )
}

fn rub(value: Decimal) -> Money {
    Money::from_value(value, Currency::RUB)
}

/// Lots of `tradables` to buy for `amount`, split by their current values and rounded
/// down; the money left buys lots of the ones closest to their next lot while it is
/// enough. Returns lots of every tradable and the money left.
fn buy_lots(tradables: &[&Tradable], amount: Money) -> (Vec<Decimal>, Money) {
    let total: Decimal = tradables.iter().map(|t| t.value().value).sum();
    let mut counts = vec![Decimal::ZERO; tradables.len()];
    if total <= Decimal::ZERO {
        return (counts, amount);
    }
    let mut left = amount.value;
    let mut fractions = Vec::with_capacity(tradables.len());
    for (i, t) in tradables.iter().enumerate() {
        let lot_price = t.lot_price().value;
        if lot_price <= Decimal::ZERO {
            continue;
        }
        let ideal = amount.value * t.value().value / total / lot_price;
        counts[i] = ideal.floor();
        left -= counts[i] * lot_price;
        fractions.push((i, ideal - counts[i]));
    }
    fractions.sort_by_key(|(_, fraction)| Reverse(*fraction));
    for (i, _) in fractions {
        let lot_price = tradables[i].lot_price().value;
        if lot_price <= left {
            counts[i] += Decimal::ONE;
            left -= lot_price;
        }
    }
    (counts, rub(left))
}

/// Lots of `tradables` to sell for no more than `amount`, one at a time: the lot that adds
/// the least taxable result per ruble first, lots of unknown cost last, the biggest
/// position first among equal ones. Returns lots of every tradable and the money left.
fn sell_lots(
    tradables: &[&Tradable],
    amount: Money,
    taxation: &SaleTaxation,
) -> (Vec<Decimal>, Money) {
    let taxed = matches!(taxation, SaleTaxation::Taxed(_));
    let mut counts = vec![Decimal::ZERO; tradables.len()];
    let mut left = amount.value;
    loop {
        let next = tradables
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                let lot_price = t.lot_price().value;
                counts[*i] < t.free_lots() && lot_price > Decimal::ZERO && lot_price <= left
            })
            .min_by_key(|(i, t)| {
                let rate = if taxed {
                    t.tax_rate(counts[*i])
                } else {
                    Some(Decimal::ZERO)
                };
                let remaining = t.value().value - counts[*i] * t.lot_price().value;
                (rate.is_none(), rate.unwrap_or_default(), Reverse(remaining))
            });
        let Some((i, t)) = next else {
            break;
        };
        counts[i] += Decimal::ONE;
        left -= t.lot_price().value;
    }
    (counts, rub(left))
}

/// Orders of the tradables with lots to trade, the biggest amount first.
fn orders(tradables: &[&Tradable], counts: &[Decimal]) -> Vec<Order> {
    let mut orders: Vec<Order> = tradables
        .iter()
        .zip(counts)
        .filter(|(_, lots)| !lots.is_zero())
        .map(|(t, lots)| Order {
            ticker: t.ticker.clone(),
            name: t.name.clone(),
            lots: *lots,
            units: *lots * t.lot,
            amount: t.lot_price() * *lots,
        })
        .collect();
    orders.sort_by_key(|o| Reverse(o.amount.value));
    orders
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::lots::{Lot, SecuritySales};
    use crate::domain::risk::RebalanceRecommendation;

    fn tradable(
        kind: InstrumentKind,
        ticker: &str,
        lot: i64,
        price: i64,
        quantity: i64,
    ) -> Tradable {
        Tradable {
            kind,
            ticker: ticker.to_string(),
            name: ticker.to_string(),
            lot: Decimal::from(lot),
            price: rub(Decimal::from(price)),
            quantity: Decimal::from(quantity),
            blocked_lots: Decimal::ZERO,
            tax_lots: None,
        }
    }

    /// Tax lot of `quantity` units bought on `bought` at `cost` per unit, priced at `price`.
    fn tax_lot(bought: &str, quantity: i64, cost: i64, price: i64) -> LotSale {
        let lot = Lot {
            bought: format!("{bought}T09:00:00Z").parse().unwrap(),
            quantity: Decimal::from(quantity),
            cost: rub(Decimal::from(cost * quantity)),
        };
        LotSale::new(
            &lot,
            rub(Decimal::from(price)),
            "2026-10-27".parse().unwrap(),
        )
    }

    fn analysis(recommendations: &[(InstrumentKind, RebalanceAction, i64)]) -> RebalancingAnalysis {
        RebalancingAnalysis {
            total_value: rub(dec!(100_000)),
            recommendations: recommendations
                .iter()
                .map(|(kind, action, value)| RebalanceRecommendation {
                    asset_type: "Assets",
                    kind: *kind,
                    current_percentage: Decimal::ZERO,
                    target_percentage: Decimal::ZERO,
                    deviation: Decimal::ZERO,
                    action: *action,
                    rebalance_value: rub(Decimal::from(*value)),
                })
                .collect(),
            max_deviation: Decimal::ZERO,
            total_rebalance_value: rub(Decimal::ZERO),
            priority_score: Decimal::ZERO,
        }
    }

    fn lots_of(plan: &TradePlan) -> Vec<(String, Decimal)> {
        plan.assets
            .iter()
            .flat_map(|a| &a.orders)
            .map(|o| (o.ticker.clone(), o.lots))
            .collect()
    }

    #[test]
    fn buy_splits_amount_by_values_in_whole_lots() {
        // Arrange: SBER is worth 3 times GAZP, so it gets 7 500 of 10 000; OFZ is a bond
        let tradables = [
            tradable(InstrumentKind::Share, "SBER", 10, 300, 100),
            tradable(InstrumentKind::Bond, "OFZ", 1, 1000, 10),
            tradable(InstrumentKind::Share, "GAZP", 10, 100, 100),
        ];
        let analysis = analysis(&[(InstrumentKind::Share, RebalanceAction::Buy, 10_000)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Untaxed);

        // Assert: ideal SBER 2.5 lots and GAZP 2.5 lots; 2 000 left is not enough
        // for a SBER lot of 3 000 but buys one more GAZP lot
        assert_eq!(
            lots_of(&plan),
            [("SBER".to_string(), dec!(2)), ("GAZP".to_string(), dec!(3))]
        );
        assert_eq!(plan.assets[0].left.value, dec!(1000));
        assert_eq!(plan.assets[0].orders[0].units, dec!(20));
        assert_eq!(plan.assets[0].orders[0].amount.value, dec!(6000));
    }

    #[test]
    fn buy_of_type_without_instruments_is_by_amount() {
        // Arrange
        let tradables = [tradable(InstrumentKind::Share, "SBER", 10, 300, 100)];
        let analysis = analysis(&[(InstrumentKind::Etf, RebalanceAction::Buy, 5000)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Untaxed);

        // Assert
        assert!(plan.assets[0].by_amount());
        assert_eq!(plan.assets[0].left.value, dec!(5000));
    }

    #[test]
    fn currencies_are_rebalanced_by_amount() {
        // Arrange
        let tradables = [tradable(InstrumentKind::Currency, "USD", 1, 90, 1000)];
        let analysis = analysis(&[(InstrumentKind::Currency, RebalanceAction::Sell, 9000)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Untaxed);

        // Assert
        assert!(plan.assets[0].by_amount());
    }

    #[test]
    fn untaxed_sell_takes_biggest_positions_first_within_free_lots() {
        // Arrange: 1 of 4 SBER lots is reserved by an order
        let sber = Tradable {
            blocked_lots: dec!(1),
            ..tradable(InstrumentKind::Share, "SBER", 10, 300, 40)
        };
        let tradables = [sber, tradable(InstrumentKind::Share, "GAZP", 10, 100, 50)];
        let analysis = analysis(&[(InstrumentKind::Share, RebalanceAction::Sell, 11_000)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Untaxed);

        // Assert
        assert_eq!(
            lots_of(&plan),
            [("SBER".to_string(), dec!(3)), ("GAZP".to_string(), dec!(2))]
        );
        assert_eq!(plan.assets[0].left.value, dec!(0));
        assert_eq!(plan.tax, Some(rub(dec!(0))));
    }

    #[test]
    fn taxed_sell_takes_losses_and_exempt_lots_before_gains() {
        // Arrange: GAIN lots gain 50 per unit, LOSS lots lose 20, LDV lots are exempt
        let with_lots = |ticker: &str, lots: Vec<LotSale>| Tradable {
            tax_lots: Some(lots),
            ..tradable(InstrumentKind::Share, ticker, 1, 100, 2)
        };
        let tradables = [
            with_lots(
                "GAIN",
                vec![
                    tax_lot("2025-01-10", 1, 50, 100),
                    tax_lot("2025-02-10", 1, 50, 100),
                ],
            ),
            with_lots(
                "LDV",
                vec![
                    tax_lot("2020-01-10", 1, 10, 100),
                    tax_lot("2025-01-10", 1, 10, 100),
                ],
            ),
            with_lots(
                "LOSS",
                vec![
                    tax_lot("2025-01-10", 1, 120, 100),
                    tax_lot("2025-02-10", 1, 120, 100),
                ],
            ),
        ];
        let analysis = analysis(&[(InstrumentKind::Share, RebalanceAction::Sell, 450)]);
        let year = YearIncome::zero();

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Taxed(year));

        // Assert: both LOSS lots, the exempt LDV lot, then a GAIN lot (50) before
        // the second LDV lot (90)
        assert_eq!(
            lots_of(&plan),
            [
                ("LOSS".to_string(), dec!(2)),
                ("GAIN".to_string(), dec!(1)),
                ("LDV".to_string(), dec!(1))
            ]
        );
        // Losses of 40 offset the gain of 50: tax on 10
        assert_eq!(plan.tax, Some(rub(dec!(1.3))));
    }

    #[test]
    fn taxed_sell_of_unknown_cost_comes_last_and_leaves_tax_unknown() {
        // Arrange
        let known = Tradable {
            tax_lots: Some(vec![tax_lot("2025-01-10", 1, 50, 100)]),
            ..tradable(InstrumentKind::Share, "KNOWN", 1, 100, 1)
        };
        let tradables = [tradable(InstrumentKind::Share, "UNKNOWN", 1, 100, 5), known];
        let analysis = analysis(&[(InstrumentKind::Share, RebalanceAction::Sell, 200)]);
        let year = YearIncome {
            sales: vec![SecuritySales {
                ticker: "X".to_string(),
                lots: Vec::new(),
                complete: true,
            }],
            ..YearIncome::zero()
        };

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Taxed(year));

        // Assert
        assert_eq!(
            lots_of(&plan),
            [
                ("UNKNOWN".to_string(), dec!(1)),
                ("KNOWN".to_string(), dec!(1))
            ]
        );
        assert_eq!(plan.tax, None);
    }

    #[test]
    fn sell_of_several_accounts_has_unknown_tax() {
        // Arrange
        let tradables = [tradable(InstrumentKind::Bond, "OFZ", 1, 1000, 10)];
        let analysis = analysis(&[(InstrumentKind::Bond, RebalanceAction::Sell, 2500)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Unknown);

        // Assert
        assert_eq!(lots_of(&plan), [("OFZ".to_string(), dec!(2))]);
        assert_eq!(plan.assets[0].left.value, dec!(500));
        assert_eq!(plan.tax, None);
    }

    #[test]
    fn hold_recommendations_have_no_orders() {
        // Arrange
        let tradables = [tradable(InstrumentKind::Bond, "OFZ", 1, 1000, 10)];
        let analysis = analysis(&[(InstrumentKind::Bond, RebalanceAction::Hold, 0)]);

        // Act
        let plan = TradePlan::new(&analysis, &tradables, &SaleTaxation::Untaxed);

        // Assert
        assert_eq!(plan.assets, Vec::new());
    }
}
