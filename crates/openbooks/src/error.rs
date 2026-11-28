use std::fmt;

use openbooks_core::{Email, Refusal, Word};

/// Why the product did not do what a caller asked.
#[derive(Debug)]
pub enum Error {
    SignedOut,
    NotMember { name: String },
    Forbidden { word: Word },
    AlreadyMember(Email),
    NoSuchMember(Email),
    LastOwner,
    Refused(Refusal),
    Host(String),
    Database(sqlx::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedOut => f.write_str("nobody is signed in"),
            Self::NotMember { name } => write!(f, "{name} is not a member of this company"),
            Self::Forbidden { word } => write!(f, "this needs the {} permission", word.as_str()),
            Self::AlreadyMember(email) => write!(f, "{} is a member already", email.as_str()),
            Self::NoSuchMember(email) => write!(f, "{} is not a member", email.as_str()),
            Self::LastOwner => f.write_str("a company always has an owner: the last one stays"),
            Self::Refused(refusal) => refusal.fmt(f),
            Self::Host(why) => write!(f, "the host failed: {why}"),
            Self::Database(error) => write!(f, "the database failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<Refusal> for Error {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}
