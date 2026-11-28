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
    NoSuchPack(String),
    NoSuchAccount(String),
    AccountPosted(String),
    ParentLoop(String),
    NoSuchAccountRole(String),
    DimensionsFull,
    NoSuchDimension(u16),
    ProjectDimension,
    DimensionPosted(u16),
    Synced(String),
    KeyReused,
    HookRefused(String),
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
            Self::NoSuchPack(name) => write!(f, "these books offer no pack {name:?}"),
            Self::NoSuchAccount(code) => write!(f, "the chart has no account {code}"),
            Self::AccountPosted(code) => write!(f, "account {code} has postings: its type stays"),
            Self::ParentLoop(code) => write!(f, "account {code} cannot sit under itself"),
            Self::NoSuchAccountRole(role) => {
                write!(f, "neither the core nor the company's pack declares the role {role}")
            }
            Self::DimensionsFull => f.write_str("a company has at most three dimension kinds"),
            Self::NoSuchDimension(slot) => write!(f, "the company has no dimension kind in slot {slot}"),
            Self::ProjectDimension => f.write_str("the first dimension kind is always Project"),
            Self::DimensionPosted(slot) => write!(f, "posted lines carry the kind in slot {slot}: rename it instead"),
            Self::Synced(code) => write!(f, "{code} is kept in step by the host: it is read-only here"),
            Self::KeyReused => f.write_str("this idempotency key was sent with another request"),
            Self::HookRefused(why) => write!(f, "the host refused the change: {why}"),
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
