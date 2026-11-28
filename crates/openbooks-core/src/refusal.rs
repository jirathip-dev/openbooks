use std::fmt;

use crate::Currency;

/// Why a core operation would not produce a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    BadCurrency,
    CurrencyMismatch { left: Currency, right: Currency },
    Overflow,
    NoWeights,
    BadRate,
    BadId,
    BadMonth,
    BadRole,
    BadEmail,
    BadKey,
    BadOrigin,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadCurrency => f.write_str("a currency is three capital letters with at most four minor places"),
            Self::CurrencyMismatch { left, right } => write!(f, "{left} and {right} do not mix without a rate"),
            Self::Overflow => f.write_str("the amount is too large"),
            Self::NoWeights => f.write_str("an allocation needs at least one line with a weight"),
            Self::BadRate => f.write_str("a rate is above zero, and one between a currency and itself"),
            Self::BadId => f.write_str("an id is above zero"),
            Self::BadMonth => f.write_str("a month is 1 to 12 in a year 1 to 9999"),
            Self::BadRole => f.write_str("a role is owner, accountant, staff or viewer"),
            Self::BadEmail => f.write_str("an email has a name, an @ and a domain"),
            Self::BadKey => f.write_str("an idempotency key is 1 to 255 visible characters"),
            Self::BadOrigin => f.write_str("an origin is 1 to 255 visible characters"),
        }
    }
}

impl std::error::Error for Refusal {}
