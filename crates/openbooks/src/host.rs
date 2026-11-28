use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use openbooks_core::{Email, ProjectId, UserId, Word};
use sqlx::PgConnection;

use crate::Error;

/// A boxed future the product and its host hand each other.
pub type Work<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// What the product asks of the program that embeds it: who is signed in, and the transaction to work in.
pub trait Host: Send + Sync {
    fn who(&self) -> Option<SignedIn>;

    fn transact<T, F>(&self, work: F) -> impl Future<Output = Result<T, Error>> + Send
    where
        T: Send,
        F: for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, T> + Send;

    fn new_id(&self) -> String;

    fn now(&self) -> SystemTime;

    fn files(&self) -> &dyn Files;

    fn names(&self, users: &[UserId]) -> BTreeMap<UserId, String>;
}

/// The host's store for uploaded files, by the key it hands back.
pub trait Files: Send + Sync {
    fn put<'a>(&'a self, name: &'a str, bytes: Vec<u8>) -> Work<'a, String>;

    fn get<'a>(&'a self, key: &'a str) -> Work<'a, Vec<u8>>;
}

/// The person a call is for, as the host signed them in; what they may do is the product's to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedIn {
    pub user: UserId,
    pub email: Email,
    pub scope: Scope,
    pub via: Via,
}

impl SignedIn {
    pub fn person(user: UserId, email: Email) -> Self {
        Self {
            user,
            email,
            scope: Scope::Company,
            via: Via::Person,
        }
    }

    /// Whether the host's narrowing leaves this word; it never adds one the role lacks.
    pub fn keeps(&self, word: Word) -> bool {
        self.scope.keeps(word) && self.via.keeps(word)
    }
}

/// How much of the company a call reaches: all of it, or the host narrowed it to some projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Company,
    Projects(BTreeSet<ProjectId>),
}

impl Scope {
    pub fn covers(&self, project: ProjectId) -> bool {
        match self {
            Self::Company => true,
            Self::Projects(projects) => projects.contains(&project),
        }
    }

    fn keeps(&self, word: Word) -> bool {
        matches!(self, Self::Company) || word != Word::Own
    }
}

/// Who makes the call for the person: they do, an assistant does, or the host's own code does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Person,
    Assistant,
    Host,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Assistant => "assistant",
            Self::Host => "host",
        }
    }

    fn keeps(self, word: Word) -> bool {
        self != Self::Assistant || !matches!(word, Word::Approve | Word::Own)
    }
}
