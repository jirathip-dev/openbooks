use std::time::{Duration, SystemTime, UNIX_EPOCH};

use openbooks_core::{Email, OrgId, Refusal, UserId};
use sqlx::{PgConnection, query, query_as, query_scalar};

use crate::refused;

/// What an audit row is about: the company itself, or one member by email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Company,
    Member(Email),
}

impl Record {
    /// The key a row is stored under and a read filters by.
    pub fn key(&self) -> String {
        match self {
            Self::Company => "company".to_owned(),
            Self::Member(email) => format!("member:{}", email.as_str()),
        }
    }
}

/// The one audit row a write leaves; the record's state after it is read when the row is written.
#[derive(Debug, Clone, Copy)]
pub struct Audit<'a> {
    pub id: &'a str,
    pub at: SystemTime,
    pub user: UserId,
    pub via: &'a str,
    pub action: &'a str,
    pub detail: &'a str,
    pub record: &'a Record,
    pub before: Option<&'a str>,
}

/// Which rows a read lists: each field set narrows it, from `from` up to just before `until`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub record: Option<Record>,
    pub user: Option<UserId>,
    pub action: Option<String>,
    pub from: Option<SystemTime>,
    pub until: Option<SystemTime>,
}

/// One audit row as a read lists it, with the record before and after the write as JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub seq: i64,
    pub id: String,
    pub at: SystemTime,
    pub user: UserId,
    pub via: String,
    pub action: String,
    pub record: String,
    pub detail: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

type Row = (
    i64,
    String,
    i64,
    i64,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn micros(at: SystemTime) -> Result<i64, sqlx::Error> {
    let since = at.duration_since(UNIX_EPOCH).map_err(|_| refused(Refusal::BadTime))?;
    i64::try_from(since.as_micros()).map_err(|_| refused(Refusal::BadTime))
}

fn row((seq, id, at, user, via, action, record, detail, before, after): Row) -> Result<AuditRow, sqlx::Error> {
    let at = u64::try_from(at).map_err(|_| refused(Refusal::BadTime))?;
    Ok(AuditRow {
        seq,
        id,
        at: UNIX_EPOCH + Duration::from_micros(at),
        user: UserId::new(user).map_err(refused)?,
        via,
        action,
        record,
        detail,
        before,
        after,
    })
}

/// The record's state now as JSON; none if it does not exist.
pub async fn snapshot(db: &mut PgConnection, org: OrgId, record: &Record) -> Result<Option<String>, sqlx::Error> {
    match record {
        Record::Company => {
            query_scalar("SELECT to_jsonb(o)::text FROM ledger.org o WHERE org_id = $1")
                .bind(org.get())
                .fetch_optional(db)
                .await
        }
        Record::Member(email) => {
            query_scalar("SELECT (to_jsonb(m) - 'org_id')::text FROM ledger.member m WHERE org_id = $1 AND email = $2")
                .bind(org.get())
                .bind(email.as_str())
                .fetch_optional(db)
                .await
        }
    }
}

pub async fn write(db: &mut PgConnection, org: OrgId, row: Audit<'_>) -> Result<(), sqlx::Error> {
    let after = snapshot(db, org, row.record).await?;
    query(
        "INSERT INTO ledger.audit (org_id, audit_id, at, user_id, via, action, detail, record, before, after) \
         VALUES ($1, $2, timestamptz 'epoch' + $3 * interval '1 microsecond', $4, $5, $6, $7, $8, $9::jsonb, $10::jsonb)",
    )
    .bind(org.get())
    .bind(row.id)
    .bind(micros(row.at)?)
    .bind(row.user.get())
    .bind(row.via)
    .bind(row.action)
    .bind(row.detail)
    .bind(row.record.key())
    .bind(row.before)
    .bind(after)
    .execute(db)
    .await
    .map(drop)
}

/// Up to `limit` of the company's rows the filter keeps, in the order they were written, from just after `after`.
pub async fn list(
    db: &mut PgConnection,
    org: OrgId,
    filter: &Filter,
    after: Option<i64>,
    limit: i64,
) -> Result<Vec<AuditRow>, sqlx::Error> {
    let rows: Vec<Row> = query_as(
        "SELECT seq, audit_id, (extract(epoch FROM at) * 1000000)::bigint, user_id, via, action, record, detail, \
         before::text, after::text FROM ledger.audit \
         WHERE org_id = $1 AND ($2::text IS NULL OR record = $2) AND ($3::bigint IS NULL OR user_id = $3) \
         AND ($4::text IS NULL OR action = $4) \
         AND ($5::bigint IS NULL OR at >= timestamptz 'epoch' + $5 * interval '1 microsecond') \
         AND ($6::bigint IS NULL OR at < timestamptz 'epoch' + $6 * interval '1 microsecond') \
         AND ($7::bigint IS NULL OR seq > $7) ORDER BY seq LIMIT $8",
    )
    .bind(org.get())
    .bind(filter.record.as_ref().map(Record::key))
    .bind(filter.user.map(UserId::get))
    .bind(filter.action.as_deref())
    .bind(filter.from.map(micros).transpose()?)
    .bind(filter.until.map(micros).transpose()?)
    .bind(after)
    .bind(limit)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(row).collect()
}
