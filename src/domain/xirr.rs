//! Annual internal rate of return for irregular cash flows.

use std::fmt;

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;

use super::NumberRange;

/// Money flow on a date: negative for investments, positive for returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CashFlow {
    pub date: DateTime<Utc>,
    pub amount: Decimal,
}

/// Annual rate displayed as percents, e.g. `12.34%`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnualRate(pub Decimal);

impl fmt::Display for AnnualRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", (self.0 * dec!(100)).round_dp(2))
    }
}

impl NumberRange for AnnualRate {
    fn is_negative(&self) -> bool {
        self.0.is_sign_negative() && !self.0.is_zero()
    }

    fn is_zero(&self) -> bool {
        self.0.is_zero()
    }
}

/// Lowest annual rate searched, -95%.
const LOWER_RATE: Decimal = dec!(-0.95);
/// Highest annual rate searched, 1000%.
const UPPER_RATE: Decimal = dec!(10);
/// Rate precision the search stops at.
const TOLERANCE: Decimal = dec!(0.0000001);
const MAX_ITERATIONS: usize = 200;
const SECONDS_IN_YEAR: Decimal = dec!(31_536_000);

/// Annual rate at which the net present value of `flows` is zero (XIRR, Actual/365).
///
/// Returns `None` when flows do not have both investments and returns, span less than
/// a day, or the rate is outside -95%..1000% a year.
///
/// # Examples
///
/// ```
/// use chrono::{TimeZone, Utc};
/// use rust_decimal_macros::dec;
/// use tinkoff::domain::xirr::{CashFlow, xirr};
///
/// let flows = [
///     CashFlow { date: Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), amount: dec!(-1000) },
///     CashFlow { date: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(), amount: dec!(1100) },
/// ];
/// assert_eq!(xirr(&flows).map(|r| r.round_dp(4)), Some(dec!(0.1)));
/// ```
#[must_use]
pub fn xirr(flows: &[CashFlow]) -> Option<Decimal> {
    let has_investment = flows.iter().any(|f| f.amount < Decimal::ZERO);
    let has_return = flows.iter().any(|f| f.amount > Decimal::ZERO);
    let start = flows.iter().map(|f| f.date).min()?;
    let end = flows.iter().map(|f| f.date).max()?;
    if !has_investment || !has_return || (end - start).num_days() < 1 {
        return None;
    }

    let timed: Vec<(Decimal, Decimal)> = flows
        .iter()
        .map(|f| {
            let years = Decimal::from((f.date - start).num_seconds()) / SECONDS_IN_YEAR;
            (years, f.amount)
        })
        .collect();
    let npv = |rate: Decimal| net_present_value(&timed, rate);

    let (mut low, mut high) = (LOWER_RATE, UPPER_RATE);
    let mut npv_low = npv(low)?;
    let npv_high = npv(high)?;
    if npv_low.is_zero() {
        return Some(low);
    }
    if npv_high.is_zero() {
        return Some(high);
    }
    if npv_low.is_sign_negative() == npv_high.is_sign_negative() {
        return None;
    }

    for _ in 0..MAX_ITERATIONS {
        let middle = (low + high) / dec!(2);
        let npv_middle = npv(middle)?;
        if npv_middle.is_zero() || high - low < TOLERANCE {
            return Some(middle);
        }
        if npv_middle.is_sign_negative() == npv_low.is_sign_negative() {
            low = middle;
            npv_low = npv_middle;
        } else {
            high = middle;
        }
    }
    Some((low + high) / dec!(2))
}

/// Sum of `amount / (1 + rate)^years`; `None` on arithmetic overflow.
fn net_present_value(timed: &[(Decimal, Decimal)], rate: Decimal) -> Option<Decimal> {
    let base = Decimal::ONE + rate;
    timed
        .iter()
        .try_fold(Decimal::ZERO, |acc, (years, amount)| {
            let factor = base.checked_powd(*years)?;
            if factor.is_zero() {
                return None;
            }
            acc.checked_add(amount.checked_div(factor)?)
        })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use rstest::rstest;

    use super::*;

    fn flow(days: i64, amount: Decimal) -> CashFlow {
        let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        CashFlow {
            date: start + Duration::days(days),
            amount,
        }
    }

    #[rstest]
    #[case::ten_percent_a_year(vec![flow(0, dec!(-1000)), flow(365, dec!(1100))], dec!(0.1))]
    #[case::ten_percent_over_two_years(vec![flow(0, dec!(-1000)), flow(730, dec!(1210))], dec!(0.1))]
    #[case::loss(vec![flow(0, dec!(-1000)), flow(365, dec!(900))], dec!(-0.1))]
    #[case::coupons(
        vec![flow(0, dec!(-1000)), flow(182, dec!(50)), flow(365, dec!(1050))],
        dec!(0.1025)
    )]
    #[case::unordered(vec![flow(365, dec!(1100)), flow(0, dec!(-1000))], dec!(0.1))]
    fn xirr_finds_annual_rate(#[case] flows: Vec<CashFlow>, #[case] expected: Decimal) {
        // Arrange

        // Act
        let rate = xirr(&flows);

        // Assert
        assert_eq!(rate.map(|r| r.round_dp(4)), Some(expected));
    }

    #[rstest]
    #[case(dec!(0.123456), "12.35%")]
    #[case(dec!(-0.05), "-5.00%")]
    #[case(dec!(0), "0%")]
    fn annual_rate_displays_percents(#[case] rate: Decimal, #[case] expected: &str) {
        // Arrange
        let rate = AnnualRate(rate);

        // Act
        let text = rate.to_string();

        // Assert
        assert_eq!(text, expected);
    }

    #[rstest]
    #[case::empty(vec![])]
    #[case::only_investments(vec![flow(0, dec!(-1000)), flow(365, dec!(-10))])]
    #[case::only_returns(vec![flow(0, dec!(1000)), flow(365, dec!(10))])]
    #[case::same_day(vec![flow(0, dec!(-1000)), flow(0, dec!(1100))])]
    #[case::beyond_upper_rate(vec![flow(0, dec!(-1000)), flow(30, dec!(2000))])]
    fn xirr_undefined(#[case] flows: Vec<CashFlow>) {
        // Arrange

        // Act
        let rate = xirr(&flows);

        // Assert
        assert_eq!(rate, None);
    }
}
