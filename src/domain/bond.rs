//! Bond events: coupons, amortizations, maturity and offers, yield to maturity and duration.

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, MathematicalOps};

use super::money::Money;
use super::paper::BondInfo;
use super::xirr::{CashFlow, xirr, years_between};

/// Kind of a bond event
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondEventKind {
    Coupon,
    /// Partial redemption of the nominal
    Amortization,
    /// Final redemption of the nominal
    Maturity,
    /// Right to present the bond for redemption; not a guaranteed payment
    Offer,
}

impl BondEventKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Coupon => "Coupon",
            Self::Amortization => "Amortization",
            Self::Maturity => "Maturity",
            Self::Offer => "Offer",
        }
    }

    /// Whether the event pays money to the holder
    #[must_use]
    pub const fn is_payment(self) -> bool {
        !matches!(self, Self::Offer)
    }
}

/// Bond event with its payment per one bond (zero when not known yet)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BondEvent {
    pub kind: BondEventKind,
    pub date: DateTime<Utc>,
    pub per_bond: Money,
}

/// Marks every redemption but the latest one as amortization.
///
/// The API reports partial and final redemptions alike, the final one is the latest.
pub fn mark_amortizations(events: &mut [BondEvent]) {
    let Some(maturity) = events
        .iter()
        .filter(|e| e.kind == BondEventKind::Maturity)
        .map(|e| e.date)
        .max()
    else {
        return;
    };
    for event in events
        .iter_mut()
        .filter(|e| e.kind == BondEventKind::Maturity && e.date < maturity)
    {
        event.kind = BondEventKind::Amortization;
    }
}

/// Maturity, next offer, yields and duration of a bond bought at `dirty_price` at `now`.
///
/// `dirty_price` (price plus accrued interest) and event payments must be in one currency.
/// A yield is `None` when some payment before its horizon is not known yet
/// (e.g. floating coupons); yield to maturity also needs a maturity.
/// Duration is calculated to the next offer, as the exchange does, when its yield is known,
/// to maturity otherwise.
#[must_use]
pub fn bond_info(events: &[BondEvent], dirty_price: Decimal, now: DateTime<Utc>) -> BondInfo {
    let today = now.date_naive();
    let future: Vec<&BondEvent> = events
        .iter()
        .filter(|e| e.date.date_naive() > today)
        .collect();
    let maturity_date = future
        .iter()
        .filter(|e| e.kind == BondEventKind::Maturity)
        .map(|e| e.date)
        .max();
    let next_offer = future
        .iter()
        .filter(|e| e.kind == BondEventKind::Offer)
        .min_by_key(|e| e.date);

    let payments: Vec<&BondEvent> = future
        .iter()
        .copied()
        .filter(|e| e.kind.is_payment())
        .collect();
    let ytm = maturity_date.and_then(|_| yield_of(&payments, dirty_price, now));
    // Redeemed at the offer: payments until it plus the offer price.
    let until_offer: Option<Vec<&BondEvent>> = next_offer.map(|offer| {
        payments
            .iter()
            .copied()
            .filter(|e| e.date <= offer.date)
            .chain(std::iter::once(*offer))
            .collect()
    });
    let yield_to_offer = until_offer
        .as_ref()
        .and_then(|flows| yield_of(flows, dirty_price, now));

    let horizon = until_offer
        .zip(yield_to_offer)
        .or_else(|| ytm.map(|rate| (payments, rate)));
    let duration = horizon
        .as_ref()
        .and_then(|(flows, rate)| macaulay_duration(flows, *rate, now));
    let modified_duration = duration
        .zip(horizon)
        .and_then(|(d, (_, rate))| d.checked_div(Decimal::ONE + rate));

    BondInfo {
        maturity_date,
        next_offer_date: next_offer.map(|e| e.date),
        ytm,
        yield_to_offer,
        duration,
        modified_duration,
    }
}

/// Macaulay duration in years: payment times weighted by payments discounted at `rate`.
fn macaulay_duration(
    payments: &[&BondEvent],
    rate: Decimal,
    now: DateTime<Utc>,
) -> Option<Decimal> {
    let base = Decimal::ONE + rate;
    let (weighted, total) =
        payments
            .iter()
            .try_fold((Decimal::ZERO, Decimal::ZERO), |(weighted, total), e| {
                let years = years_between(now, e.date);
                let present = e.per_bond.value.checked_div(base.checked_powd(years)?)?;
                Some((weighted + years * present, total + present))
            })?;
    weighted.checked_div(total)
}

/// Annual yield of buying at `dirty_price` at `now` and receiving `payments`.
fn yield_of(payments: &[&BondEvent], dirty_price: Decimal, now: DateTime<Utc>) -> Option<Decimal> {
    if dirty_price <= Decimal::ZERO || payments.iter().any(|e| e.per_bond.value.is_zero()) {
        return None;
    }
    let purchase = CashFlow {
        date: now,
        amount: -dirty_price,
    };
    let flows: Vec<CashFlow> = std::iter::once(purchase)
        .chain(payments.iter().map(|e| CashFlow {
            date: e.date,
            amount: e.per_bond.value,
        }))
        .collect();
    xirr(&flows)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use iso_currency::Currency;
    use rust_decimal_macros::dec;

    use super::*;

    fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
    }

    fn event(kind: BondEventKind, date: DateTime<Utc>, per_bond: Decimal) -> BondEvent {
        BondEvent {
            kind,
            date,
            per_bond: Money::from_value(per_bond, Currency::RUB),
        }
    }

    #[test]
    fn mark_amortizations_keeps_latest_as_maturity() {
        // Arrange
        let mut events = [
            event(BondEventKind::Maturity, at(2027, 1, 1), dec!(250)),
            event(BondEventKind::Coupon, at(2027, 6, 1), dec!(30)),
            event(BondEventKind::Maturity, at(2028, 1, 1), dec!(750)),
        ];

        // Act
        mark_amortizations(&mut events);

        // Assert
        let kinds: Vec<BondEventKind> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                BondEventKind::Amortization,
                BondEventKind::Coupon,
                BondEventKind::Maturity
            ]
        );
    }

    #[test]
    fn bond_info_ytm_of_bullet_bond() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Coupon, at(2026, 7, 2), dec!(50)),
            event(BondEventKind::Coupon, at(2027, 1, 1), dec!(50)),
            event(BondEventKind::Maturity, at(2027, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(1000), now);

        // Assert
        assert_eq!(info.maturity_date, Some(at(2027, 1, 1)));
        assert_eq!(info.next_offer_date, None);
        assert_eq!(info.ytm.map(|r| r.round_dp(4)), Some(dec!(0.1025)));
    }

    #[test]
    fn bond_info_ignores_past_events_and_reports_offer() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Coupon, at(2025, 7, 1), dec!(50)),
            event(BondEventKind::Offer, at(2025, 12, 1), dec!(1000)),
            event(BondEventKind::Offer, at(2026, 6, 1), dec!(1000)),
            event(BondEventKind::Maturity, at(2027, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(950), now);

        // Assert
        assert_eq!(info.next_offer_date, Some(at(2026, 6, 1)));
        assert_eq!(info.ytm.map(|r| r.round_dp(4)), Some(dec!(0.0526)));
    }

    #[test]
    fn bond_info_yield_to_offer_with_unknown_later_coupons() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Coupon, at(2026, 7, 2), dec!(50)),
            event(BondEventKind::Offer, at(2027, 1, 1), dec!(1000)),
            event(BondEventKind::Coupon, at(2027, 1, 1), dec!(50)),
            event(BondEventKind::Coupon, at(2027, 7, 1), dec!(0)),
            event(BondEventKind::Maturity, at(2028, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(1000), now);

        // Assert
        assert_eq!(info.ytm, None);
        assert_eq!(
            info.yield_to_offer.map(|r| r.round_dp(4)),
            Some(dec!(0.1025))
        );
    }

    #[test]
    fn bond_info_duration_of_zero_coupon_bond_is_its_term() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [event(BondEventKind::Maturity, at(2027, 1, 1), dec!(1000))];

        // Act
        let info = bond_info(&events, dec!(900), now);

        // Assert
        assert_eq!(info.duration, Some(dec!(1)));
        assert_eq!(
            info.modified_duration.map(|d| d.round_dp(4)),
            Some(dec!(0.9))
        );
    }

    #[test]
    fn bond_info_duration_to_maturity() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Coupon, at(2026, 7, 2), dec!(50)),
            event(BondEventKind::Coupon, at(2027, 1, 1), dec!(50)),
            event(BondEventKind::Maturity, at(2027, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(1000), now);

        // Assert
        assert_eq!(info.duration.map(|d| d.round_dp(4)), Some(dec!(0.9761)));
        assert_eq!(
            info.modified_duration.map(|d| d.round_dp(4)),
            Some(dec!(0.8854))
        );
    }

    #[test]
    fn bond_info_duration_to_offer() {
        // Arrange: coupons after the offer are unknown, so there is no YTM
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Coupon, at(2026, 7, 2), dec!(50)),
            event(BondEventKind::Offer, at(2027, 1, 1), dec!(1000)),
            event(BondEventKind::Coupon, at(2027, 1, 1), dec!(50)),
            event(BondEventKind::Coupon, at(2027, 7, 1), dec!(0)),
            event(BondEventKind::Maturity, at(2028, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(1000), now);

        // Assert
        assert_eq!(info.duration.map(|d| d.round_dp(4)), Some(dec!(0.9761)));
    }

    #[test]
    fn bond_info_prefers_offer_horizon_for_duration() {
        // Arrange
        let now = at(2026, 1, 1);
        let events = [
            event(BondEventKind::Offer, at(2027, 1, 1), dec!(1000)),
            event(BondEventKind::Maturity, at(2030, 1, 1), dec!(1000)),
        ];

        // Act
        let info = bond_info(&events, dec!(900), now);

        // Assert
        assert!(info.ytm.is_some());
        assert_eq!(info.duration, Some(dec!(1)));
    }

    #[rstest::rstest]
    #[case::unknown_floating_coupon(vec![
        event(BondEventKind::Coupon, at(2026, 7, 1), dec!(0)),
        event(BondEventKind::Maturity, at(2027, 1, 1), dec!(1000)),
    ])]
    #[case::no_maturity(vec![event(BondEventKind::Coupon, at(2026, 7, 1), dec!(50))])]
    fn bond_info_without_ytm(#[case] events: Vec<BondEvent>) {
        // Arrange
        let now = at(2026, 1, 1);

        // Act
        let info = bond_info(&events, dec!(1000), now);

        // Assert
        assert_eq!(info.ytm, None);
        assert_eq!(info.duration, None);
        assert_eq!(info.modified_duration, None);
    }
}
