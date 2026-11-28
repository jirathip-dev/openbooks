pub mod audit;
pub mod chart;
pub mod events;
pub mod items;
pub mod members;

use std::collections::BTreeSet;
use std::fmt;

use openbooks_core::Refusal;
use sqlx::migrate::Migrator;
use sqlx::{PgConnection, PgPool};

fn refused(refusal: Refusal) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(refusal))
}

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
    Pack(String),
    Database(sqlx::Error),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(versions) => write!(f, "the database lacks migrations {versions:?}"),
            Self::Unknown(versions) => write!(f, "the database has migrations this build lacks: {versions:?}"),
            Self::Pack(why) => f.write_str(why),
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

/// Applies, in one transaction, each of a pack's migrations the database lacks, inside the pack's own schema `pack_<name>`.
pub async fn migrate_pack(pool: &PgPool, pack: &str, steps: &[Migration]) -> Result<(), SchemaError> {
    if pack.is_empty() || !pack.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return Err(SchemaError::Pack(format!(
            "a pack's name is lowercase letters and _, not {pack:?}"
        )));
    }
    let schema = format!("pack_{pack}");
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('openbooks.migrate'))")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(&format!(
        "CREATE SCHEMA IF NOT EXISTS {schema}; SET LOCAL search_path TO {schema}"
    ))
    .execute(&mut *tx)
    .await?;
    let applied: BTreeSet<i64> = sqlx::query_scalar("SELECT version FROM openbooks.pack_version WHERE pack = $1")
        .bind(pack)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
    let outside =
        "SELECT count(*) FROM pg_class WHERE relnamespace NOT IN ($1::regnamespace, 'pg_toast'::regnamespace)";
    for step in steps.iter().filter(|m| !applied.contains(&m.version)) {
        let before: i64 = sqlx::query_scalar(outside).bind(&schema).fetch_one(&mut *tx).await?;
        sqlx::raw_sql(step.sql).execute(&mut *tx).await?;
        let after: i64 = sqlx::query_scalar(outside).bind(&schema).fetch_one(&mut *tx).await?;
        if after != before {
            let why = format!(
                "pack {pack}'s migration {} made a relation outside {schema}",
                step.version
            );
            return Err(SchemaError::Pack(why));
        }
        sqlx::query("INSERT INTO openbooks.pack_version (pack, version) VALUES ($1, $2)")
            .bind(pack)
            .bind(step.version)
            .execute(&mut *tx)
            .await?;
    }
    Ok(tx.commit().await?)
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
