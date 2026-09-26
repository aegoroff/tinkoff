use std::fmt::Display;

use chrono::{DateTime, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;

use super::bond::BondEventKind;
use super::money::Money;
use super::paper::{Figi, Ticker};

/// Dividend payment information
#[derive(Clone)]
pub struct DividendPayment {
    pub figi: Figi,
    pub ticker: Ticker,
    pub name: String,
    pub currency: Currency,
    pub dividend_per_share: Money,
    pub total_dividend: Money,
    pub quantity: Decimal,
    pub ex_dividend_date: DateTime<Utc>,
    pub payment_date: Option<DateTime<Utc>>,
    pub dividend_type: String,
}

/// Dividend calendar with upcoming payments
pub struct DividendCalendar {
    pub upcoming: Vec<DividendPayment>,
}

/// Coupon payment information
#[derive(Clone)]
pub struct CouponPayment {
    pub figi: Figi,
    pub ticker: Ticker,
    pub name: String,
    pub currency: Currency,
    pub coupon_per_bond: Money,
    pub total_coupon: Money,
    pub quantity: Decimal,
    pub coupon_date: DateTime<Utc>,
    /// Coupon, amortization or maturity
    pub kind: BondEventKind,
}

/// Coupon calendar with upcoming payments
pub struct CouponCalendar {
    pub upcoming: Vec<CouponPayment>,
}

/// Trait for calendar payment items (dividends, coupons, etc.)
pub trait CalendarPayment: Clone {
    /// Get the payment date for grouping (used for sorting in calendar)
    fn payment_date(&self) -> DateTime<Utc>;

    /// Get the ex-date / coupon date for display
    fn ex_date(&self) -> DateTime<Utc>;

    /// Get the instrument name
    fn name(&self) -> &str;

    /// Text of the name column: the instrument name, for non-regular payments with their kind
    fn title(&self) -> String {
        self.name().to_string()
    }

    /// Whether the payment returns the bond nominal (amortization or maturity) instead of income
    fn is_redemption(&self) -> bool {
        false
    }

    /// Get the payment amount per unit (dividend per share, coupon per bond)
    fn payment_per_unit(&self) -> Money;

    /// Get the total payment amount
    fn total_payment(&self) -> Money;

    /// Get the calendar title (e.g., "Dividend Calendar", "Coupon Calendar")
    fn calendar_title() -> &'static str;

    /// Get the column headers for the table
    fn column_headers() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    );

    /// Get the empty message
    #[must_use]
    fn empty_message() -> &'static str {
        "No upcoming payments"
    }

    /// Get month label
    #[must_use]
    fn month_label(month_name: &str) -> String {
        format!("Month {month_name} Total:")
    }

    /// Get year label
    #[must_use]
    fn year_label(year: i32) -> String {
        format!("Year {year} Total:")
    }
}

impl CalendarPayment for DividendPayment {
    fn payment_date(&self) -> DateTime<Utc> {
        self.payment_date.unwrap_or(self.ex_dividend_date)
    }

    fn ex_date(&self) -> DateTime<Utc> {
        self.ex_dividend_date
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn payment_per_unit(&self) -> Money {
        self.dividend_per_share
    }

    fn total_payment(&self) -> Money {
        self.total_dividend
    }

    fn calendar_title() -> &'static str {
        "Dividend Calendar"
    }

    fn column_headers() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "Payment Date",
            "Ex-Dividend Date",
            "Company",
            "Dividend per Share",
            "Total Dividend",
        )
    }
}

impl CalendarPayment for CouponPayment {
    fn payment_date(&self) -> DateTime<Utc> {
        self.coupon_date
    }

    fn ex_date(&self) -> DateTime<Utc> {
        self.coupon_date
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_redemption(&self) -> bool {
        matches!(
            self.kind,
            BondEventKind::Amortization | BondEventKind::Maturity
        )
    }

    fn title(&self) -> String {
        match self.kind {
            BondEventKind::Coupon => self.name.clone(),
            kind => format!("{} · {}", self.name, kind.label()),
        }
    }

    fn payment_per_unit(&self) -> Money {
        self.coupon_per_bond
    }

    fn total_payment(&self) -> Money {
        self.total_coupon
    }

    fn calendar_title() -> &'static str {
        "Bond Payments Calendar"
    }

    fn column_headers() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "Payment Date",
            "Coupon Date",
            "Company",
            "Payment per Bond",
            "Total Payment",
        )
    }
}

impl Display for CouponPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} | {} | {})",
            self.name,
            self.ticker,
            self.figi,
            self.currency.code()
        )
    }
}

impl Display for DividendPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} | {} | {})",
            self.name,
            self.ticker,
            self.figi,
            self.currency.code()
        )
    }
}

/// Combined payment type for merged dividend and coupon calendar
#[derive(Clone)]
pub enum CombinedPayment {
    Dividend(DividendPayment),
    Coupon(CouponPayment),
}

/// Combined calendar with both dividend and coupon payments
pub struct CombinedCalendar {
    pub upcoming: Vec<CombinedPayment>,
}

impl CombinedCalendar {
    /// Merges dividend and coupon calendars into one sorted by payment date.
    #[must_use]
    pub fn merge(dividends: DividendCalendar, coupons: CouponCalendar) -> Self {
        let mut upcoming: Vec<CombinedPayment> = dividends
            .upcoming
            .into_iter()
            .map(CombinedPayment::Dividend)
            .chain(coupons.upcoming.into_iter().map(CombinedPayment::Coupon))
            .collect();
        upcoming.sort_by_key(CalendarPayment::payment_date);
        Self { upcoming }
    }
}

/// Which payments a calendar includes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarKind {
    Dividends,
    Coupons,
    Combined,
}

/// Calendar of upcoming payments of the requested [`CalendarKind`]
pub enum Calendar {
    Dividends(DividendCalendar),
    Coupons(CouponCalendar),
    Combined(CombinedCalendar),
}

impl CalendarPayment for CombinedPayment {
    fn payment_date(&self) -> DateTime<Utc> {
        match self {
            Self::Dividend(d) => d.payment_date(),
            Self::Coupon(c) => c.payment_date(),
        }
    }

    fn ex_date(&self) -> DateTime<Utc> {
        match self {
            Self::Dividend(d) => d.ex_date(),
            Self::Coupon(c) => c.ex_date(),
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Dividend(d) => d.name(),
            Self::Coupon(c) => c.name(),
        }
    }

    fn is_redemption(&self) -> bool {
        match self {
            Self::Dividend(d) => d.is_redemption(),
            Self::Coupon(c) => c.is_redemption(),
        }
    }

    fn title(&self) -> String {
        match self {
            Self::Dividend(d) => d.title(),
            Self::Coupon(c) => c.title(),
        }
    }

    fn payment_per_unit(&self) -> Money {
        match self {
            Self::Dividend(d) => d.payment_per_unit(),
            Self::Coupon(c) => c.payment_per_unit(),
        }
    }

    fn total_payment(&self) -> Money {
        match self {
            Self::Dividend(d) => d.total_payment(),
            Self::Coupon(c) => c.total_payment(),
        }
    }

    fn calendar_title() -> &'static str {
        "Payments Calendar"
    }

    fn column_headers() -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        (
            "Payment Date",
            "Ex-Date",
            "Company",
            "Payment per Unit",
            "Total Payment",
        )
    }

    fn empty_message() -> &'static str {
        "No upcoming dividend or coupon payments"
    }
}

impl Display for CombinedPayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dividend(d) => write!(
                f,
                "{} ({} | {} | {})",
                d.name,
                d.ticker,
                d.figi,
                d.currency.code()
            ),
            Self::Coupon(c) => write!(
                f,
                "{} ({} | {} | {})",
                c.name,
                c.ticker,
                c.figi,
                c.currency.code()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    use super::*;

    fn dividend(name: &str, day: u32) -> DividendPayment {
        let date = Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap();
        DividendPayment {
            figi: Figi::new(name),
            ticker: Ticker::new(name),
            name: name.to_string(),
            currency: Currency::RUB,
            dividend_per_share: Money::from_value(dec!(1), Currency::RUB),
            total_dividend: Money::from_value(dec!(10), Currency::RUB),
            quantity: dec!(10),
            ex_dividend_date: date,
            payment_date: Some(date),
            dividend_type: String::new(),
        }
    }

    fn coupon(name: &str, day: u32) -> CouponPayment {
        CouponPayment {
            figi: Figi::new(name),
            ticker: Ticker::new(name),
            name: name.to_string(),
            currency: Currency::RUB,
            coupon_per_bond: Money::from_value(dec!(2), Currency::RUB),
            total_coupon: Money::from_value(dec!(20), Currency::RUB),
            quantity: dec!(10),
            coupon_date: Utc.with_ymd_and_hms(2026, 10, day, 0, 0, 0).unwrap(),
            kind: BondEventKind::Coupon,
        }
    }

    #[test]
    fn merge_sorts_payments_by_date() {
        // Arrange
        let dividends = DividendCalendar {
            upcoming: vec![dividend("D20", 20), dividend("D5", 5)],
        };
        let coupons = CouponCalendar {
            upcoming: vec![coupon("C10", 10)],
        };

        // Act
        let combined = CombinedCalendar::merge(dividends, coupons);

        // Assert
        let names: Vec<&str> = combined
            .upcoming
            .iter()
            .map(CalendarPayment::name)
            .collect();
        assert_eq!(names, ["D5", "C10", "D20"]);
    }

    #[test]
    fn merge_empty_calendars() {
        // Arrange
        let dividends = DividendCalendar { upcoming: vec![] };
        let coupons = CouponCalendar { upcoming: vec![] };

        // Act
        let combined = CombinedCalendar::merge(dividends, coupons);

        // Assert
        assert!(combined.upcoming.is_empty());
    }
}
