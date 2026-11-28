use openbooks_core::{ContactKind, Email, OrgId};
use sqlx::{PgConnection, query, query_as, query_scalar};

use crate::refused;

/// A customer, supplier or employee by its code: its name, how to reach it and what its documents default to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub code: String,
    pub name: String,
    pub kind: ContactKind,
    pub address: Option<String>,
    pub email: Option<Email>,
    pub phone: Option<String>,
    pub terms: Option<String>,
    pub currency: Option<String>,
    pub withholding: Option<String>,
    pub host_ref: Option<String>,
}

/// A contact's tax id and branch as plain text, and the bank account it is paid to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub tax_id: Option<String>,
    pub branch: Option<String>,
    pub bank_account: Option<String>,
}

type Row = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn contact(
    (code, name, kind, address, email, phone, terms, currency, withholding, host_ref): Row,
) -> Result<Contact, sqlx::Error> {
    Ok(Contact {
        code,
        name,
        kind: ContactKind::parse(&kind).map_err(refused)?,
        address,
        email: email.map(|e| Email::new(&e)).transpose().map_err(refused)?,
        phone,
        terms,
        currency,
        withholding,
        host_ref,
    })
}

const CONTACT: &str = "SELECT c.code, c.name, c.kind, c.address, c.email, c.phone, t.code, c.currency, c.withholding, \
     c.host_ref FROM ledger.contact c LEFT JOIN ledger.terms t ON t.org_id = c.org_id AND t.terms_id = c.terms_id";

/// Up to `limit` contacts by code, from just after `after`.
pub async fn contacts(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Contact>, sqlx::Error> {
    let sql = format!("{CONTACT} WHERE c.org_id = $1 AND ($2::text IS NULL OR c.code > $2) ORDER BY c.code LIMIT $3");
    let rows: Vec<Row> = query_as(&sql)
        .bind(org.get())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    rows.into_iter().map(contact).collect()
}

/// The contact with the code, or with the host reference when `by_host` is set.
pub async fn contact_by(
    db: &mut PgConnection,
    org: OrgId,
    key: &str,
    by_host: bool,
) -> Result<Option<(Contact, Identity)>, sqlx::Error> {
    let column = if by_host { "host_ref" } else { "code" };
    let row: Option<Row> = query_as(&format!("{CONTACT} WHERE c.org_id = $1 AND c.{column} = $2"))
        .bind(org.get())
        .bind(key)
        .fetch_optional(&mut *db)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let (tax_id, branch, bank_account) = query_as(&format!(
        "SELECT tax_id, branch, bank_account FROM ledger.contact WHERE org_id = $1 AND {column} = $2"
    ))
    .bind(org.get())
    .bind(key)
    .fetch_one(db)
    .await?;
    let identity = Identity {
        tax_id,
        branch,
        bank_account,
    };
    Ok(Some((contact(row)?, identity)))
}

/// Whether the company has payment terms with the code.
pub async fn terms_exist(db: &mut PgConnection, org: OrgId, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar("SELECT EXISTS (SELECT 1 FROM ledger.terms WHERE org_id = $1 AND code = $2)")
        .bind(org.get())
        .bind(code)
        .fetch_one(db)
        .await
}

/// Adds the contact with its tax id and branch, or changes the one with its code or, when it has one, its host reference; its bank account stays.
pub async fn save(
    db: &mut PgConnection,
    org: OrgId,
    saved: &Contact,
    tax_id: Option<&str>,
    branch: Option<&str>,
) -> Result<(), sqlx::Error> {
    let key = if saved.host_ref.is_some() {
        "(org_id, host_ref)"
    } else {
        "(org_id, code)"
    };
    query(&format!(
        "INSERT INTO ledger.contact (org_id, code, name, kind, address, email, phone, terms_id, currency, withholding, \
         tax_id, branch, host_ref) VALUES ($1, $2, $3, $4, $5, $6, $7, \
         (SELECT terms_id FROM ledger.terms WHERE org_id = $1 AND code = $8), $9, $10, $11, $12, $13) \
         ON CONFLICT {key} DO UPDATE SET code = excluded.code, name = excluded.name, kind = excluded.kind, \
         address = excluded.address, email = excluded.email, phone = excluded.phone, terms_id = excluded.terms_id, \
         currency = excluded.currency, withholding = excluded.withholding, tax_id = excluded.tax_id, \
         branch = excluded.branch"
    ))
    .bind(org.get())
    .bind(&saved.code)
    .bind(&saved.name)
    .bind(saved.kind.as_str())
    .bind(saved.address.as_deref())
    .bind(saved.email.as_ref().map(Email::as_str))
    .bind(saved.phone.as_deref())
    .bind(saved.terms.as_deref())
    .bind(saved.currency.as_deref())
    .bind(saved.withholding.as_deref())
    .bind(tax_id)
    .bind(branch)
    .bind(saved.host_ref.as_deref())
    .execute(db)
    .await
    .map(drop)
}

/// Sets the bank account of the contact with the code; false if there is none.
pub async fn set_bank(
    db: &mut PgConnection,
    org: OrgId,
    code: &str,
    bank_account: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let done = query("UPDATE ledger.contact SET bank_account = $3 WHERE org_id = $1 AND code = $2")
        .bind(org.get())
        .bind(code)
        .bind(bank_account)
        .execute(db)
        .await?;
    Ok(done.rows_affected() == 1)
}
