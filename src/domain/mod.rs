pub mod calendar;
pub mod display;
pub mod history;
pub mod money;
pub mod paper;
pub mod portfolio;
pub mod risk;

pub use calendar::{
    CalendarPayment, CouponCalendar, CouponPayment, DividendCalendar, DividendPayment,
};
pub use history::{History, HistoryItem};
pub use money::{Income, Money};
pub use paper::{
    CouponProfit, DividendProfit, Instrument, NoneProfit, Paper, Position, Profit, Totals,
};
pub use portfolio::{Asset, LoadedPaper, Portfolio};

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
