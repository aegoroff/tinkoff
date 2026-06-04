use iso_currency::Currency;

use super::money::{Income, Money};
use super::paper::{CouponProfit, DividendProfit, NoneProfit, Paper, Profit};

/// A position loaded from the API, tagged by instrument kind.
pub enum LoadedPaper {
    Bond(Paper<CouponProfit>),
    Share(Paper<DividendProfit>),
    Etf(Paper<NoneProfit>),
    Currency(Paper<NoneProfit>),
    Future(Paper<NoneProfit>),
}

/// Portfolio is an [`Asset`]'s container
/// [`Asset`] is a [`Paper`]'s container
pub struct Portfolio {
    pub bonds: Asset<CouponProfit>,
    pub shares: Asset<DividendProfit>,
    pub etfs: Asset<NoneProfit>,
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
            etfs: Asset::new("Etfs", NoneProfit, output_papers),
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

    pub(crate) fn papers(&self) -> &[Paper<P>] {
        &self.papers
    }

    fn fold<B, IF, F>(&self, mut init: IF, f: F) -> B
    where
        IF: FnMut(Currency) -> B,
        F: FnMut(B, &Paper<P>) -> B,
    {
        let currency = self.currency();
        self.papers.iter().fold(init(currency), f)
    }

    fn currency(&self) -> Currency {
        if self.papers.is_empty() {
            iso_currency::Currency::RUB
        } else {
            self.papers[0].currency()
        }
    }
}

#[cfg(test)]
mod tests {
    use iso_currency::Currency;
    use rstest::{fixture, rstest};
    use rust_decimal_macros::dec;

    use super::*;
    use crate::domain::paper::{CouponProfit, DividendProfit, NoneProfit, Position, Totals};

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

    #[fixture]
    fn test_portfolio() -> Portfolio {
        let currency = Currency::RUB;
        let mut bonds = Asset::new("Bonds", CouponProfit, true);
        bonds.add_paper(Paper {
            name: "1".to_string(),
            ticker: "1t".to_string(),
            figi: "1f".to_string(),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(10), currency),
                current_instrument_price: Money::from_value(dec!(11), currency),
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::from_value(dec!(100), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: CouponProfit,
        });
        let mut shares = Asset::new("Shares", DividendProfit, true);
        shares.add_paper(Paper {
            name: "2".to_string(),
            ticker: "2t".to_string(),
            figi: "2f".to_string(),
            position: Position {
                currency,
                average_buy_price: Money::from_value(dec!(5), currency),
                current_instrument_price: Money::from_value(dec!(6), currency),
                quantity: dec!(100),
            },
            totals: Totals {
                additional_profit: Money::from_value(dec!(50), currency),
                fees: Money::from_value(dec!(10), currency),
            },
            profit: DividendProfit,
        });

        let etfs = Asset::new("Etfs", NoneProfit, true);
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
