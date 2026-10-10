use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Market-data instrument used as an FX quote for a currency vs RUB.
#[derive(Debug, Clone)]
pub struct FxInstrument {
    pub instrument_id: String,
    pub nominal: Decimal,
}

/// Converts a market quote into RUB per 1 unit of the foreign currency.
///
/// The quote is the price of `nominal` units; the lot size does not affect it,
/// `price * lot / nominal` from T-Invest marketdata FAQ is the price of a whole lot.
#[must_use]
pub fn quote_to_rub_rate(price: Decimal, nominal: Decimal) -> Decimal {
    let nominal = if nominal.is_zero() {
        Decimal::ONE
    } else {
        nominal.abs()
    };
    price / nominal
}

/// Candidate row when building the FX instrument map from the Currencies catalog.
#[derive(Debug, Clone)]
pub struct FxCandidate {
    pub currency: Currency,
    pub instrument_id: String,
    pub lot: i32,
    pub nominal: Decimal,
    pub settlement_is_rub: bool,
}

/// Picks the best FX instrument per currency: prefer RUB settlement, nominal 1, smallest lot.
#[must_use]
pub fn build_fx_map(
    candidates: impl IntoIterator<Item = FxCandidate>,
) -> std::collections::HashMap<Currency, FxInstrument> {
    let mut best: std::collections::HashMap<Currency, (FxCandidate, i32)> =
        std::collections::HashMap::new();

    for c in candidates {
        if c.currency == Currency::RUB {
            continue;
        }
        let score = fx_score(&c);
        match best.get(&c.currency) {
            Some((_, prev)) if *prev >= score => {}
            _ => {
                best.insert(c.currency, (c, score));
            }
        }
    }

    best.into_iter()
        .map(|(currency, (c, _))| {
            (
                currency,
                FxInstrument {
                    instrument_id: c.instrument_id,
                    nominal: c.nominal,
                },
            )
        })
        .collect()
}

fn fx_score(c: &FxCandidate) -> i32 {
    let mut score = 0;
    if c.settlement_is_rub {
        score += 100;
    }
    if c.nominal == dec!(1) {
        score += 50;
    }
    // Prefer smaller lots (USD000UTSTOM lot=1 over lot=1000).
    score += (1000 - c.lot.clamp(1, 1000)) / 10;
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn quote_to_rub_rate_unit_nominal() {
        assert_eq!(quote_to_rub_rate(dec!(90.5), dec!(1)), dec!(90.5));
    }

    #[test]
    fn quote_to_rub_rate_divides_by_nominal() {
        // JPY-style: the price of 100 yen
        assert_eq!(quote_to_rub_rate(dec!(50), dec!(100)), dec!(0.5));
    }

    #[test]
    fn build_fx_map_prefers_lot_one_rub_settlement() {
        let map = build_fx_map([
            FxCandidate {
                currency: Currency::USD,
                instrument_id: "lot1000".into(),
                lot: 1000,
                nominal: dec!(1),
                settlement_is_rub: true,
            },
            FxCandidate {
                currency: Currency::USD,
                instrument_id: "lot1".into(),
                lot: 1,
                nominal: dec!(1),
                settlement_is_rub: true,
            },
        ]);
        assert_eq!(map[&Currency::USD].instrument_id, "lot1");
    }
}
