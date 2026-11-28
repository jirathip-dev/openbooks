use openbooks_core::{IdempotencyKey, OrgId, Origin, UserId};

use crate::refused;
use sqlx::{PgConnection, query, query_as, query_scalar};

/// One change, written in the change's own transaction and handed to the host's hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub org: OrgId,
    pub id: i64,
    pub action: String,
    pub detail: String,
    pub origin: Option<Origin>,
}

/// What a key already holds: nothing yet, the answer to the same request, or another request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    New,
    Answered(Option<String>),
    Reused,
}

pub async fn write(
    db: &mut PgConnection,
    org: OrgId,
    audit: &str,
    action: &str,
    detail: &str,
    origin: Option<&Origin>,
) -> Result<Event, sqlx::Error> {
    let id = query_scalar(
        "INSERT INTO ledger.event (org_id, audit_id, action, detail, origin) VALUES ($1, $2, $3, $4, $5) \
         RETURNING event_id",
    )
    .bind(org.get())
    .bind(audit)
    .bind(action)
    .bind(detail)
    .bind(origin.map(Origin::as_str))
    .fetch_one(db)
    .await?;
    Ok(Event {
        org,
        id,
        action: action.to_owned(),
        detail: detail.to_owned(),
        origin: origin.cloned(),
    })
}

/// The company's undelivered events in order, held until the transaction ends so each goes out once.
pub async fn pending(db: &mut PgConnection, org: OrgId) -> Result<Vec<Event>, sqlx::Error> {
    let rows: Vec<(i64, String, String, Option<String>)> = query_as(
        "SELECT event_id, action, detail, origin FROM ledger.event WHERE org_id = $1 AND NOT delivered \
         ORDER BY event_id FOR UPDATE",
    )
    .bind(org.get())
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(id, action, detail, origin)| {
            Ok(Event {
                org,
                id,
                action,
                detail,
                origin: origin.as_deref().map(Origin::new).transpose().map_err(refused)?,
            })
        })
        .collect()
}

pub async fn delivered(db: &mut PgConnection, org: OrgId, id: i64) -> Result<(), sqlx::Error> {
    query("UPDATE ledger.event SET delivered = true WHERE org_id = $1 AND event_id = $2")
        .bind(org.get())
        .bind(id)
        .execute(db)
        .await
        .map(drop)
}

/// Takes the key for this request, waiting out a transaction that holds it, or says what it holds already.
pub async fn claim(
    db: &mut PgConnection,
    user: UserId,
    key: &IdempotencyKey,
    request: &str,
) -> Result<Claim, sqlx::Error> {
    let taken =
        query("INSERT INTO openbooks.idempotency (user_id, key, request) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(user.get())
            .bind(key.as_str())
            .bind(request)
            .execute(&mut *db)
            .await?;
    if taken.rows_affected() == 1 {
        return Ok(Claim::New);
    }
    let (held, answer): (String, Option<String>) =
        query_as("SELECT request, answer FROM openbooks.idempotency WHERE user_id = $1 AND key = $2")
            .bind(user.get())
            .bind(key.as_str())
            .fetch_one(db)
            .await?;
    Ok(if held == request {
        Claim::Answered(answer)
    } else {
        Claim::Reused
    })
}

pub async fn answer(
    db: &mut PgConnection,
    user: UserId,
    key: &IdempotencyKey,
    answer: Option<&str>,
) -> Result<(), sqlx::Error> {
    query("UPDATE openbooks.idempotency SET answer = $3 WHERE user_id = $1 AND key = $2")
        .bind(user.get())
        .bind(key.as_str())
        .bind(answer)
        .execute(db)
        .await
        .map(drop)
}
