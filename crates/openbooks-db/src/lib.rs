pub mod members;

use std::collections::BTreeSet;
use std::fmt;

use sqlx::migrate::Migrator;
use sqlx::{PgConnection, PgPool};

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

/// Applies, in one transaction, every migration this build has that the database lacks.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('openbooks.migrate'))")
        .execute(&mut *tx)
        .await?;
    let applied = applied(&mut tx).await?;
    for migration in migrations().filter(|m| !applied.contains(&m.version)) {
        sqlx::raw_sql(migration.sql).execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Confirms the database holds exactly this build's migrations, however they were applied.
pub async fn check(pool: &PgPool) -> Result<(), SchemaError> {
    let applied = applied(&mut *pool.acquire().await?).await?;
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

async fn applied(db: &mut PgConnection) -> Result<BTreeSet<i64>, sqlx::Error> {
    let exists: Option<bool> = sqlx::query_scalar("SELECT to_regclass('openbooks.schema_version') IS NOT NULL")
        .fetch_one(&mut *db)
        .await?;
    if exists != Some(true) {
        return Ok(BTreeSet::new());
    }
    let versions = sqlx::query_scalar!("SELECT version FROM openbooks.schema_version")
        .fetch_all(db)
        .await?;
    Ok(versions.into_iter().collect())
}
