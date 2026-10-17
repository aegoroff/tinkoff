use chrono::{DateTime, Utc};

const ACCOUNT_TYPE_HELP: &str =
    "tinkoff (broker, default), iis, invest-box, invest-fund, debit, saving, dfa";

/// Type of a brokerage account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountType {
    /// Broker account
    Tinkoff,
    /// Individual investment account
    TinkoffIis,
    InvestBox,
    InvestFund,
    Debit,
    Saving,
    /// Digital financial assets account
    Dfa,
    Unspecified,
}

impl AccountType {
    /// Parses a CLI account type name.
    ///
    /// # Errors
    ///
    /// Returns an error if `value` is not a known account type name.
    pub fn parse(value: &str) -> Result<Self, String> {
        let names: [(&[&str], Self); 7] = [
            (&["tinkoff", "broker"], Self::Tinkoff),
            (&["iis", "tinkoff-iis"], Self::TinkoffIis),
            (&["invest-box", "investbox"], Self::InvestBox),
            (&["invest-fund", "investfund"], Self::InvestFund),
            (&["debit"], Self::Debit),
            (&["saving"], Self::Saving),
            (&["dfa"], Self::Dfa),
        ];
        names
            .into_iter()
            .find(|(aliases, _)| aliases.iter().any(|a| value.eq_ignore_ascii_case(a)))
            .map(|(_, account_type)| account_type)
            .ok_or_else(|| {
                format!("unknown account type '{value}'; expected one of: {ACCOUNT_TYPE_HELP}")
            })
    }

    /// CLI name of the account type, the inverse of [`AccountType::parse`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Tinkoff => "tinkoff",
            Self::TinkoffIis => "iis",
            Self::InvestBox => "invest-box",
            Self::InvestFund => "invest-fund",
            Self::Debit => "debit",
            Self::Saving => "saving",
            Self::Dfa => "dfa",
            Self::Unspecified => "unspecified",
        }
    }
}

/// Status of a brokerage account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    /// Being opened
    New,
    Open,
    Closed,
    Unspecified,
}

impl AccountStatus {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Unspecified => "unspecified",
        }
    }
}

/// Brokerage account of the user.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub account_type: AccountType,
    pub status: AccountStatus,
    /// `None` when the API gave no date
    pub opened: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::tinkoff("tinkoff", AccountType::Tinkoff)]
    #[case::broker("broker", AccountType::Tinkoff)]
    #[case::iis("iis", AccountType::TinkoffIis)]
    #[case::invest_box("invest-box", AccountType::InvestBox)]
    #[case::case_insensitive("InvestBox", AccountType::InvestBox)]
    fn parse_account_type_aliases(#[case] value: &str, #[case] expected: AccountType) {
        // Act
        let parsed = AccountType::parse(value);

        // Assert
        assert_eq!(parsed, Ok(expected));
    }

    #[rstest]
    #[case(AccountType::Tinkoff)]
    #[case(AccountType::TinkoffIis)]
    #[case(AccountType::InvestBox)]
    #[case(AccountType::InvestFund)]
    #[case(AccountType::Debit)]
    #[case(AccountType::Saving)]
    #[case(AccountType::Dfa)]
    fn account_type_name_round_trips(#[case] account_type: AccountType) {
        // Arrange
        let name = account_type.name();

        // Act
        let parsed = AccountType::parse(name);

        // Assert
        assert_eq!(parsed, Ok(account_type));
    }

    #[rstest]
    #[case::unknown("savings")]
    #[case::unspecified("unspecified")]
    #[case::empty("")]
    fn parse_account_type_rejects(#[case] value: &str) {
        // Act
        let parsed = AccountType::parse(value);

        // Assert
        assert!(parsed.is_err());
    }
}
