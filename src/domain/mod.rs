pub mod calendar;
pub mod display;
pub mod history;
pub mod money;
pub mod paper;
pub mod portfolio;

pub use calendar::{
    CalendarPayment, CouponCalendar, CouponPayment, DividendCalendar, DividendPayment,
};
pub use history::{History, HistoryItem};
pub use money::{Income, Money};
pub use paper::{
    CouponProfit, DividentProfit, Instrument, NoneProfit, Paper, Position, Profit, Totals,
};
pub use portfolio::{Asset, LoadedPaper, Portfolio};

/// Numeric value that can be classified as negative, zero, or positive (for table coloring).
pub trait NumberRange {
    fn is_negative(&self) -> bool;
    fn is_zero(&self) -> bool;
}
