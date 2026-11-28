use openbooks_core::{Currency, Email, OrgId, Role, UserId};

use crate::refused;
use sqlx::{PgConnection, query, query_as, query_scalar};

/// One person on a company's member list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub email: Email,
    pub user: Option<UserId>,
    pub role: Role,
}

type Row = (String, Option<i64>, String);

const COLUMNS: &str = "email, user_id, role";

fn member((email, user, role): Row) -> Result<Member, sqlx::Error> {
    Ok(Member {
        email: Email::new(&email).map_err(refused)?,
        user: user.map(UserId::new).transpose().map_err(refused)?,
        role: Role::parse(&role).map_err(refused)?,
    })
}

pub async fn create_org(db: &mut PgConnection, name: &str, base: Currency, pack: &str) -> Result<OrgId, sqlx::Error> {
    let sql = "INSERT INTO ledger.org (name, base_currency, pack) VALUES ($1, $2, $3) RETURNING org_id";
    let id: i64 = query_scalar(sql)
        .bind(name)
        .bind(base.code())
        .bind(pack)
        .fetch_one(&mut *db)
        .await?;
    let org = OrgId::new(id).map_err(refused)?;
    crate::chart::add_project(db, org).await?;
    Ok(org)
}

/// The name of the pack the company keeps its books under, if there is such a company.
pub async fn org_pack(db: &mut PgConnection, org: OrgId) -> Result<Option<String>, sqlx::Error> {
    query_scalar("SELECT pack FROM ledger.org WHERE org_id = $1")
        .bind(org.get())
        .fetch_optional(db)
        .await
}

/// Holds the company's row until the transaction ends, so member writes run one at a time; false if there is none.
pub async fn lock_org(db: &mut PgConnection, org: OrgId) -> Result<bool, sqlx::Error> {
    let found = query("SELECT 1 FROM ledger.org WHERE org_id = $1 FOR UPDATE")
        .bind(org.get())
        .fetch_optional(db)
        .await?;
    Ok(found.is_some())
}

pub async fn member_by_user(db: &mut PgConnection, org: OrgId, user: UserId) -> Result<Option<Member>, sqlx::Error> {
    let sql = format!("SELECT {COLUMNS} FROM ledger.member WHERE org_id = $1 AND user_id = $2 AND NOT removed");
    let row = query_as(&sql)
        .bind(org.get())
        .bind(user.get())
        .fetch_optional(db)
        .await?;
    row.map(member).transpose()
}

pub async fn member_by_email(db: &mut PgConnection, org: OrgId, email: &Email) -> Result<Option<Member>, sqlx::Error> {
    let sql = format!("SELECT {COLUMNS} FROM ledger.member WHERE org_id = $1 AND email = $2 AND NOT removed");
    let row = query_as(&sql)
        .bind(org.get())
        .bind(email.as_str())
        .fetch_optional(db)
        .await?;
    row.map(member).transpose()
}

/// Ties a member added by email to the user id that signed in with it, once.
pub async fn tie(
    db: &mut PgConnection,
    org: OrgId,
    email: &Email,
    user: UserId,
) -> Result<Option<Member>, sqlx::Error> {
    let sql = format!(
        "UPDATE ledger.member SET user_id = $3 WHERE org_id = $1 AND email = $2 AND user_id IS NULL AND NOT removed \
         RETURNING {COLUMNS}"
    );
    let row = query_as(&sql)
        .bind(org.get())
        .bind(email.as_str())
        .bind(user.get())
        .fetch_optional(db)
        .await?;
    row.map(member).transpose()
}

/// Adds a member, or brings back a removed one; false if the email is a member already.
pub async fn add(db: &mut PgConnection, org: OrgId, email: &Email, role: Role) -> Result<bool, sqlx::Error> {
    let added = query(
        "INSERT INTO ledger.member (org_id, email, role) VALUES ($1, $2, $3) \
         ON CONFLICT (org_id, email) DO UPDATE SET role = excluded.role, removed = false WHERE ledger.member.removed",
    )
    .bind(org.get())
    .bind(email.as_str())
    .bind(role.as_str())
    .execute(db)
    .await?;
    Ok(added.rows_affected() == 1)
}

/// Up to `limit` members by email, from just after `after`.
pub async fn members(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Member>, sqlx::Error> {
    let sql = format!(
        "SELECT {COLUMNS} FROM ledger.member WHERE org_id = $1 AND NOT removed AND ($2::text IS NULL OR email > $2) \
         ORDER BY email LIMIT $3"
    );
    let rows: Vec<Row> = query_as(&sql)
        .bind(org.get())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    rows.into_iter().map(member).collect()
}

pub async fn owners(db: &mut PgConnection, org: OrgId) -> Result<i64, sqlx::Error> {
    query_scalar("SELECT count(*) FROM ledger.member WHERE org_id = $1 AND role = 'owner' AND NOT removed")
        .bind(org.get())
        .fetch_one(db)
        .await
}

pub async fn set_role(db: &mut PgConnection, org: OrgId, email: &Email, role: Role) -> Result<(), sqlx::Error> {
    query("UPDATE ledger.member SET role = $3 WHERE org_id = $1 AND email = $2 AND NOT removed")
        .bind(org.get())
        .bind(email.as_str())
        .bind(role.as_str())
        .execute(db)
        .await
        .map(drop)
}

pub async fn remove(db: &mut PgConnection, org: OrgId, email: &Email) -> Result<(), sqlx::Error> {
    query("UPDATE ledger.member SET removed = true WHERE org_id = $1 AND email = $2")
        .bind(org.get())
        .bind(email.as_str())
        .execute(db)
        .await
        .map(drop)
}
