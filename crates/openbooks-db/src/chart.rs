use openbooks_core::{AccountType, Date, OrgId};
use sqlx::{PgConnection, query, query_as, query_scalar};

use crate::refused;

/// One account of a company's chart, by its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent: Option<String>,
    pub active: bool,
    pub currency: Option<String>,
}

/// The account that plays a role, from the day it took it on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAccount {
    pub role: String,
    pub from: Date,
    pub account: String,
}

/// A kind of dimension the company tags lines with, in its slot of three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dimension {
    pub slot: u16,
    pub name: String,
}

/// One value of a dimension kind; one with a host reference is kept in step by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionValue {
    pub code: String,
    pub name: String,
    pub active: bool,
    pub host_ref: Option<String>,
}

/// A payment term: what a document is due in, in days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    pub code: String,
    pub name: String,
    pub days: u16,
    pub active: bool,
}

type AccountRow = (String, String, String, Option<String>, bool, Option<String>);

fn account((code, name, account_type, parent, active, currency): AccountRow) -> Result<Account, sqlx::Error> {
    Ok(Account {
        code,
        name,
        account_type: AccountType::parse(&account_type).map_err(refused)?,
        parent,
        active,
        currency,
    })
}

fn slot(slot: i16) -> u16 {
    u16::try_from(slot).unwrap_or_default()
}

fn day(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.month().year(),
        date.month().month(),
        date.day()
    )
}

const ACCOUNT: &str = "SELECT a.code, a.name, a.type, p.code, a.active, a.currency FROM ledger.account a \
     LEFT JOIN ledger.account p ON p.org_id = a.org_id AND p.account_id = a.parent_id";

pub async fn account_by_code(db: &mut PgConnection, org: OrgId, code: &str) -> Result<Option<Account>, sqlx::Error> {
    let row = query_as(&format!("{ACCOUNT} WHERE a.org_id = $1 AND a.code = $2"))
        .bind(org.get())
        .bind(code)
        .fetch_optional(db)
        .await?;
    row.map(account).transpose()
}

/// Up to `limit` accounts by code, from just after `after`.
pub async fn accounts(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Account>, sqlx::Error> {
    let sql = format!("{ACCOUNT} WHERE a.org_id = $1 AND ($2::text IS NULL OR a.code > $2) ORDER BY a.code LIMIT $3");
    let rows: Vec<AccountRow> = query_as(&sql)
        .bind(org.get())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    rows.into_iter().map(account).collect()
}

/// Whether a posted entry has a line on the account.
pub async fn posted_to(db: &mut PgConnection, org: OrgId, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ledger.line l JOIN ledger.account a USING (org_id, account_id) \
         JOIN ledger.entry e USING (org_id, entry_id) WHERE l.org_id = $1 AND a.code = $2 AND e.posted_at IS NOT NULL)",
    )
    .bind(org.get())
    .bind(code)
    .fetch_one(db)
    .await
}

/// Whether the account `code` is `parent` or one of the accounts over it.
pub async fn above_or_is(db: &mut PgConnection, org: OrgId, parent: &str, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar(
        "WITH RECURSIVE up AS (SELECT account_id, parent_id, code FROM ledger.account WHERE org_id = $1 AND code = $2 \
         UNION SELECT a.account_id, a.parent_id, a.code FROM ledger.account a \
         JOIN up ON a.org_id = $1 AND a.account_id = up.parent_id) SELECT EXISTS (SELECT 1 FROM up WHERE code = $3)",
    )
    .bind(org.get())
    .bind(parent)
    .bind(code)
    .fetch_one(db)
    .await
}

/// Adds the account, or changes the one with its code; its parent must be on the chart.
pub async fn save_account(db: &mut PgConnection, org: OrgId, account: &Account) -> Result<(), sqlx::Error> {
    query(
        "INSERT INTO ledger.account (org_id, code, name, type, parent_id, active, currency) \
         VALUES ($1, $2, $3, $4, (SELECT account_id FROM ledger.account WHERE org_id = $1 AND code = $5), $6, $7) \
         ON CONFLICT (org_id, code) DO UPDATE SET name = excluded.name, type = excluded.type, \
         parent_id = excluded.parent_id, active = excluded.active, currency = excluded.currency",
    )
    .bind(org.get())
    .bind(&account.code)
    .bind(&account.name)
    .bind(account.account_type.as_str())
    .bind(account.parent.as_deref())
    .bind(account.active)
    .bind(account.currency.as_deref())
    .execute(db)
    .await
    .map(drop)
}

/// Says whether the account needs a value of the kind in `slot` on each line; false if either is missing.
pub async fn require(
    db: &mut PgConnection,
    org: OrgId,
    code: &str,
    slot: u16,
    required: bool,
) -> Result<bool, sqlx::Error> {
    let done = query(
        "INSERT INTO ledger.account_dimension (org_id, account_id, kind_id, required) \
         SELECT a.org_id, a.account_id, k.kind_id, $4 FROM ledger.account a \
         JOIN ledger.dimension k ON k.org_id = a.org_id AND k.slot = $3 AND NOT k.removed \
         WHERE a.org_id = $1 AND a.code = $2 \
         ON CONFLICT (org_id, account_id, kind_id) DO UPDATE SET required = excluded.required",
    )
    .bind(org.get())
    .bind(code)
    .bind(i16::try_from(slot).unwrap_or_default())
    .bind(required)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Gives the role to the account from the date on; false if the account is not on the chart.
pub async fn give_role(
    db: &mut PgConnection,
    org: OrgId,
    role: &str,
    from: Date,
    code: &str,
) -> Result<bool, sqlx::Error> {
    let done = query(
        "INSERT INTO ledger.account_role (org_id, role, from_date, account_id) \
         SELECT org_id, $2, $3::date, account_id FROM ledger.account WHERE org_id = $1 AND code = $4 \
         ON CONFLICT (org_id, role, from_date) DO UPDATE SET account_id = excluded.account_id",
    )
    .bind(org.get())
    .bind(role)
    .bind(day(from))
    .bind(code)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// The account that plays each role on the date, by role.
pub async fn roles_on(db: &mut PgConnection, org: OrgId, on: Date) -> Result<Vec<RoleAccount>, sqlx::Error> {
    let rows: Vec<(String, i32, i32, i32, String)> = query_as(
        "SELECT DISTINCT ON (r.role) r.role, extract(year FROM r.from_date)::int, extract(month FROM r.from_date)::int, \
         extract(day FROM r.from_date)::int, a.code FROM ledger.account_role r JOIN ledger.account a USING (org_id, account_id) \
         WHERE r.org_id = $1 AND r.from_date <= $2::date ORDER BY r.role, r.from_date DESC",
    )
    .bind(org.get())
    .bind(day(on))
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(role, year, month, day, account)| {
            let part = |n: i32| u8::try_from(n).unwrap_or_default();
            let year = u16::try_from(year).unwrap_or_default();
            let from = Date::new(year, part(month), part(day)).map_err(refused)?;
            Ok(RoleAccount { role, from, account })
        })
        .collect()
}

/// Makes the company's first kind, Project, which it always has.
pub async fn add_project(db: &mut PgConnection, org: OrgId) -> Result<(), sqlx::Error> {
    query("INSERT INTO ledger.dimension (org_id, slot, name) VALUES ($1, 1, 'Project')")
        .bind(org.get())
        .execute(db)
        .await
        .map(drop)
}

/// The company's kinds in use, by slot.
pub async fn dimensions(db: &mut PgConnection, org: OrgId) -> Result<Vec<Dimension>, sqlx::Error> {
    let rows: Vec<(i16, String)> =
        query_as("SELECT slot, name FROM ledger.dimension WHERE org_id = $1 AND NOT removed ORDER BY slot")
            .bind(org.get())
            .fetch_all(db)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(s, name)| Dimension { slot: slot(s), name })
        .collect())
}

/// Turns on a kind in the first free slot; none if all three are taken.
pub async fn add_dimension(db: &mut PgConnection, org: OrgId, name: &str) -> Result<Option<u16>, sqlx::Error> {
    let taken: Option<i16> = query_scalar(
        "INSERT INTO ledger.dimension (org_id, slot, name) SELECT $1, s, $2 FROM generate_series(2, 3) s \
         WHERE NOT EXISTS (SELECT 1 FROM ledger.dimension WHERE org_id = $1 AND slot = s AND NOT removed) \
         ORDER BY s LIMIT 1 RETURNING slot",
    )
    .bind(org.get())
    .bind(name)
    .fetch_optional(db)
    .await?;
    Ok(taken.map(slot))
}

/// Renames the kind in `slot`, or takes it out of use; false if no kind is there.
pub async fn change_dimension(
    db: &mut PgConnection,
    org: OrgId,
    at: u16,
    name: &str,
    removed: bool,
) -> Result<bool, sqlx::Error> {
    let done = query(
        "UPDATE ledger.dimension SET name = $3, removed = $4 WHERE org_id = $1 AND slot = $2 AND slot > 1 AND NOT removed",
    )
    .bind(org.get())
    .bind(i16::try_from(at).unwrap_or_default())
    .bind(name)
    .bind(removed)
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Whether a posted line carries a value of the kind in `slot`.
pub async fn dimension_posted(db: &mut PgConnection, org: OrgId, at: u16) -> Result<bool, sqlx::Error> {
    query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ledger.line_dimension d JOIN ledger.dimension k USING (org_id, kind_id) \
         JOIN ledger.entry e USING (org_id, entry_id) \
         WHERE d.org_id = $1 AND k.slot = $2 AND NOT k.removed AND e.posted_at IS NOT NULL)",
    )
    .bind(org.get())
    .bind(i16::try_from(at).unwrap_or_default())
    .fetch_one(db)
    .await
}

const VALUE: &str = "SELECT v.code, v.name, v.active, v.host_ref FROM ledger.dimension_value v \
     JOIN ledger.dimension k ON k.org_id = v.org_id AND k.kind_id = v.kind_id AND NOT k.removed";

fn value((code, name, active, host_ref): (String, String, bool, Option<String>)) -> DimensionValue {
    DimensionValue {
        code,
        name,
        active,
        host_ref,
    }
}

pub async fn value_by_code(
    db: &mut PgConnection,
    org: OrgId,
    at: u16,
    code: &str,
) -> Result<Option<DimensionValue>, sqlx::Error> {
    let row = query_as(&format!("{VALUE} WHERE v.org_id = $1 AND k.slot = $2 AND v.code = $3"))
        .bind(org.get())
        .bind(i16::try_from(at).unwrap_or_default())
        .bind(code)
        .fetch_optional(db)
        .await?;
    Ok(row.map(value))
}

/// Up to `limit` values of the kind in `slot` by code, from just after `after`.
pub async fn values(
    db: &mut PgConnection,
    org: OrgId,
    at: u16,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<DimensionValue>, sqlx::Error> {
    let sql = format!(
        "{VALUE} WHERE v.org_id = $1 AND k.slot = $2 AND ($3::text IS NULL OR v.code > $3) ORDER BY v.code LIMIT $4"
    );
    let rows: Vec<(String, String, bool, Option<String>)> = query_as(&sql)
        .bind(org.get())
        .bind(i16::try_from(at).unwrap_or_default())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    Ok(rows.into_iter().map(value).collect())
}

/// Adds the value to the kind in `slot`, or changes the one with its code or, when it has one, its host reference; false if no kind is there.
pub async fn save_value(
    db: &mut PgConnection,
    org: OrgId,
    at: u16,
    saved: &DimensionValue,
) -> Result<bool, sqlx::Error> {
    let key = if saved.host_ref.is_some() {
        "(org_id, kind_id, host_ref)"
    } else {
        "(org_id, kind_id, code)"
    };
    let done = query(&format!(
        "INSERT INTO ledger.dimension_value (org_id, kind_id, code, name, active, host_ref) \
         SELECT org_id, kind_id, $3, $4, $5, $6 FROM ledger.dimension WHERE org_id = $1 AND slot = $2 AND NOT removed \
         ON CONFLICT {key} DO UPDATE SET code = excluded.code, name = excluded.name, active = excluded.active"
    ))
    .bind(org.get())
    .bind(i16::try_from(at).unwrap_or_default())
    .bind(&saved.code)
    .bind(&saved.name)
    .bind(saved.active)
    .bind(saved.host_ref.as_deref())
    .execute(db)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Adds the term, or changes the one with its code.
pub async fn save_terms(db: &mut PgConnection, org: OrgId, terms: &Terms) -> Result<(), sqlx::Error> {
    query(
        "INSERT INTO ledger.terms (org_id, code, name, days, active) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (org_id, code) DO UPDATE SET name = excluded.name, days = excluded.days, active = excluded.active",
    )
    .bind(org.get())
    .bind(&terms.code)
    .bind(&terms.name)
    .bind(i32::from(terms.days))
    .bind(terms.active)
    .execute(db)
    .await
    .map(drop)
}

/// Up to `limit` payment terms by code, from just after `after`.
pub async fn terms(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Terms>, sqlx::Error> {
    let rows: Vec<(String, String, i32, bool)> = query_as(
        "SELECT code, name, days, active FROM ledger.terms WHERE org_id = $1 AND ($2::text IS NULL OR code > $2) \
         ORDER BY code LIMIT $3",
    )
    .bind(org.get())
    .bind(after)
    .bind(limit)
    .fetch_all(db)
    .await?;
    let terms = rows.into_iter().map(|(code, name, days, active)| Terms {
        code,
        name,
        days: u16::try_from(days).unwrap_or_default(),
        active,
    });
    Ok(terms.collect())
}
