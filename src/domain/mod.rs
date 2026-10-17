use chrono::{DateTime, NaiveDate, TimeDelta, Utc};

pub mod analytics;
pub mod benchmark;
pub mod bond;
pub mod calendar;
pub mod display;
pub mod fx;
pub mod history;
pub mod income;
pub mod money;
pub mod paper;
pub mod portfolio;
pub mod risk;
pub mod tax;
pub mod xirr;

pub use calendar::{
    CalendarPayment, CouponCalendar, CouponPayment, DividendCalendar, DividendPayment,
};
pub use history::{History, HistoryItem};
pub use money::{Income, Money};
pub use paper::{
    BondInfo, CouponProfit, DividendProfit, Figi, Instrument, InstrumentKind, NoneProfit, Paper,
    Position, Profit, Ticker, Totals,
};
pub use portfolio::{Asset, LoadedPaper, Portfolio};

/// Moscow time offset from UTC; Russia has no daylight saving time since 2014.
const MOSCOW_OFFSET: TimeDelta = TimeDelta::hours(3);

/// Calendar date in Moscow, where the exchange trades and the broker keeps its books.
///
/// The API returns times in UTC, so an operation made shortly after midnight in Moscow
/// has the previous day as its UTC date.
///
/// # Examples
///
/// ```
/// use chrono::{NaiveDate, TimeZone, Utc};
/// use tinkoff::domain::moscow_date;
///
/// let after_midnight = Utc.with_ymd_and_hms(2025, 12, 31, 22, 0, 0).unwrap();
/// assert_eq!(moscow_date(after_midnight), NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
/// ```
#[must_use]
pub fn moscow_date(time: DateTime<Utc>) -> NaiveDate {
    (time + MOSCOW_OFFSET).date_naive()
}

/// Numeric value that can be classified as negative, zero, or positive (for table coloring).
///
/// This trait is used for UI colorization of numeric values in tables.
/// Implementations should check the sign of the value.
pub trait NumberRange {
    /// Returns `true` if the value is negative (less than zero).
    fn is_negative(&self) -> bool;

    /// Returns `true` if the value is exactly zero.
    fn is_zero(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::moscow_evening("2026-01-30T20:59:59Z", "2026-01-30")]
    #[case::moscow_midnight("2026-01-30T21:00:00Z", "2026-01-31")]
    #[case::utc_midnight("2026-01-31T00:00:00Z", "2026-01-31")]
    #[case::new_year_in_moscow("2025-12-31T22:00:00Z", "2026-01-01")]
    fn moscow_date_is_utc_plus_three(#[case] time: &str, #[case] expected: &str) {
        // Arrange
        let time: DateTime<Utc> = time.parse().unwrap();

        // Act
        let date = moscow_date(time);

        // Assert
        assert_eq!(date, expected.parse::<NaiveDate>().unwrap());
    }
}
