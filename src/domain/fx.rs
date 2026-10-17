use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use color_eyre::eyre::{self, Result, WrapErr};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tokio::sync::OnceCell;

use super::{Money, moscow_date};

/// How many days back to look for an FX candle; covers the New Year holidays.
const FX_LOOKBACK_DAYS: i64 = 14;

/// Close prices by Moscow date.
pub type DailyPrices = BTreeMap<NaiveDate, Decimal>;

/// Lazily loaded closes of one currency for one calendar year.
type YearCloses = Arc<OnceCell<DailyPrices>>;

/// Daily close prices of an instrument; RUB rates are derived from them.
pub trait DailyCloses: Send + Sync {
    /// Closes of `instrument_id` within calendar `year` keyed by Moscow date.
    /// Implementations retry transient failures themselves.
    fn daily_closes(
        &self,
        instrument_id: &str,
        year: i32,
    ) -> impl Future<Output = Result<DailyPrices>> + Send;
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

/// Converts money to RUB at historical daily rates.
pub struct RubConverter<S> {
    instruments: HashMap<Currency, FxInstrument>,
    source: S,
    /// Closes loaded once per currency and calendar year; concurrent requests share one load.
    closes: Mutex<HashMap<(Currency, i32), YearCloses>>,
}

impl<S: DailyCloses> RubConverter<S> {
    /// Picks the best FX instrument per currency from the Currencies catalog
    /// and loads their closes from `source` on demand.
    #[must_use]
    pub fn new(candidates: impl IntoIterator<Item = FxCandidate>, source: S) -> Self {
        Self {
            instruments: build_fx_map(candidates),
            source,
            closes: Mutex::new(HashMap::new()),
        }
    }

    /// `money` in RUB at the rate of `at`'s Moscow date, or of the closest
    /// earlier trading day within [`FX_LOOKBACK_DAYS`]. RUB is returned as is.
    ///
    /// Closes of a currency are loaded once per calendar year; a failed load
    /// is not cached and is repeated by the next call.
    ///
    /// # Errors
    ///
    /// Returns an error if the currency has no FX instrument, there is no candle
    /// within the lookback window, or closes cannot be loaded.
    pub async fn to_rub(&self, money: Money, at: DateTime<Utc>) -> Result<Money> {
        if money.currency == Currency::RUB {
            return Ok(money);
        }
        let currency = money.currency;
        let instrument = self
            .instruments
            .get(&currency)
            .ok_or_else(|| eyre::eyre!("No FX instrument for currency {currency:?}"))?;
        let date = moscow_date(at);
        // Weekends and holidays have no candle: take the closest earlier trading day.
        for offset in 0..=FX_LOOKBACK_DAYS {
            let Some(day) = date.checked_sub_signed(Duration::days(offset)) else {
                break;
            };
            if let Some(close) = self.close(currency, instrument, day).await? {
                let rate = quote_to_rub_rate(close, instrument.nominal);
                return Ok(money.to_currency(rate, Currency::RUB));
            }
        }
        Err(eyre::eyre!("No FX candle for {currency:?} around {date}"))
    }

    /// Close of `day` from the closes of its year, loading them on first use.
    async fn close(
        &self,
        currency: Currency,
        instrument: &FxInstrument,
        day: NaiveDate,
    ) -> Result<Option<Decimal>> {
        let year = day.year();
        let cell = {
            let mut cells = self
                .closes
                .lock()
                .map_err(|e| eyre::eyre!("FX cache is poisoned: {e}"))?;
            Arc::clone(cells.entry((currency, year)).or_default())
        };
        let closes = cell
            .get_or_try_init(|| async {
                self.source
                    .daily_closes(&instrument.instrument_id, year)
                    .await
                    .wrap_err_with(|| format!("FX rates of {currency:?} in {year} not loaded"))
            })
            .await?;
        Ok(closes.get(&day).copied())
    }
}

/// Market-data instrument used as an FX quote for a currency vs RUB.
#[derive(Debug, Clone)]
struct FxInstrument {
    instrument_id: String,
    nominal: Decimal,
}

/// Converts a market quote into RUB per 1 unit of the foreign currency.
///
/// The quote is the price of `nominal` units; the lot size does not affect it,
/// `price * lot / nominal` from T-Invest marketdata FAQ is the price of a whole lot.
fn quote_to_rub_rate(price: Decimal, nominal: Decimal) -> Decimal {
    let nominal = if nominal.is_zero() {
        Decimal::ONE
    } else {
        nominal.abs()
    };
    price / nominal
}

/// Picks the best FX instrument per currency: prefer RUB settlement, nominal 1, smallest lot.
fn build_fx_map(
    candidates: impl IntoIterator<Item = FxCandidate>,
) -> HashMap<Currency, FxInstrument> {
    let mut best: HashMap<Currency, (FxCandidate, i32)> = HashMap::new();

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
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Closes source serving fixed prices and counting loads.
    #[derive(Default)]
    struct FakeCloses {
        prices: HashMap<String, DailyPrices>,
        calls: AtomicU32,
        /// Number of first loads that fail.
        failures: u32,
    }

    impl FakeCloses {
        fn with(instrument_id: &str, prices: &[(NaiveDate, Decimal)]) -> Self {
            Self {
                prices: HashMap::from([(
                    instrument_id.to_string(),
                    prices.iter().copied().collect(),
                )]),
                ..Self::default()
            }
        }
    }

    impl DailyCloses for FakeCloses {
        async fn daily_closes(&self, instrument_id: &str, year: i32) -> Result<DailyPrices> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            if call < self.failures {
                return Err(eyre::eyre!("source failed"));
            }
            Ok(self
                .prices
                .get(instrument_id)
                .into_iter()
                .flatten()
                .filter(|(date, _)| date.year() == year)
                .map(|(date, close)| (*date, *close))
                .collect())
        }
    }

    fn candidate(
        currency: Currency,
        instrument_id: &str,
        lot: i32,
        nominal: Decimal,
    ) -> FxCandidate {
        FxCandidate {
            currency,
            instrument_id: instrument_id.to_string(),
            lot,
            nominal,
            settlement_is_rub: true,
        }
    }

    fn usd_converter(source: FakeCloses) -> RubConverter<FakeCloses> {
        RubConverter::new([candidate(Currency::USD, "USD", 1, dec!(1))], source)
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// Noon in Moscow of the given date.
    fn moscow_noon(date: NaiveDate) -> DateTime<Utc> {
        date.and_hms_opt(9, 0, 0).unwrap().and_utc()
    }

    fn usd(value: Decimal) -> Money {
        Money::from_value(value, Currency::USD)
    }

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
            candidate(Currency::USD, "lot1000", 1000, dec!(1)),
            candidate(Currency::USD, "lot1", 1, dec!(1)),
        ]);
        assert_eq!(map[&Currency::USD].instrument_id, "lot1");
    }

    #[tokio::test]
    async fn to_rub_keeps_rub_without_loading() {
        // Arrange
        let fx = usd_converter(FakeCloses::default());
        let money = Money::from_value(dec!(100), Currency::RUB);

        // Act
        let rub = fx
            .to_rub(money, moscow_noon(day(2026, 9, 25)))
            .await
            .unwrap();

        // Assert
        assert_eq!(rub, money);
        assert_eq!(fx.source.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn to_rub_at_rate_of_the_day() {
        // Arrange
        let fx = usd_converter(FakeCloses::with(
            "USD",
            &[(day(2026, 9, 24), dec!(90)), (day(2026, 9, 25), dec!(91))],
        ));

        // Act
        let rub = fx
            .to_rub(usd(dec!(2)), moscow_noon(day(2026, 9, 25)))
            .await
            .unwrap();

        // Assert
        assert_eq!(rub, Money::from_value(dec!(182), Currency::RUB));
    }

    #[tokio::test]
    async fn to_rub_by_moscow_date() {
        // Arrange
        let fx = usd_converter(FakeCloses::with(
            "USD",
            &[(day(2026, 9, 24), dec!(90)), (day(2026, 9, 25), dec!(91))],
        ));
        let after_moscow_midnight = day(2026, 9, 24).and_hms_opt(21, 30, 0).unwrap().and_utc();

        // Act
        let rub = fx
            .to_rub(usd(dec!(1)), after_moscow_midnight)
            .await
            .unwrap();

        // Assert
        assert_eq!(rub.value, dec!(91));
    }

    #[tokio::test]
    async fn to_rub_on_weekend_takes_last_trading_day() {
        // Arrange
        let friday = day(2026, 9, 25);
        let sunday = day(2026, 9, 27);
        let fx = usd_converter(FakeCloses::with("USD", &[(friday, dec!(91))]));

        // Act
        let rub = fx.to_rub(usd(dec!(1)), moscow_noon(sunday)).await.unwrap();

        // Assert
        assert_eq!(rub.value, dec!(91));
    }

    #[tokio::test]
    async fn to_rub_looks_back_into_previous_year() {
        // Arrange
        let fx = usd_converter(FakeCloses::with("USD", &[(day(2025, 12, 30), dec!(78))]));

        // Act
        let rub = fx
            .to_rub(usd(dec!(1)), moscow_noon(day(2026, 1, 5)))
            .await
            .unwrap();

        // Assert
        assert_eq!(rub.value, dec!(78));
        assert_eq!(fx.source.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn to_rub_fails_beyond_lookback() {
        // Arrange
        let fx = usd_converter(FakeCloses::with("USD", &[(day(2026, 9, 1), dec!(90))]));

        // Act
        let result = fx.to_rub(usd(dec!(1)), moscow_noon(day(2026, 9, 25))).await;

        // Assert
        let message = result.unwrap_err().to_string();
        assert!(message.contains("No FX candle"), "{message}");
    }

    #[tokio::test]
    async fn to_rub_fails_without_instrument() {
        // Arrange
        let fx = usd_converter(FakeCloses::default());
        let eur = Money::from_value(dec!(1), Currency::EUR);

        // Act
        let result = fx.to_rub(eur, moscow_noon(day(2026, 9, 25))).await;

        // Assert
        let message = result.unwrap_err().to_string();
        assert!(message.contains("No FX instrument"), "{message}");
    }

    #[tokio::test]
    async fn to_rub_divides_quote_by_nominal() {
        // Arrange
        let date = day(2026, 9, 25);
        let fx = RubConverter::new(
            [candidate(Currency::KZT, "KZT", 1, dec!(100))],
            FakeCloses::with("KZT", &[(date, dec!(18))]),
        );
        let kzt = Money::from_value(dec!(1000), Currency::KZT);

        // Act
        let rub = fx.to_rub(kzt, moscow_noon(date)).await.unwrap();

        // Assert
        assert_eq!(rub.value, dec!(180));
    }

    #[tokio::test]
    async fn to_rub_ignores_lot_size() {
        // Arrange
        let date = day(2026, 9, 25);
        let fx = RubConverter::new(
            [candidate(Currency::USD, "USD000UTSTOM", 1000, dec!(1))],
            FakeCloses::with("USD000UTSTOM", &[(date, dec!(90))]),
        );

        // Act
        let rub = fx.to_rub(usd(dec!(1)), moscow_noon(date)).await.unwrap();

        // Assert
        assert_eq!(rub.value, dec!(90));
    }

    #[tokio::test]
    async fn to_rub_loads_year_once_for_concurrent_calls() {
        // Arrange
        let fx = usd_converter(FakeCloses::with(
            "USD",
            &[(day(2026, 9, 24), dec!(90)), (day(2026, 9, 25), dec!(91))],
        ));

        // Act
        let (first, second) = tokio::join!(
            fx.to_rub(usd(dec!(1)), moscow_noon(day(2026, 9, 24))),
            fx.to_rub(usd(dec!(1)), moscow_noon(day(2026, 9, 25))),
        );

        // Assert
        assert_eq!(first.unwrap().value, dec!(90));
        assert_eq!(second.unwrap().value, dec!(91));
        assert_eq!(fx.source.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn to_rub_repeats_failed_load() {
        // Arrange
        let date = day(2026, 9, 25);
        let fx = usd_converter(FakeCloses {
            failures: 1,
            ..FakeCloses::with("USD", &[(date, dec!(91))])
        });

        // Act
        let failed = fx.to_rub(usd(dec!(1)), moscow_noon(date)).await;
        let repeated = fx.to_rub(usd(dec!(1)), moscow_noon(date)).await;

        // Assert
        assert!(failed.is_err());
        assert_eq!(repeated.unwrap().value, dec!(91));
        assert_eq!(fx.source.calls.load(Ordering::SeqCst), 2);
    }
}
