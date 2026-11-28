use std::collections::BTreeSet;
use std::fmt;

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// One step of the schema, for a host that applies migrations its own way, in version order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Why a database's schema is not the one this build was made for.
#[derive(Debug)]
pub enum SchemaError {
    Missing(Vec<i64>),
    Unknown(Vec<i64>),
    Database(sqlx::Error),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(versions) => write!(f, "the database lacks migrations {versions:?}"),
            Self::Unknown(versions) => write!(f, "the database has migrations this build lacks: {versions:?}"),
            Self::Database(error) => write!(f, "the schema could not be read: {error}"),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<sqlx::Error> for SchemaError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

pub fn migrations() -> impl Iterator<Item = Migration> {
    MIGRATOR.iter().map(|m| Migration {
        version: m.version,
        name: &m.description,
        sql: &m.sql,
    })
}

/// Applies every migration this build has that the database lacks, each in its own transaction.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

/// Confirms the database holds exactly this build's migrations, however they were applied.
pub async fn check(pool: &PgPool) -> Result<(), SchemaError> {
    let applied: BTreeSet<i64> = match sqlx::query_scalar!("SELECT version FROM openbooks.schema_version")
        .fetch_all(pool)
        .await
    {
        Ok(versions) => versions.into_iter().collect(),
        Err(sqlx::Error::Database(e)) if e.code().as_deref() == Some("42P01") => BTreeSet::new(),
        Err(e) => return Err(e.into()),
    };
    let known: BTreeSet<i64> = migrations().map(|m| m.version).collect();
    let missing: Vec<i64> = known.difference(&applied).copied().collect();
    let unknown: Vec<i64> = applied.difference(&known).copied().collect();
    if !missing.is_empty() {
        Err(SchemaError::Missing(missing))
    } else if !unknown.is_empty() {
        Err(SchemaError::Unknown(unknown))
    } else {
        Ok(())
    }
}
