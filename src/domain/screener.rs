//! Bond screener: bonds of the market meeting criteria, ranked by yield.
//!
//! The market lists over a thousand bonds and their yields need events, one request
//! per bond. So bonds are filtered by their listing first, then by the exchange yield
//! of their quotes, and only a shortlist gets events to calculate yields and duration.
//! Bonds are ranked by yield to worst, as an offer may be the issuer's call.

use chrono::{DateTime, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::bond::{BondEvent, BondEventKind};
use super::money::Money;
use super::paper::BondInfo;
use super::xirr::years_between;

/// Bonds whose events are loaded per bond shown.
const SHORTLIST_FACTOR: usize = 3;

/// Percentage points our yield may differ from the exchange one, so that a bond
/// near a yield bound is not dropped before its own yield is calculated.
const EXCHANGE_YIELD_TOLERANCE: Decimal = dec!(1);

/// Risk level the broker assigns to a bond; it is not a credit rating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BondRisk {
    Low,
    Moderate,
    High,
}

impl BondRisk {
    /// Parses a CLI risk level name.
    ///
    /// # Errors
    ///
    /// Returns a message listing the expected names for an unknown one.
    pub fn parse(value: &str) -> Result<Self, String> {
        [Self::Low, Self::Moderate, Self::High]
            .into_iter()
            .find(|r| value.eq_ignore_ascii_case(r.name()))
            .ok_or_else(|| format!("unknown risk level '{value}'; expected low, moderate or high"))
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

/// Feature of a bond that matters to an investor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BondFeature {
    /// Coupons follow a rate, so they are not known in advance
    Floating,
    /// No maturity
    Perpetual,
    Subordinated,
    /// The nominal is redeemed in parts
    Amortization,
    /// Only qualified investors can buy it
    ForQualified,
}

impl BondFeature {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Floating => "floating",
            Self::Perpetual => "perpetual",
            Self::Subordinated => "subordinated",
            Self::Amortization => "amortization",
            Self::ForQualified => "qualified",
        }
    }
}

/// Bond as the market lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedBond {
    pub figi: String,
    pub uid: String,
    pub ticker: String,
    pub name: String,
    /// Issuer's sector, e.g. `government` or `municipal`
    pub sector: String,
    /// Current nominal; `None` when the API gave none or one of an unknown currency
    pub nominal: Option<Money>,
    /// Accrued interest per bond in the nominal currency
    pub accrued_interest: Option<Money>,
    pub maturity: Option<DateTime<Utc>>,
    /// Date of the next offer; `None` without one
    pub offer: Option<DateTime<Utc>>,
    pub coupons_per_year: i32,
    pub features: Vec<BondFeature>,
    /// `None` when the broker gives none
    pub risk: Option<BondRisk>,
    /// Whether it can be bought through the API
    pub buyable: bool,
}

impl ListedBond {
    #[must_use]
    pub fn has(&self, feature: BondFeature) -> bool {
        self.features.contains(&feature)
    }

    /// Date the bond is expected to be redeemed: the next offer, or maturity.
    #[must_use]
    pub fn redemption(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.offer.filter(|d| *d > now).or(self.maturity)
    }

    /// Price with accrued interest in the nominal currency, of `price_percent` of the nominal.
    #[must_use]
    pub fn dirty_price(&self, price_percent: Decimal) -> Option<Money> {
        let nominal = self.nominal?;
        let accrued = self
            .accrued_interest
            .filter(|a| a.currency == nominal.currency)
            .map_or(Decimal::ZERO, |a| a.value);
        Some(Money::from_value(
            nominal.value * price_percent / dec!(100) + accrued,
            nominal.currency,
        ))
    }
}

/// Market quote of a bond.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BondQuote {
    /// Last price in percent of the nominal
    pub price_percent: Option<Decimal>,
    /// Yield the exchange calculates, in percent
    pub exchange_yield: Option<Decimal>,
}

/// What bonds the screener looks for. Yields are in percent, horizons in years
/// until the next offer or maturity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenerCriteria {
    pub currency: Currency,
    pub min_yield: Option<Decimal>,
    pub max_yield: Option<Decimal>,
    pub min_years: Option<Decimal>,
    pub max_years: Option<Decimal>,
    pub max_duration: Option<Decimal>,
    pub max_risk: BondRisk,
    /// Whether bonds for qualified investors only are included
    pub qualified: bool,
    /// Issuer's sectors in lower case; any sector when empty
    pub sectors: Vec<String>,
    pub limit: usize,
}

impl ScreenerCriteria {
    /// Whether the listing of `bond` meets the criteria at `now`. Floating coupons and
    /// perpetual bonds are left out, as their yields cannot be calculated.
    #[must_use]
    pub fn admits(&self, bond: &ListedBond, now: DateTime<Utc>) -> bool {
        let risk_ok = match bond.risk {
            Some(risk) => risk <= self.max_risk,
            None => self.max_risk == BondRisk::High,
        };
        let years = bond.redemption(now).map(|d| years_between(now, d));
        bond.buyable
            && !bond.has(BondFeature::Floating)
            && !bond.has(BondFeature::Perpetual)
            && (self.qualified || !bond.has(BondFeature::ForQualified))
            && bond.nominal.is_some_and(|n| n.currency == self.currency)
            && risk_ok
            && (self.sectors.is_empty() || self.sectors.contains(&bond.sector.to_lowercase()))
            && years.is_some_and(|y| {
                y > Decimal::ZERO
                    && self.min_years.is_none_or(|min| y >= min)
                    && self.max_years.is_none_or(|max| y <= max)
            })
    }

    /// Whether a yield in percent is within the bounds widened by `tolerance`.
    fn yield_within(&self, percent: Decimal, tolerance: Decimal) -> bool {
        self.min_yield.is_none_or(|min| percent >= min - tolerance)
            && self.max_yield.is_none_or(|max| percent <= max + tolerance)
    }

    /// Bonds whose events are worth loading: priced, with an exchange yield near the
    /// bounds or none, the highest exchange yield first.
    #[must_use]
    pub fn shortlist(&self, quoted: Vec<(ListedBond, BondQuote)>) -> Vec<(ListedBond, Decimal)> {
        let mut priced: Vec<(ListedBond, Decimal, Option<Decimal>)> = quoted
            .into_iter()
            .filter_map(|(bond, quote)| {
                let price = quote.price_percent.filter(|p| *p > Decimal::ZERO)?;
                let near = quote
                    .exchange_yield
                    .is_none_or(|y| self.yield_within(y, EXCHANGE_YIELD_TOLERANCE));
                near.then_some((bond, price, quote.exchange_yield))
            })
            .collect();
        priced.sort_by_key(|(_, _, y)| (y.is_none(), std::cmp::Reverse(*y)));
        priced
            .into_iter()
            .take(self.limit * SHORTLIST_FACTOR)
            .map(|(bond, price, _)| (bond, price))
            .collect()
    }

    /// Bonds with calculated yields within the bounds and duration within the limit,
    /// the highest yield first, at most [`Self::limit`] of them.
    #[must_use]
    pub fn rank(&self, bonds: Vec<ScreenedBond>) -> Vec<ScreenedBond> {
        let mut admitted: Vec<ScreenedBond> = bonds
            .into_iter()
            .filter(|b| {
                b.yield_rate()
                    .is_some_and(|r| self.yield_within(r * dec!(100), Decimal::ZERO))
                    && self
                        .max_duration
                        .is_none_or(|max| b.info.duration.is_some_and(|d| d <= max))
            })
            .collect();
        admitted.sort_by_key(|b| std::cmp::Reverse(b.yield_rate()));
        admitted.truncate(self.limit);
        admitted
    }
}

/// Bond with its price, yields and duration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenedBond {
    pub bond: ListedBond,
    pub price_percent: Decimal,
    pub info: BondInfo,
    /// Next coupon per bond; `None` when no future coupon is announced
    pub next_coupon: Option<Money>,
}

/// The earliest future coupon of a known, nonzero amount.
#[must_use]
pub fn next_coupon(events: &[BondEvent], now: DateTime<Utc>) -> Option<Money> {
    events
        .iter()
        .filter(|e| e.kind == BondEventKind::Coupon && e.date > now && !e.per_bond.value.is_zero())
        .min_by_key(|e| e.date)
        .map(|e| e.per_bond)
}

/// Date a yield is calculated to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YieldHorizon {
    Offer,
    Maturity,
}

impl ScreenedBond {
    /// Yield to worst as a fraction: the lower of the yields to the next offer and to
    /// maturity that are known. An offer may be the issuer's right to redeem, which it
    /// uses when that is cheaper for it, so the higher yield is not to be counted on.
    #[must_use]
    pub fn yield_to_worst(&self) -> Option<(Decimal, YieldHorizon)> {
        let to_offer = self.info.yield_to_offer.map(|y| (y, YieldHorizon::Offer));
        let to_maturity = self.info.ytm.map(|y| (y, YieldHorizon::Maturity));
        match (to_offer, to_maturity) {
            (Some(offer), Some(maturity)) => Some(if offer.0 <= maturity.0 {
                offer
            } else {
                maturity
            }),
            (known, None) | (None, known) => known,
        }
    }

    /// Annual rate of the next coupon in percent of the current nominal.
    #[must_use]
    pub fn coupon_rate(&self) -> Option<Decimal> {
        let coupon = self.next_coupon?;
        let nominal = self
            .bond
            .nominal
            .filter(|n| n.currency == coupon.currency)?;
        (coupon.value * Decimal::from(self.bond.coupons_per_year) * dec!(100))
            .checked_div(nominal.value)
    }

    /// Yield to worst as a fraction; see [`Self::yield_to_worst`].
    #[must_use]
    pub fn yield_rate(&self) -> Option<Decimal> {
        self.yield_to_worst().map(|(rate, _)| rate)
    }
}

/// Bonds found by the screener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondScreen {
    pub bonds: Vec<ScreenedBond>,
    /// Bonds whose listing meets the criteria
    pub listed: usize,
    /// Bonds whose yields were calculated
    pub checked: usize,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-10-27T09:00:00Z".parse().unwrap()
    }

    fn at(date: &str) -> DateTime<Utc> {
        format!("{date}T00:00:00Z").parse().unwrap()
    }

    fn rub(value: Decimal) -> Money {
        Money::from_value(value, Currency::RUB)
    }

    fn bond(ticker: &str) -> ListedBond {
        ListedBond {
            figi: ticker.to_string(),
            uid: ticker.to_string(),
            ticker: ticker.to_string(),
            name: ticker.to_string(),
            sector: "financial".to_string(),
            nominal: Some(rub(dec!(1000))),
            accrued_interest: Some(rub(dec!(12.5))),
            maturity: Some(at("2029-10-27")),
            offer: None,
            coupons_per_year: 2,
            features: Vec::new(),
            risk: Some(BondRisk::Low),
            buyable: true,
        }
    }

    fn with(bond: ListedBond, feature: BondFeature) -> ListedBond {
        ListedBond {
            features: vec![feature],
            ..bond
        }
    }

    fn criteria() -> ScreenerCriteria {
        ScreenerCriteria {
            currency: Currency::RUB,
            min_yield: None,
            max_yield: None,
            min_years: None,
            max_years: None,
            max_duration: None,
            max_risk: BondRisk::Moderate,
            qualified: false,
            sectors: Vec::new(),
            limit: 2,
        }
    }

    #[rstest]
    #[case::low("LOW", BondRisk::Low)]
    #[case::moderate("moderate", BondRisk::Moderate)]
    #[case::high("High", BondRisk::High)]
    fn bond_risk_parses_name(#[case] value: &str, #[case] expected: BondRisk) {
        // Act
        let risk = BondRisk::parse(value);

        // Assert
        assert_eq!(risk, Ok(expected));
    }

    #[test]
    fn bond_risk_rejects_unknown_name() {
        // Act
        let risk = BondRisk::parse("junk");

        // Assert
        assert!(risk.is_err());
    }

    #[test]
    fn redemption_is_next_offer_before_maturity() {
        // Arrange
        let past_offer = ListedBond {
            offer: Some(at("2026-01-10")),
            ..bond("A")
        };
        let next_offer = ListedBond {
            offer: Some(at("2027-04-10")),
            ..bond("B")
        };

        // Act
        let dates = (past_offer.redemption(now()), next_offer.redemption(now()));

        // Assert
        assert_eq!(dates, (Some(at("2029-10-27")), Some(at("2027-04-10"))));
    }

    #[test]
    fn dirty_price_adds_accrued_interest_to_percent_of_nominal() {
        // Act
        let price = bond("A").dirty_price(dec!(97.5));

        // Assert
        assert_eq!(price, Some(rub(dec!(987.5))));
    }

    #[rstest]
    #[case::plain(bond("A"), true)]
    #[case::floating(with(bond("A"), BondFeature::Floating), false)]
    #[case::perpetual(with(bond("A"), BondFeature::Perpetual), false)]
    #[case::qualified(with(bond("A"), BondFeature::ForQualified), false)]
    #[case::not_buyable(ListedBond { buyable: false, ..bond("A") }, false)]
    #[case::high_risk(ListedBond { risk: Some(BondRisk::High), ..bond("A") }, false)]
    #[case::unknown_risk(ListedBond { risk: None, ..bond("A") }, false)]
    #[case::other_currency(
        ListedBond { nominal: Some(Money::from_value(dec!(1000), Currency::USD)), ..bond("A") },
        false
    )]
    #[case::redeemed(ListedBond { maturity: Some(at("2026-01-01")), ..bond("A") }, false)]
    #[case::without_dates(ListedBond { maturity: None, ..bond("A") }, false)]
    fn criteria_admit_listing(#[case] bond: ListedBond, #[case] expected: bool) {
        // Act
        let admitted = criteria().admits(&bond, now());

        // Assert
        assert_eq!(admitted, expected);
    }

    #[rstest]
    #[case::within(Some(dec!(1)), Some(dec!(5)), true)]
    #[case::too_short(Some(dec!(4)), None, false)]
    #[case::too_long(None, Some(dec!(2)), false)]
    fn criteria_admit_years_to_redemption(
        #[case] min_years: Option<Decimal>,
        #[case] max_years: Option<Decimal>,
        #[case] expected: bool,
    ) {
        // Arrange: three years to maturity
        let criteria = ScreenerCriteria {
            min_years,
            max_years,
            ..criteria()
        };

        // Act
        let admitted = criteria.admits(&bond("A"), now());

        // Assert
        assert_eq!(admitted, expected);
    }

    #[test]
    fn criteria_qualified_and_high_risk_admit_more() {
        // Arrange
        let criteria = ScreenerCriteria {
            qualified: true,
            max_risk: BondRisk::High,
            ..criteria()
        };
        let bond = ListedBond {
            risk: None,
            ..with(bond("A"), BondFeature::ForQualified)
        };

        // Act
        let admitted = criteria.admits(&bond, now());

        // Assert
        assert!(admitted);
    }

    #[rstest]
    #[case::any_sector(&[], "financial", true)]
    #[case::listed_sector(&["government", "municipal"], "Municipal", true)]
    #[case::other_sector(&["government"], "financial", false)]
    fn criteria_admits_sectors(
        #[case] sectors: &[&str],
        #[case] sector: &str,
        #[case] expected: bool,
    ) {
        // Arrange
        let criteria = ScreenerCriteria {
            sectors: sectors.iter().map(ToString::to_string).collect(),
            ..criteria()
        };
        let bond = ListedBond {
            sector: sector.to_string(),
            ..bond("A")
        };

        // Act
        let admitted = criteria.admits(&bond, now());

        // Assert
        assert_eq!(admitted, expected);
    }

    fn event(kind: BondEventKind, date: &str, amount: Decimal) -> BondEvent {
        BondEvent {
            kind,
            date: at(date),
            per_bond: rub(amount),
        }
    }

    #[test]
    fn next_coupon_is_earliest_future_announced_one() {
        // Arrange
        let events = [
            event(BondEventKind::Coupon, "2026-04-27", dec!(30)),
            event(BondEventKind::Coupon, "2027-10-27", dec!(0)),
            event(BondEventKind::Coupon, "2027-04-27", dec!(35.4)),
            event(BondEventKind::Maturity, "2027-01-27", dec!(1000)),
            event(BondEventKind::Coupon, "2028-04-27", dec!(40)),
        ];

        // Act
        let coupon = next_coupon(&events, now());

        // Assert
        assert_eq!(coupon, Some(rub(dec!(35.4))));
    }

    #[test]
    fn next_coupon_is_none_without_announced_ones() {
        // Arrange
        let events = [event(BondEventKind::Coupon, "2027-04-27", dec!(0))];

        // Act
        let coupon = next_coupon(&events, now());

        // Assert
        assert_eq!(coupon, None);
    }

    #[rstest]
    #[case::announced(Some(rub(dec!(35.4))), Some(dec!(7.08)))]
    #[case::unknown(None, None)]
    #[case::other_currency(Some(Money::from_value(dec!(35.4), Currency::USD)), None)]
    fn coupon_rate_is_annual_percent_of_nominal(
        #[case] coupon: Option<Money>,
        #[case] expected: Option<Decimal>,
    ) {
        // Arrange
        let bond = ScreenedBond {
            next_coupon: coupon,
            ..screened("A", None, None, dec!(1))
        };

        // Act
        let rate = bond.coupon_rate();

        // Assert
        assert_eq!(rate, expected);
    }

    fn quote(price: Option<Decimal>, exchange_yield: Option<Decimal>) -> BondQuote {
        BondQuote {
            price_percent: price,
            exchange_yield,
        }
    }

    #[test]
    fn shortlist_takes_priced_near_bounds_highest_exchange_yield_first() {
        // Arrange
        let criteria = ScreenerCriteria {
            min_yield: Some(dec!(15)),
            max_yield: Some(dec!(25)),
            limit: 1,
            ..criteria()
        };
        let quoted = vec![
            (bond("NO_PRICE"), quote(None, Some(dec!(20)))),
            (bond("NO_YIELD"), quote(Some(dec!(99)), None)),
            (bond("LOW"), quote(Some(dec!(99)), Some(dec!(13)))),
            (bond("NEAR"), quote(Some(dec!(98)), Some(dec!(14.5)))),
            (bond("HIGH"), quote(Some(dec!(80)), Some(dec!(30)))),
            (bond("BEST"), quote(Some(dec!(95)), Some(dec!(24)))),
        ];

        // Act
        let shortlist = criteria.shortlist(quoted);

        // Assert
        let tickers: Vec<&str> = shortlist.iter().map(|(b, _)| b.ticker.as_str()).collect();
        assert_eq!(tickers, ["BEST", "NEAR", "NO_YIELD"]);
        assert_eq!(shortlist[0].1, dec!(95));
    }

    fn screened(
        ticker: &str,
        ytm: Option<Decimal>,
        yto: Option<Decimal>,
        duration: Decimal,
    ) -> ScreenedBond {
        ScreenedBond {
            bond: bond(ticker),
            price_percent: dec!(100),
            info: BondInfo {
                maturity_date: Some(at("2029-10-27")),
                next_offer_date: None,
                ytm,
                yield_to_offer: yto,
                duration: Some(duration),
                modified_duration: None,
            },
            next_coupon: None,
        }
    }

    #[test]
    fn rank_filters_by_own_yield_and_duration_highest_first() {
        // Arrange
        let criteria = ScreenerCriteria {
            min_yield: Some(dec!(15)),
            max_duration: Some(dec!(3)),
            limit: 2,
            ..criteria()
        };
        let bonds = vec![
            screened("LOW", Some(dec!(0.14)), None, dec!(2)),
            screened("OFFER", None, Some(dec!(0.19)), dec!(1)),
            screened("WORST", Some(dec!(0.12)), Some(dec!(0.30)), dec!(1)),
            screened("LONG", Some(dec!(0.25)), None, dec!(4)),
            screened("MID", Some(dec!(0.17)), None, dec!(2.5)),
            screened("TOP", Some(dec!(0.21)), None, dec!(2.9)),
            screened("UNKNOWN", None, None, dec!(1)),
        ];

        // Act
        let ranked = criteria.rank(bonds);

        // Assert
        let tickers: Vec<&str> = ranked.iter().map(|b| b.bond.ticker.as_str()).collect();
        assert_eq!(tickers, ["TOP", "OFFER"]);
    }

    #[rstest]
    #[case::offer_lower(Some(dec!(0.20)), Some(dec!(0.15)), Some((dec!(0.15), YieldHorizon::Offer)))]
    #[case::maturity_lower(Some(dec!(0.18)), Some(dec!(0.60)), Some((dec!(0.18), YieldHorizon::Maturity)))]
    #[case::only_offer(None, Some(dec!(0.21)), Some((dec!(0.21), YieldHorizon::Offer)))]
    #[case::only_maturity(Some(dec!(0.17)), None, Some((dec!(0.17), YieldHorizon::Maturity)))]
    #[case::none(None, None, None)]
    fn yield_to_worst_is_lower_known_yield(
        #[case] ytm: Option<Decimal>,
        #[case] yto: Option<Decimal>,
        #[case] expected: Option<(Decimal, YieldHorizon)>,
    ) {
        // Arrange
        let bond = screened("A", ytm, yto, dec!(1));

        // Act
        let worst = bond.yield_to_worst();

        // Assert
        assert_eq!(worst, expected);
    }
}
