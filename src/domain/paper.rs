use chrono::{DateTime, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;
use std::fmt;

use super::money::{Income, Money};
use super::xirr::{CashFlow, xirr};

/// Newtype for FIGI (Financial Instrument Global Identifier)
/// Provides type safety and prevents mixing up with other string identifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Figi(pub String);

impl Figi {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Figi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Figi {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Figi {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Figi {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Newtype for ticker symbol
/// Provides type safety and prevents mixing up with other string identifiers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ticker(pub String);

impl Ticker {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Ticker {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Ticker {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Ticker {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Kind of a portfolio instrument supported by the reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstrumentKind {
    Share,
    Bond,
    Etf,
    Currency,
    Future,
}

impl InstrumentKind {
    /// Kind of an API instrument type such as `share`; `None` for unsupported ones.
    ///
    /// # Examples
    ///
    /// ```
    /// use tinkoff::domain::InstrumentKind;
    ///
    /// assert_eq!(InstrumentKind::from_api("futures"), Some(InstrumentKind::Future));
    /// assert_eq!(InstrumentKind::from_api("option"), None);
    /// ```
    #[must_use]
    pub fn from_api(instrument_type: &str) -> Option<Self> {
        match instrument_type {
            "share" => Some(Self::Share),
            "bond" => Some(Self::Bond),
            "etf" => Some(Self::Etf),
            "currency" => Some(Self::Currency),
            "futures" => Some(Self::Future),
            _ => None,
        }
    }

    /// Whether the API gives dividends of the instrument: shares and ETFs only.
    #[must_use]
    pub fn pays_dividends(self) -> bool {
        matches!(self, Self::Share | Self::Etf)
    }
}

#[derive(Clone)]
pub struct Instrument {
    pub name: String,
    pub ticker: Ticker,
    /// Currency the instrument is exposed to: the nominal one of bonds and currencies,
    /// the trading one otherwise; `None` when the API returned an unknown code
    pub currency: Option<Currency>,
    /// Asset the instrument belongs to; `None` when unknown
    pub asset_uid: Option<String>,
    /// Economic sector code, e.g. `financial`; `None` when unknown or not applicable
    pub sector: Option<String>,
}

/// Instrument found by its ticker or name.
#[derive(Clone, Debug)]
pub struct FoundInstrument {
    pub figi: String,
    pub ticker: String,
    pub name: String,
}

#[derive(Clone, Copy)]
pub struct Position {
    pub currency: Currency,
    pub average_buy_price: Money,
    pub current_instrument_price: Money,
    /// Accrued coupon interest (NKD) per unit; zero for non-bond instruments
    pub accrued_interest: Money,
    pub quantity: Decimal,
    /// Change of the whole position value since the previous trading day
    pub daily_yield: Money,
    /// Variation margin of a futures contract.
    /// `Some` replaces the contract price in the portfolio value, including zero.
    /// `None` for instruments valued by price times quantity.
    pub var_margin: Option<Money>,
    /// Trading of the instrument is blocked by the exchange
    pub blocked: bool,
    /// Amount reserved by active orders
    pub blocked_lots: Decimal,
}

/// Position of the account portfolio as the API reports it, prices in RUB.
#[derive(Clone)]
pub struct AccountPosition {
    pub figi: String,
    pub instrument_uid: String,
    /// API instrument type, e.g. `share`; see [`InstrumentKind::from_api`]
    pub instrument_type: String,
    /// `Err` with the reason when the API gave incomplete or non-RUB prices
    pub position: Result<Position, String>,
}

impl Position {
    /// Money this position adds to the portfolio.
    ///
    /// A futures contract adds its variation margin. Its price times quantity is the
    /// contract size, which the broker does not include in the portfolio total.
    #[must_use]
    pub fn current(&self) -> Money {
        self.var_margin.unwrap_or_else(|| self.notional())
    }

    /// Current price plus accrued interest multiplied by quantity.
    ///
    /// For a futures contract this is its size, not the money it adds to the portfolio.
    #[must_use]
    pub fn notional(&self) -> Money {
        (self.current_instrument_price + self.accrued_interest) * self.quantity
    }
}

#[derive(Clone)]
pub struct Totals {
    /// Dividends, coupons etc. i.e. some extra value
    /// an asset may earn, net of taxes withheld from it
    pub additional_profit: Money,
    /// Broker, exchange and other fees; taxes are in `additional_profit`
    pub fees: Money,
    /// All payments of the paper's operations in RUB, for XIRR
    pub cash_flows: Vec<CashFlow>,
}

/// Bond specific data
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BondInfo {
    pub maturity_date: Option<DateTime<Utc>>,
    pub next_offer_date: Option<DateTime<Utc>>,
    /// Annual yield to maturity; `None` for unknown future coupons or no maturity
    pub ytm: Option<Decimal>,
    /// Annual yield to the next offer if the bond is redeemed there
    pub yield_to_offer: Option<Decimal>,
    /// Macaulay duration in years: to the next offer when its yield is known,
    /// to maturity otherwise
    pub duration: Option<Decimal>,
    /// Modified duration: relative price change for a unit change of the yield
    pub modified_duration: Option<Decimal>,
}

/// Represents additional asset profit
/// besides balance value growing due to price increase.
/// Used mainly for output
pub trait Profit: Copy + Clone {
    /// shows whether additional profit
    /// applicable to an asset
    fn applicable() -> bool;
    /// Profit name
    fn name() -> &'static str;
}

#[derive(Clone, Copy)]
pub struct DividendProfit;
#[derive(Clone, Copy)]
pub struct CouponProfit;
#[derive(Clone, Copy)]
pub struct NoneProfit;

/// Paper represents things like share, bond, currency, etf etc.
#[derive(Clone)]
pub struct Paper<P: Profit> {
    pub name: String,
    pub ticker: Ticker,
    pub figi: Figi,
    pub position: Position,
    pub totals: Totals,
    pub profit: P,
    /// Set for bonds when their events were loaded
    pub bond: Option<BondInfo>,
    /// Economic sector code from the API, e.g. `financial`; `None` when unknown
    pub sector: Option<String>,
}

impl Profit for DividendProfit {
    fn applicable() -> bool {
        true
    }

    fn name() -> &'static str {
        "Dividends"
    }
}

impl Profit for CouponProfit {
    fn applicable() -> bool {
        true
    }

    fn name() -> &'static str {
        "Coupons"
    }
}

impl Profit for NoneProfit {
    fn applicable() -> bool {
        false
    }

    fn name() -> &'static str {
        ""
    }
}

impl<P: Profit> Paper<P> {
    /// Paper income (difference between current and balance prices)
    #[must_use]
    pub fn income(&self) -> Income {
        Income::new(self.current(), self.balance())
    }

    /// Total income (income + dividends)
    #[must_use]
    pub fn total_income(&self) -> Income {
        let div = self.dividends();
        Income::new(self.current() + (div.current - div.balance), self.balance())
    }

    /// Money paid for the position.
    ///
    /// A futures contract is not paid for at its price, so its balance is zero and its
    /// variation margin is the whole result.
    #[must_use]
    pub fn balance(&self) -> Money {
        if self.position.var_margin.is_some() {
            Money::zero(self.position.current_instrument_price.currency)
        } else {
            self.position.average_buy_price * self.position.quantity
        }
    }

    /// Current position value. Futures contribute their variation margin.
    #[must_use]
    pub fn current(&self) -> Money {
        self.position.current()
    }

    /// Gross exposure: absolute contract size, so a short position or a futures contract
    /// counts as much risk as a long one of the same size.
    ///
    /// This is the price times quantity, not the variation margin a futures contract
    /// adds to the portfolio.
    #[must_use]
    pub fn exposure(&self) -> Decimal {
        self.position.notional().value.abs()
    }

    /// Change since the previous trading day relative to the value at its close
    #[must_use]
    pub fn daily_income(&self) -> Income {
        let current = self.current();
        Income::new(current, current - self.position.daily_yield)
    }

    /// Dividends and coupons
    #[must_use]
    pub fn dividends(&self) -> Income {
        Income::new(
            self.totals.additional_profit + self.balance(),
            self.balance(),
        )
    }

    /// Fees; taxes are deducted from dividends and coupons instead
    #[must_use]
    pub fn fees(&self) -> Income {
        // IMPORTANT: we must add self.totals.fees because their value is negative
        Income::new(self.balance() + self.totals.fees, self.balance())
    }

    #[must_use]
    pub fn currency(&self) -> Currency {
        self.position.currency
    }

    #[must_use]
    pub fn quantity(&self) -> Decimal {
        self.position.quantity
    }

    #[must_use]
    pub fn current_instrument_price(&self) -> Money {
        self.position.current_instrument_price
    }

    #[must_use]
    pub fn accrued_interest(&self) -> Money {
        self.position.accrued_interest
    }

    #[must_use]
    pub fn average_buy_price(&self) -> Money {
        self.position.average_buy_price
    }

    /// Annual return (XIRR) of the paper's payments with its current value received at `at`.
    #[must_use]
    pub fn xirr(&self, at: DateTime<Utc>) -> Option<Decimal> {
        xirr(&self.cash_flows_until(at))
    }

    /// Operation payments followed by the current value as if the paper were sold at `at`.
    #[must_use]
    pub fn cash_flows_until(&self, at: DateTime<Utc>) -> Vec<CashFlow> {
        let mut flows = self.totals.cash_flows.clone();
        flows.push(CashFlow {
            date: at,
            amount: self.current().value,
        });
        flows
    }

    /// Returns the same paper tagged with another additional profit kind.
    #[must_use]
    pub fn with_profit<Q: Profit>(self, profit: Q) -> Paper<Q> {
        Paper {
            name: self.name,
            ticker: self.ticker,
            figi: self.figi,
            position: self.position,
            totals: self.totals,
            profit,
            bond: self.bond,
            sector: self.sector,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest]
    #[case::share("share", Some(InstrumentKind::Share))]
    #[case::bond("bond", Some(InstrumentKind::Bond))]
    #[case::etf("etf", Some(InstrumentKind::Etf))]
    #[case::currency("currency", Some(InstrumentKind::Currency))]
    #[case::futures("futures", Some(InstrumentKind::Future))]
    #[case::option("option", None)]
    #[case::structured_product("sp", None)]
    #[case::empty("", None)]
    fn instrument_kind_from_api(
        #[case] instrument_type: &str,
        #[case] expected: Option<InstrumentKind>,
    ) {
        // Act
        let kind = InstrumentKind::from_api(instrument_type);

        // Assert
        assert_eq!(kind, expected);
    }

    #[rstest]
    #[case::share(InstrumentKind::Share, true)]
    #[case::etf(InstrumentKind::Etf, true)]
    #[case::bond(InstrumentKind::Bond, false)]
    #[case::currency(InstrumentKind::Currency, false)]
    #[case::future(InstrumentKind::Future, false)]
    fn instrument_kind_pays_dividends(#[case] kind: InstrumentKind, #[case] expected: bool) {
        // Act
        let pays = kind.pays_dividends();

        // Assert
        assert_eq!(pays, expected);
    }

    fn bond(accrued_interest: Decimal) -> Paper<CouponProfit> {
        let currency = Currency::RUB;
        Paper {
            name: "Bond".to_string(),
            ticker: Ticker::new("BND"),
            figi: Figi::new("FIGI"),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(1000), currency),
                current_instrument_price: Money::from_value(dec!(990), currency),
                accrued_interest: Money::from_value(accrued_interest, currency),
                quantity: dec!(10),
                daily_yield: Money::zero(currency),
                blocked: false,
                blocked_lots: dec!(0),
                var_margin: None,
            },
            totals: Totals {
                additional_profit: Money::zero(currency),
                fees: Money::zero(currency),
                cash_flows: vec![],
            },
            profit: CouponProfit,
            bond: None,
            sector: None,
        }
    }

    #[test]
    fn xirr_includes_current_value() {
        // Arrange
        let bought = chrono::TimeZone::with_ymd_and_hms(&Utc, 2025, 1, 1, 0, 0, 0).unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 1, 1, 0, 0, 0).unwrap();
        let mut paper = bond(Decimal::ZERO);
        paper.totals.cash_flows = vec![CashFlow {
            date: bought,
            amount: dec!(-9000),
        }];

        // Act
        let rate = paper.xirr(now);

        // Assert
        assert_eq!(rate.map(|r| r.round_dp(4)), Some(dec!(0.1)));
    }

    #[test]
    fn xirr_without_operations_is_undefined() {
        // Arrange
        let paper = bond(Decimal::ZERO);

        // Act
        let rate = paper.xirr(Utc::now());

        // Assert
        assert_eq!(rate, None);
    }

    #[test]
    fn with_profit_keeps_paper_data() {
        // Arrange
        let paper = bond(dec!(15.5)).with_profit(NoneProfit);

        // Act
        let paper = paper.with_profit(CouponProfit);

        // Assert
        assert_eq!(paper.name, "Bond");
        assert_eq!(paper.figi.as_str(), "FIGI");
        assert_eq!(paper.current().value, dec!(10055));
    }

    #[rstest]
    #[case::growth(dec!(55), dec!(10000))]
    #[case::fall(dec!(-45), dec!(10100))]
    #[case::unchanged(dec!(0), dec!(10055))]
    fn daily_income_starts_from_previous_close(
        #[case] daily_yield: Decimal,
        #[case] previous: Decimal,
    ) {
        // Arrange
        let mut paper = bond(dec!(15.5));
        paper.position.daily_yield = Money::from_value(daily_yield, Currency::RUB);

        // Act
        let daily = paper.daily_income();

        // Assert
        assert_eq!(daily.current, dec!(10055));
        assert_eq!(daily.balance, previous);
    }

    #[test]
    fn current_includes_accrued_interest() {
        // Arrange
        let paper = bond(dec!(15.5));

        // Act
        let current = paper.current();

        // Assert
        assert_eq!(current.value, dec!(10055));
    }

    #[test]
    fn current_without_accrued_interest() {
        // Arrange
        let paper = bond(Decimal::ZERO);

        // Act
        let current = paper.current();

        // Assert
        assert_eq!(current.value, dec!(9900));
    }

    #[test]
    fn income_accounts_accrued_interest() {
        // Arrange
        let paper = bond(dec!(15.5));

        // Act
        let income = paper.income();

        // Assert
        assert_eq!(income.current, dec!(10055));
        assert_eq!(income.balance, dec!(10000));
    }

    #[test]
    fn futures_add_variation_margin_and_expose_the_contract_size() {
        // Arrange: contract price 93 250, open margin 1 500, held short
        let mut paper = bond(Decimal::ZERO);
        paper.position.average_buy_price = Money::from_value(dec!(90000), Currency::RUB);
        paper.position.current_instrument_price = Money::from_value(dec!(93250), Currency::RUB);
        paper.position.quantity = dec!(-1);
        paper.position.var_margin = Some(Money::from_value(dec!(1500), Currency::RUB));

        // Act
        let (current, balance, exposure) = (paper.current(), paper.balance(), paper.exposure());
        let terminal = paper.cash_flows_until(Utc::now()).last().map(|f| f.amount);

        // Assert
        assert_eq!(current.value, dec!(1500));
        assert_eq!(balance.value, dec!(0));
        assert_eq!(current.value - balance.value, dec!(1500));
        assert_eq!(exposure, dec!(93250));
        assert_eq!(terminal, Some(dec!(1500)));
    }
}
