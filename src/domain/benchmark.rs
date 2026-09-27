//! Comparison of returns with market indices: the same payments invested into an index.

use std::collections::BTreeMap;

use chrono::{DateTime, Days, NaiveDate, Utc};
use rust_decimal::Decimal;

use super::paper::Profit;
use super::portfolio::{Asset, Portfolio};
use super::xirr::{CashFlow, xirr};

/// Index close prices by trading day
pub type IndexPrices = BTreeMap<NaiveDate, Decimal>;

/// Market index returns are compared with
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Benchmark {
    pub ticker: &'static str,
    pub description: &'static str,
}

/// Indices of shares and government bonds: price ones with a long history
/// and total return ones (dividends and coupons reinvested) with a shorter one.
pub const BENCHMARKS: [Benchmark; 4] = [
    Benchmark {
        ticker: "IMOEX",
        description: "MOEX Russia index, price only",
    },
    Benchmark {
        ticker: "MCFTR",
        description: "MOEX Russia total return index, dividends included",
    },
    Benchmark {
        ticker: "RGBI",
        description: "Government bonds index, price only",
    },
    Benchmark {
        ticker: "RGBITR",
        description: "Government bonds total return index, coupons included",
    },
];

/// Days a price is looked back for when a date has no trading.
const LOOKBACK_DAYS: u64 = 10;

/// Close of `date` or of the closest earlier trading day within [`LOOKBACK_DAYS`].
fn price_on(prices: &IndexPrices, date: NaiveDate) -> Option<Decimal> {
    let from = date.checked_sub_days(Days::new(LOOKBACK_DAYS))?;
    prices
        .range(from..=date)
        .next_back()
        .map(|(_, price)| *price)
        .filter(|price| *price > Decimal::ZERO)
}

/// Annual return (XIRR) of investing `payments` into the index instead (public market equivalent).
///
/// Every payment buys index units at its date close when negative and sells them when positive;
/// the units left are valued at `at`. `None` when the index has no price near some payment date
/// or at `at`, or when the payments sell more units than they bought.
#[must_use]
pub fn index_xirr(
    payments: &[CashFlow],
    prices: &IndexPrices,
    at: DateTime<Utc>,
) -> Option<Decimal> {
    let units = payments.iter().try_fold(Decimal::ZERO, |units, flow| {
        let price = price_on(prices, flow.date.date_naive())?;
        units.checked_sub(flow.amount.checked_div(price)?)
    })?;
    let value = units.checked_mul(price_on(prices, at.date_naive())?)?;
    if value <= Decimal::ZERO {
        return None;
    }
    let flows: Vec<CashFlow> = payments
        .iter()
        .copied()
        .chain(std::iter::once(CashFlow {
            date: at,
            amount: value,
        }))
        .collect();
    xirr(&flows)
}

/// Benchmark with its price history; empty when the index could not be loaded
pub struct IndexHistory {
    pub benchmark: Benchmark,
    pub prices: IndexPrices,
}

/// Return of payments and of the same payments invested into every benchmark
pub struct BenchmarkRow {
    pub name: &'static str,
    pub xirr: Option<Decimal>,
    /// In the order of [`BenchmarkComparison::indices`]
    pub benchmarks: Vec<Option<Decimal>>,
}

/// Returns of the portfolio and its asset types compared with benchmarks
pub struct BenchmarkComparison {
    pub indices: Vec<IndexHistory>,
    /// All securities, then not empty shares, bonds and ETFs
    pub rows: Vec<BenchmarkRow>,
}

impl BenchmarkComparison {
    /// Compares returns of the portfolio papers as if sold at `at` with `indices`.
    ///
    /// Currencies are left out of securities: they are cash, their payments are conversions
    /// that may come from papers sold long ago and distort the return.
    #[must_use]
    pub fn new(portfolio: &Portfolio, indices: Vec<IndexHistory>, at: DateTime<Utc>) -> Self {
        let payments = [
            portfolio.shares.payments(),
            portfolio.bonds.payments(),
            portfolio.etfs.payments(),
            portfolio.futures.payments(),
        ]
        .concat();
        let flows = [
            portfolio.shares.cash_flows_until(at),
            portfolio.bonds.cash_flows_until(at),
            portfolio.etfs.cash_flows_until(at),
            portfolio.futures.cash_flows_until(at),
        ]
        .concat();
        let securities = benchmark_row("Securities", &payments, xirr(&flows), &indices, at);
        let rows = std::iter::once(securities)
            .chain(asset_row("Shares", &portfolio.shares, &indices, at))
            .chain(asset_row("Bonds", &portfolio.bonds, &indices, at))
            .chain(asset_row("ETFs", &portfolio.etfs, &indices, at))
            .collect();
        Self { indices, rows }
    }
}

fn benchmark_row(
    name: &'static str,
    payments: &[CashFlow],
    xirr: Option<Decimal>,
    indices: &[IndexHistory],
    at: DateTime<Utc>,
) -> BenchmarkRow {
    BenchmarkRow {
        name,
        xirr,
        benchmarks: indices
            .iter()
            .map(|i| index_xirr(payments, &i.prices, at))
            .collect(),
    }
}

/// Row of a not empty asset.
fn asset_row<P: Profit>(
    name: &'static str,
    asset: &Asset<P>,
    indices: &[IndexHistory],
    at: DateTime<Utc>,
) -> Option<BenchmarkRow> {
    (!asset.is_empty()).then(|| benchmark_row(name, &asset.payments(), asset.xirr(at), indices, at))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::{CouponProfit, Figi, Money, NoneProfit, Paper, Position, Ticker, Totals};

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn flow(date: DateTime<Utc>, amount: Decimal) -> CashFlow {
        CashFlow { date, amount }
    }

    fn prices(points: &[(NaiveDate, Decimal)]) -> IndexPrices {
        points.iter().copied().collect()
    }

    #[test]
    fn index_xirr_follows_index_growth() {
        // Arrange: the index grows 10% in a year
        let prices = prices(&[(day(2025, 1, 1), dec!(100)), (day(2026, 1, 1), dec!(110))]);
        let payments = [flow(at(2025, 1, 1), dec!(-1000))];

        // Act
        let rate = index_xirr(&payments, &prices, at(2026, 1, 1));

        // Assert
        assert_eq!(rate.map(|r| r.round_dp(4)), Some(dec!(0.1)));
    }

    #[test]
    fn index_xirr_sells_units_on_income() {
        // Arrange: 10 units bought at 100, 5 sold at 200 as income, 5 left worth 1000
        let prices = prices(&[
            (day(2025, 1, 1), dec!(100)),
            (day(2026, 1, 1), dec!(200)),
            (day(2027, 1, 1), dec!(200)),
        ]);
        let payments = [
            flow(at(2025, 1, 1), dec!(-1000)),
            flow(at(2026, 1, 1), dec!(1000)),
        ];

        // Act
        let rate = index_xirr(&payments, &prices, at(2027, 1, 1));

        // Assert: -1000, +1000 a year later, +1000 two years later
        assert_eq!(rate.map(|r| r.round_dp(4)), Some(dec!(0.618)));
    }

    #[test]
    fn index_xirr_takes_earlier_trading_day_price() {
        // Arrange: payment on a weekend, the price of Friday is taken
        let prices = prices(&[(day(2025, 1, 3), dec!(100)), (day(2026, 1, 2), dec!(120))]);
        let payments = [flow(at(2025, 1, 4), dec!(-1000))];

        // Act
        let rate = index_xirr(&payments, &prices, at(2026, 1, 4));

        // Assert
        assert!(rate.is_some_and(|r| r > dec!(0.19) && r < dec!(0.21)));
    }

    #[rstest::rstest]
    #[case::payment_before_index_history(at(2020, 1, 1))]
    #[case::payment_in_long_gap(at(2025, 6, 1))]
    fn index_xirr_without_price_is_none(#[case] date: DateTime<Utc>) {
        // Arrange
        let prices = prices(&[(day(2025, 1, 1), dec!(100)), (day(2026, 1, 1), dec!(110))]);
        let payments = [flow(date, dec!(-1000))];

        // Act
        let rate = index_xirr(&payments, &prices, at(2026, 1, 1));

        // Assert
        assert_eq!(rate, None);
    }

    #[test]
    fn comparison_has_rows_of_not_empty_assets() {
        // Arrange
        let mut portfolio = Portfolio::new(false);
        let mut paper = test_paper();
        paper.totals.cash_flows = vec![flow(at(2025, 1, 1), dec!(-1000))];
        portfolio.bonds.add_paper(paper);
        let indices = vec![IndexHistory {
            benchmark: BENCHMARKS[3],
            prices: prices(&[(day(2025, 1, 1), dec!(100)), (day(2026, 1, 1), dec!(105))]),
        }];

        // Act
        let comparison = BenchmarkComparison::new(&portfolio, indices, at(2026, 1, 1));

        // Assert
        let names: Vec<&str> = comparison.rows.iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["Securities", "Bonds"]);
        let bonds = &comparison.rows[1];
        assert_eq!(bonds.xirr.map(|r| r.round_dp(4)), Some(dec!(0.1)));
        let rounded: Vec<Option<Decimal>> = bonds
            .benchmarks
            .iter()
            .map(|r| r.map(|r| r.round_dp(4)))
            .collect();
        assert_eq!(rounded, vec![Some(dec!(0.05))]);
    }

    /// Bond worth 1100 RUB now.
    fn test_paper() -> Paper<CouponProfit> {
        let rub = |value| Money::from_value(value, Currency::RUB);
        Paper {
            name: "Bond".to_string(),
            ticker: Ticker::new("BND"),
            figi: Figi::new("FIGI"),
            position: Position {
                currency: Currency::RUB,
                average_buy_price: rub(dec!(1000)),
                current_instrument_price: rub(dec!(1100)),
                accrued_interest: rub(dec!(0)),
                quantity: dec!(1),
                daily_yield: rub(dec!(0)),
                blocked: false,
                blocked_lots: dec!(0),
            },
            totals: Totals {
                additional_profit: rub(dec!(0)),
                fees: rub(dec!(0)),
                cash_flows: vec![],
            },
            profit: CouponProfit,
            bond: None,
            sector: None,
        }
    }

    #[test]
    fn securities_leave_currencies_out() {
        // Arrange: currency sold for 5000 with nothing invested would distort the return
        let mut portfolio = Portfolio::new(false);
        let mut bond = test_paper();
        bond.totals.cash_flows = vec![flow(at(2025, 1, 1), dec!(-1000))];
        portfolio.bonds.add_paper(bond);
        let mut currency = test_paper().with_profit(NoneProfit);
        currency.totals.cash_flows = vec![flow(at(2025, 1, 1), dec!(5000))];
        portfolio.currencies.add_paper(currency);

        // Act
        let comparison = BenchmarkComparison::new(&portfolio, vec![], at(2026, 1, 1));

        // Assert
        assert_eq!(
            comparison.rows[0].xirr.map(|r| r.round_dp(4)),
            Some(dec!(0.1))
        );
    }

    #[test]
    fn index_xirr_is_none_when_more_sold_than_bought() {
        // Arrange: the index fell by half, income exceeds what is left
        let prices = prices(&[
            (day(2024, 1, 1), dec!(100)),
            (day(2025, 1, 1), dec!(50)),
            (day(2026, 1, 1), dec!(50)),
        ]);
        let payments = [
            flow(at(2024, 1, 1), dec!(-1000)),
            flow(at(2025, 1, 1), dec!(600)),
        ];

        // Act
        let rate = index_xirr(&payments, &prices, at(2026, 1, 1));

        // Assert
        assert_eq!(rate, None);
    }
}
