use std::fmt::Display;

use chrono::{DateTime, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;

use super::money::Money;

/// Dividend payment information
#[derive(Clone)]
pub struct DividendPayment {
    pub figi: String,
    pub ticker: String,
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
    pub figi: String,
    pub ticker: String,
    pub name: String,
    pub currency: Currency,
    pub coupon_per_bond: Money,
    pub total_coupon: Money,
    pub quantity: Decimal,
    pub coupon_date: DateTime<Utc>,
    pub coupon_type: String,
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

    fn payment_per_unit(&self) -> Money {
        self.coupon_per_bond
    }

    fn total_payment(&self) -> Money {
        self.total_coupon
    }

    fn calendar_title() -> &'static str {
        "Coupon Calendar"
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
            "Coupon per Bond",
            "Total Coupon",
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
