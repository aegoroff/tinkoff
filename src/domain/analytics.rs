//! Analyst forecasts and fundamental metrics of shares.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::money::Money;
use super::paper::Ticker;

/// Consensus recommendation of investment houses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recommendation {
    Buy,
    Hold,
    Sell,
}

/// Consensus 12 months forecast of investment houses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forecast {
    /// `None` when the API does not specify it
    pub recommendation: Option<Recommendation>,
    pub current_price: Money,
    pub target_price: Money,
    pub min_target: Money,
    pub max_target: Money,
    /// Forecasts of investment houses the consensus is made of, the latest first
    pub targets: Vec<HouseForecast>,
}

/// Forecast of one investment house.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HouseForecast {
    pub company: String,
    /// `None` when the API does not specify it
    pub recommendation: Option<Recommendation>,
    pub date: Option<DateTime<Utc>>,
    pub target_price: Money,
}

impl Forecast {
    /// Expected price change to the consensus target in percent;
    /// `None` when the current price is unknown.
    #[must_use]
    pub fn upside(&self) -> Option<Decimal> {
        self.upside_to(self.target_price)
    }

    /// Expected price change to `target` in percent; `None` when the current price is unknown.
    #[must_use]
    pub fn upside_to(&self, target: Money) -> Option<Decimal> {
        let current = self.current_price.value;
        if current.is_zero() {
            return None;
        }
        Some((target.value - current) / current * dec!(100))
    }

    /// Number of investment houses that gave a forecast
    #[must_use]
    pub fn analysts(&self) -> usize {
        self.targets.len()
    }
}

/// Fundamental metrics of an asset; `None` for metrics the API does not provide.
///
/// Money amounts are in the reporting currency, flows are for the last 12 months.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fundamentals {
    pub pe: Option<Decimal>,
    pub pb: Option<Decimal>,
    pub ev_to_ebitda: Option<Decimal>,
    pub net_debt_to_ebitda: Option<Decimal>,
    /// Return on equity in percent
    pub roe: Option<Decimal>,
    /// Dividend yield for the last 12 months in percent
    pub dividend_yield: Option<Decimal>,
    pub beta: Option<Decimal>,
    pub market_cap: Option<Money>,
    pub enterprise_value: Option<Money>,
    pub ps: Option<Decimal>,
    pub price_to_fcf: Option<Decimal>,
    pub revenue: Option<Money>,
    pub ebitda: Option<Money>,
    pub net_income: Option<Money>,
    pub free_cash_flow: Option<Money>,
    pub eps: Option<Money>,
    /// Revenue change for a year in percent
    pub revenue_growth: Option<Decimal>,
    /// Net margin in percent
    pub net_margin: Option<Decimal>,
    /// Return on assets in percent
    pub roa: Option<Decimal>,
    pub total_debt: Option<Money>,
    /// Total debt to equity in percent
    pub debt_to_equity: Option<Decimal>,
    pub dividends_per_share: Option<Money>,
    /// Average dividend yield for five years in percent
    pub five_year_dividend_yield: Option<Decimal>,
    /// Share of net income paid as dividends in percent
    pub payout_ratio: Option<Decimal>,
    pub ex_dividend_date: Option<DateTime<Utc>>,
    /// Shares in free circulation in percent
    pub free_float: Option<Decimal>,
    pub low_52_weeks: Option<Money>,
    pub high_52_weeks: Option<Money>,
}

/// Forecast and fundamentals of a portfolio share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareAnalytics {
    pub name: String,
    pub ticker: Ticker,
    pub forecast: Option<Forecast>,
    pub fundamentals: Option<Fundamentals>,
}

/// Analytics of all portfolio shares ordered by ticker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analytics {
    shares: Vec<ShareAnalytics>,
}

impl Analytics {
    #[must_use]
    pub fn new(mut shares: Vec<ShareAnalytics>) -> Self {
        shares.sort_by(|a, b| a.ticker.as_str().cmp(b.ticker.as_str()));
        Self { shares }
    }

    #[must_use]
    pub fn shares(&self) -> &[ShareAnalytics] {
        &self.shares
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::rstest;

    use super::*;

    fn rub(value: Decimal) -> Money {
        Money::from_value(value, Currency::RUB)
    }

    fn forecast(current: Decimal, target: Decimal) -> Forecast {
        Forecast {
            recommendation: Some(Recommendation::Buy),
            current_price: rub(current),
            target_price: rub(target),
            min_target: rub(target),
            max_target: rub(target),
            targets: vec![],
        }
    }

    fn share(ticker: &str) -> ShareAnalytics {
        ShareAnalytics {
            name: ticker.to_string(),
            ticker: Ticker::new(ticker),
            forecast: None,
            fundamentals: None,
        }
    }

    #[rstest]
    #[case::growth(dec!(100), dec!(125), Some(dec!(25)))]
    #[case::fall(dec!(200), dec!(150), Some(dec!(-25)))]
    #[case::unknown_price(dec!(0), dec!(150), None)]
    fn upside_relative_to_current_price(
        #[case] current: Decimal,
        #[case] target: Decimal,
        #[case] expected: Option<Decimal>,
    ) {
        // Arrange
        let forecast = forecast(current, target);

        // Act
        let upside = forecast.upside();

        // Assert
        assert_eq!(upside, expected);
    }

    #[test]
    fn analytics_sorted_by_ticker() {
        // Arrange
        let shares = vec![share("SBER"), share("GAZP"), share("LKOH")];

        // Act
        let analytics = Analytics::new(shares);

        // Assert
        let tickers: Vec<&str> = analytics
            .shares()
            .iter()
            .map(|s| s.ticker.as_str())
            .collect();
        assert_eq!(tickers, ["GAZP", "LKOH", "SBER"]);
    }
}
