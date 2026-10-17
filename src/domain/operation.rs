use chrono::{DateTime, Utc};

use super::{InstrumentKind, Money, tax::TaxOperationKind};

/// What an operation means for the reports; API operation types are grouped into these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Buy,
    Sell,
    Dividend,
    Coupon,
    DividendTax,
    /// Tax on coupons and its corrections
    CouponTax,
    /// Taxes other than on dividends and coupons and their corrections
    OtherTax,
    /// Income of overnight placement of free cash
    Overnight,
    /// Broker, exchange and other fees and penalties
    Fee,
    Other,
}

impl OperationKind {
    /// Kinds of operations that affect taxes: income and taxes on it.
    pub const TAXED: [Self; 5] = [
        Self::Dividend,
        Self::Coupon,
        Self::DividendTax,
        Self::CouponTax,
        Self::OtherTax,
    ];

    /// Whether the operation is income of a paper or a tax withheld from it,
    /// so that paper income is net of taxes.
    #[must_use]
    pub fn is_income(self) -> bool {
        self.tax_kind().is_some() || self == Self::Overnight
    }

    #[must_use]
    pub fn is_fee(self) -> bool {
        self == Self::Fee
    }

    /// Tax kind of the operation; `None` for operations that do not affect taxes.
    #[must_use]
    pub fn tax_kind(self) -> Option<TaxOperationKind> {
        match self {
            Self::Dividend => Some(TaxOperationKind::Dividend),
            Self::Coupon => Some(TaxOperationKind::Coupon),
            Self::DividendTax => Some(TaxOperationKind::DividendTax),
            Self::CouponTax => Some(TaxOperationKind::CouponTax),
            Self::OtherTax => Some(TaxOperationKind::OtherTax),
            Self::Buy | Self::Sell | Self::Overnight | Self::Fee | Self::Other => None,
        }
    }
}

/// Execution state of an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationState {
    Executed,
    Canceled,
    InProgress,
    Unspecified,
}

impl OperationState {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Executed => "Executed",
            Self::Canceled => "Canceled",
            Self::InProgress => "In progress",
            Self::Unspecified => "Not specified",
        }
    }
}

/// Operation of an account as the API reports it.
#[derive(Clone, Debug)]
pub struct Operation {
    pub id: String,
    pub kind: OperationKind,
    pub state: OperationState,
    pub date: DateTime<Utc>,
    /// Kind of the instrument; `None` for operations without one or of an unsupported one
    pub instrument_kind: Option<InstrumentKind>,
    /// `None` when the API gave no amount or one of an unknown currency
    pub payment: Option<Money>,
    pub price: Option<Money>,
    /// Accrued coupon interest (NKD) of a bond trade
    pub accrued_interest: Option<Money>,
    pub quantity: i64,
    pub quantity_rest: i64,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::dividend(OperationKind::Dividend, true, false)]
    #[case::coupon(OperationKind::Coupon, true, false)]
    #[case::dividend_tax(OperationKind::DividendTax, true, false)]
    #[case::coupon_tax(OperationKind::CouponTax, true, false)]
    #[case::other_tax(OperationKind::OtherTax, true, false)]
    #[case::overnight(OperationKind::Overnight, true, false)]
    #[case::fee(OperationKind::Fee, false, true)]
    #[case::buy(OperationKind::Buy, false, false)]
    #[case::sell(OperationKind::Sell, false, false)]
    #[case::other(OperationKind::Other, false, false)]
    fn operation_kind_is_income_or_fee(
        #[case] kind: OperationKind,
        #[case] income: bool,
        #[case] fee: bool,
    ) {
        // Act
        let (is_income, is_fee) = (kind.is_income(), kind.is_fee());

        // Assert
        assert_eq!((is_income, is_fee), (income, fee));
    }

    #[test]
    fn taxed_kinds_have_tax_kind() {
        // Act
        let without_tax_kind: Vec<_> = OperationKind::TAXED
            .into_iter()
            .filter(|k| k.tax_kind().is_none())
            .collect();

        // Assert
        assert!(without_tax_kind.is_empty(), "{without_tax_kind:?}");
    }

    #[rstest]
    #[case::overnight(OperationKind::Overnight)]
    #[case::fee(OperationKind::Fee)]
    #[case::buy(OperationKind::Buy)]
    fn operation_kind_without_tax_kind(#[case] kind: OperationKind) {
        // Act
        let tax_kind = kind.tax_kind();

        // Assert
        assert_eq!(tax_kind, None);
    }
}
