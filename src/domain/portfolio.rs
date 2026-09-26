use chrono::{DateTime, Utc};
use iso_currency::Currency;
use rust_decimal::Decimal;

use super::money::{Income, Money};
use super::paper::{CouponProfit, DividendProfit, NoneProfit, Paper, Profit};
use super::xirr::{CashFlow, xirr};

/// A position loaded from the API, tagged by instrument kind.
pub enum LoadedPaper {
    Bond(Paper<CouponProfit>),
    Share(Paper<DividendProfit>),
    Etf(Paper<DividendProfit>),
    Currency(Paper<NoneProfit>),
    Future(Paper<NoneProfit>),
}

impl LoadedPaper {
    /// Current market value and nominal (instrument) currency.
    #[must_use]
    pub fn current_value_and_nominal_currency(&self) -> (rust_decimal::Decimal, Currency) {
        match self {
            Self::Bond(p) => (p.current().value, p.currency()),
            Self::Share(p) | Self::Etf(p) => (p.current().value, p.currency()),
            Self::Currency(p) | Self::Future(p) => (p.current().value, p.currency()),
        }
    }
}

/// Portfolio is an [`Asset`]'s container
/// [`Asset`] is a [`Paper`]'s container
pub struct Portfolio {
    pub bonds: Asset<CouponProfit>,
    pub shares: Asset<DividendProfit>,
    pub etfs: Asset<DividendProfit>,
    pub currencies: Asset<NoneProfit>,
    pub futures: Asset<NoneProfit>,
}

/// Asset is a [`Paper`]'s container
pub struct Asset<P: Profit> {
    pub(crate) name: &'static str,
    papers: Vec<Paper<P>>,
    pub profit: P,
    /// Whether to include asset's papers into output
    /// If true papers will be displyed
    /// If false they only accounted during calculations (balance, income etc,)
    pub(crate) output_papers: bool,
}

/// Macro to generate Portfolio aggregation methods
macro_rules! impl_portfolio_aggregator {
    ($method:ident, $asset_method:ident, $return_type:ty, $zero:expr) => {
        #[must_use]
        pub fn $method(&self) -> $return_type {
            self.assets()
                .iter()
                .map(|a| a.$asset_method())
                .fold($zero, |acc, x| acc + x)
        }
    };
}

impl Portfolio {
    pub fn add_loaded_paper(&mut self, paper: LoadedPaper) {
        match paper {
            LoadedPaper::Bond(p) => self.bonds.add_paper(p),
            LoadedPaper::Share(p) => self.shares.add_paper(p),
            LoadedPaper::Etf(p) => self.etfs.add_paper(p),
            LoadedPaper::Currency(p) => self.currencies.add_paper(p),
            LoadedPaper::Future(p) => self.futures.add_paper(p),
        }
    }

    #[must_use]
    pub fn new(output_papers: bool) -> Self {
        Self {
            bonds: Asset::new("Bonds", CouponProfit, output_papers),
            shares: Asset::new("Shares", DividendProfit, output_papers),
            etfs: Asset::new("Etfs", DividendProfit, output_papers),
            currencies: Asset::new("Currencies", NoneProfit, output_papers),
            futures: Asset::new("Futures", NoneProfit, output_papers),
        }
    }

    /// Returns a slice of all assets
    #[must_use]
    fn assets(&self) -> [&dyn PortfolioAsset; 5] {
        [
            &self.bonds as &dyn PortfolioAsset,
            &self.shares as &dyn PortfolioAsset,
            &self.etfs as &dyn PortfolioAsset,
            &self.currencies as &dyn PortfolioAsset,
            &self.futures as &dyn PortfolioAsset,
        ]
    }

    impl_portfolio_aggregator!(income, income, Income, Income::zero(Currency::RUB));
    impl_portfolio_aggregator!(
        total_income,
        total_income,
        Income,
        Income::zero(Currency::RUB)
    );
    impl_portfolio_aggregator!(balance, balance, Money, Money::zero(Currency::RUB));
    impl_portfolio_aggregator!(current, current, Money, Money::zero(Currency::RUB));
    impl_portfolio_aggregator!(dividends, dividends, Money, Money::zero(Currency::RUB));

    /// Annual return (XIRR) of all papers as if the portfolio were sold at `at`.
    #[must_use]
    pub fn xirr(&self, at: DateTime<Utc>) -> Option<Decimal> {
        let flows: Vec<CashFlow> = [
            self.bonds.cash_flows_until(at),
            self.shares.cash_flows_until(at),
            self.etfs.cash_flows_until(at),
            self.currencies.cash_flows_until(at),
            self.futures.cash_flows_until(at),
        ]
        .concat();
        xirr(&flows)
    }

    /// Iterates over copies of all papers in the portfolio tagged by instrument kind.
    pub fn papers(&self) -> impl Iterator<Item = LoadedPaper> + '_ {
        let bonds = self.bonds.papers().iter().cloned().map(LoadedPaper::Bond);
        let shares = self.shares.papers().iter().cloned().map(LoadedPaper::Share);
        let etfs = self.etfs.papers().iter().cloned().map(LoadedPaper::Etf);
        let currencies = self
            .currencies
            .papers()
            .iter()
            .cloned()
            .map(LoadedPaper::Currency);
        let futures = self
            .futures
            .papers()
            .iter()
            .cloned()
            .map(LoadedPaper::Future);
        bonds
            .chain(shares)
            .chain(etfs)
            .chain(currencies)
            .chain(futures)
    }

    #[must_use]
    pub fn count_not_empty_assets(&self) -> usize {
        self.assets().iter().filter(|a| !a.is_asset_empty()).count()
    }
}

/// Trait for portfolio assets to enable iteration
trait PortfolioAsset {
    fn income(&self) -> Income;
    fn total_income(&self) -> Income;
    fn balance(&self) -> Money;
    fn current(&self) -> Money;
    fn dividends(&self) -> Money;
    fn is_asset_empty(&self) -> bool;
}

impl<P: Profit> PortfolioAsset for Asset<P> {
    fn income(&self) -> Income {
        Asset::income(self)
    }

    fn total_income(&self) -> Income {
        Asset::total_income(self)
    }

    fn balance(&self) -> Money {
        Asset::balance(self)
    }

    fn current(&self) -> Money {
        Asset::current(self)
    }

    fn dividends(&self) -> Money {
        Asset::dividends(self)
    }

    fn is_asset_empty(&self) -> bool {
        Asset::is_empty(self)
    }
}

impl Default for Portfolio {
    fn default() -> Self {
        Self::new(true)
    }
}

impl<P: Profit> Asset<P> {
    #[must_use]
    pub fn new(name: &'static str, profit: P, output_papers: bool) -> Self {
        Self {
            papers: vec![],
            name,
            output_papers,
            profit,
        }
    }

    pub fn add_paper(&mut self, paper: Paper<P>) {
        self.papers.push(paper);
    }

    pub fn income(&self) -> Income {
        self.fold(Income::zero, |mut acc, p| {
            acc += p.income();
            acc
        })
    }

    pub fn total_income(&self) -> Income {
        self.fold(Income::zero, |mut acc, p| {
            acc += p.total_income();
            acc
        })
    }

    pub fn current(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc += p.current();
            acc
        })
    }

    pub fn balance(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            acc += p.balance();
            acc
        })
    }

    pub fn dividends(&self) -> Money {
        self.fold(Money::zero, |mut acc, p| {
            // IMPORTANT: We need absolute dividend value here but current is absolute + balance
            // so we have to subtract
            acc += p.dividends().current - p.dividends().balance;
            acc
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.papers.is_empty()
    }

    /// Payments of all papers followed by their current values received at `at`.
    #[must_use]
    pub fn cash_flows_until(&self, at: DateTime<Utc>) -> Vec<CashFlow> {
        self.papers
            .iter()
            .flat_map(|p| p.cash_flows_until(at))
            .collect()
    }

    /// Annual return (XIRR) of the asset as if all its papers were sold at `at`.
    #[must_use]
    pub fn xirr(&self, at: DateTime<Utc>) -> Option<Decimal> {
        xirr(&self.cash_flows_until(at))
    }

    #[must_use]
    pub fn papers(&self) -> &[Paper<P>] {
        &self.papers
    }

    fn fold<B, IF, F>(&self, mut init: IF, f: F) -> B
    where
        IF: FnMut(Currency) -> B,
        F: FnMut(B, &Paper<P>) -> B,
    {
        // Settlement currency for aggregates is always RUB after FX conversion.
        // Position.currency stays as the instrument nominal for risk allocation.
        let currency = self
            .papers
            .first()
            .map_or(Currency::RUB, |p| p.average_buy_price().currency);
        self.papers.iter().fold(init(currency), f)
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::{fixture, rstest};
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::paper::{
        CouponProfit, DividendProfit, Figi, NoneProfit, Position, Ticker, Totals,
    };

    #[rstest]
    fn portfolio_balance(test_portfolio: Portfolio) {
        assert_eq!(dec!(1500), test_portfolio.balance().value);
    }

    #[rstest]
    fn portfolio_current(test_portfolio: Portfolio) {
        assert_eq!(dec!(1700), test_portfolio.current().value);
    }

    #[rstest]
    fn portfolio_income(test_portfolio: Portfolio) {
        assert_eq!(dec!(1500), test_portfolio.income().balance);
        assert_eq!(dec!(1700), test_portfolio.income().current);
        assert_eq!(dec!(13.33), test_portfolio.income().percent().round_dp(2));
    }

    #[rstest]
    fn portfolio_dividends(test_portfolio: Portfolio) {
        assert_eq!(dec!(150), test_portfolio.dividends().value);
    }

    #[rstest]
    fn portfolio_total_income(test_portfolio: Portfolio) {
        assert_eq!(dec!(1850), test_portfolio.total_income().current);
    }

    #[rstest]
    fn portfolio_papers_includes_all_assets(test_portfolio: Portfolio) {
        // Arrange

        // Act
        let papers: Vec<LoadedPaper> = test_portfolio.papers().collect();

        // Assert
        assert_eq!(papers.len(), 2);
        assert!(matches!(&papers[0], LoadedPaper::Bond(p) if p.name == "1"));
        assert!(matches!(&papers[1], LoadedPaper::Share(p) if p.name == "2"));
    }

    #[test]
    fn empty_portfolio_has_no_papers() {
        // Arrange
        let portfolio = Portfolio::new(true);

        // Act
        let count = portfolio.papers().count();

        // Assert
        assert_eq!(count, 0);
    }

    #[rstest]
    fn portfolio_xirr_combines_all_papers(mut test_portfolio: Portfolio) {
        // Arrange
        let bought = chrono::TimeZone::with_ymd_and_hms(&Utc, 2025, 1, 1, 0, 0, 0).unwrap();
        let now = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 1, 1, 0, 0, 0).unwrap();
        // Current values are 1100 and 600: 1000 and 500 invested a year before give 13.33%.
        test_portfolio.bonds.papers[0].totals.cash_flows = vec![CashFlow {
            date: bought,
            amount: dec!(-1000),
        }];
        test_portfolio.shares.papers[0].totals.cash_flows = vec![CashFlow {
            date: bought,
            amount: dec!(-500),
        }];

        // Act
        let rate = test_portfolio.xirr(now);

        // Assert
        assert_eq!(rate.map(|r| r.round_dp(4)), Some(dec!(0.1333)));
    }

    #[fixture]
    fn test_portfolio() -> Portfolio {
        let currency = Currency::RUB;
        let mut bonds = Asset::new("Bonds", CouponProfit, true);
        bonds.add_paper(Paper {
            name: "1".to_string(),
            ticker: Ticker::new("1t".to_string()),
            figi: Figi::new("1f".to_string()),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(10), currency),
                current_instrument_price: Money::from_value(dec!(11), currency),
                accrued_interest: Money::zero(currency),
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::from_value(dec!(100), currency),
                fees: Money::from_value(dec!(10), currency),
                cash_flows: vec![],
            },
            profit: CouponProfit,
            bond: None,
        });
        let mut shares = Asset::new("Shares", DividendProfit, true);
        shares.add_paper(Paper {
            name: "2".to_string(),
            ticker: Ticker::new("2t".to_string()),
            figi: Figi::new("2f".to_string()),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(5), currency),
                current_instrument_price: Money::from_value(dec!(6), currency),
                accrued_interest: Money::zero(currency),
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::from_value(dec!(50), currency),
                fees: Money::from_value(dec!(10), currency),
                cash_flows: vec![],
            },
            profit: DividendProfit,
            bond: None,
        });

        let etfs = Asset::new("Etfs", DividendProfit, true);
        let currencies = Asset::new("Currencies", NoneProfit, true);
        let futures = Asset::new("Futures", NoneProfit, true);
        Portfolio {
            bonds,
            shares,
            etfs,
            currencies,
            futures,
        }
    }
}
