use openbooks_core::{ItemKind, OrgId, Quantity};
use sqlx::{PgConnection, query, query_as, query_scalar};

use crate::refused;

/// A unit the company counts goods and services in, by its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub code: String,
    pub name: String,
}

/// A good or service of the catalogue, by its code; one with a host reference is kept in step by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub code: String,
    pub name: String,
    pub kind: ItemKind,
    pub unit: String,
    pub sales_account: Option<String>,
    pub purchase_account: Option<String>,
    pub sales_tax: Option<String>,
    pub purchase_tax: Option<String>,
    pub weight: Option<Quantity>,
    pub volume: Option<Quantity>,
    pub host_ref: Option<String>,
}

/// Which side of trade a line is on, and so which of an item's defaults it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Sale,
    Purchase,
}

/// What a line that names an item starts with: the item's unit, and its account and tax code for the side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDefaults {
    pub unit: String,
    pub account: Option<String>,
    pub tax: Option<String>,
}

impl Item {
    pub fn defaults(&self, side: Side) -> LineDefaults {
        let (account, tax) = match side {
            Side::Sale => (&self.sales_account, &self.sales_tax),
            Side::Purchase => (&self.purchase_account, &self.purchase_tax),
        };
        LineDefaults {
            unit: self.unit.clone(),
            account: account.clone(),
            tax: tax.clone(),
        }
    }
}

type ItemRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

fn item(
    (code, name, kind, unit, sales_account, purchase_account, sales_tax, purchase_tax, weight, volume, host_ref): ItemRow,
) -> Result<Item, sqlx::Error> {
    Ok(Item {
        code,
        name,
        kind: ItemKind::parse(&kind).map_err(refused)?,
        unit,
        sales_account,
        purchase_account,
        sales_tax,
        purchase_tax,
        weight: weight.map(Quantity),
        volume: volume.map(Quantity),
        host_ref,
    })
}

const ITEM: &str = "SELECT i.code, i.name, i.kind, u.code, s.code, p.code, i.sales_tax, i.purchase_tax, \
     i.weight_e4, i.volume_e4, i.host_ref FROM ledger.item i \
     JOIN ledger.unit u ON u.org_id = i.org_id AND u.unit_id = i.unit_id \
     LEFT JOIN ledger.account s ON s.org_id = i.org_id AND s.account_id = i.sales_account_id \
     LEFT JOIN ledger.account p ON p.org_id = i.org_id AND p.account_id = i.purchase_account_id";

pub async fn item_by_code(db: &mut PgConnection, org: OrgId, code: &str) -> Result<Option<Item>, sqlx::Error> {
    let row = query_as(&format!("{ITEM} WHERE i.org_id = $1 AND i.code = $2"))
        .bind(org.get())
        .bind(code)
        .fetch_optional(db)
        .await?;
    row.map(item).transpose()
}

pub async fn item_by_host_ref(db: &mut PgConnection, org: OrgId, host_ref: &str) -> Result<Option<Item>, sqlx::Error> {
    let row = query_as(&format!("{ITEM} WHERE i.org_id = $1 AND i.host_ref = $2"))
        .bind(org.get())
        .bind(host_ref)
        .fetch_optional(db)
        .await?;
    row.map(item).transpose()
}

/// Up to `limit` items by code, from just after `after`.
pub async fn items(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Item>, sqlx::Error> {
    let sql = format!("{ITEM} WHERE i.org_id = $1 AND ($2::text IS NULL OR i.code > $2) ORDER BY i.code LIMIT $3");
    let rows: Vec<ItemRow> = query_as(&sql)
        .bind(org.get())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    rows.into_iter().map(item).collect()
}

/// Whether a posted entry has a line that names the item.
pub async fn item_posted(db: &mut PgConnection, org: OrgId, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ledger.line l JOIN ledger.item i USING (org_id, item_id) \
         JOIN ledger.entry e USING (org_id, entry_id) WHERE l.org_id = $1 AND i.code = $2 AND e.posted_at IS NOT NULL)",
    )
    .bind(org.get())
    .bind(code)
    .fetch_one(db)
    .await
}

/// Adds the item, or changes the one with its code or, when it has one, its host reference; its unit and accounts must exist.
pub async fn save_item(db: &mut PgConnection, org: OrgId, saved: &Item) -> Result<(), sqlx::Error> {
    let key = if saved.host_ref.is_some() {
        "(org_id, host_ref)"
    } else {
        "(org_id, code)"
    };
    let account = |n: u8| format!("(SELECT account_id FROM ledger.account WHERE org_id = $1 AND code = ${n})");
    query(&format!(
        "INSERT INTO ledger.item (org_id, code, name, kind, unit_id, sales_account_id, purchase_account_id, \
         sales_tax, purchase_tax, weight_e4, volume_e4, host_ref) \
         VALUES ($1, $2, $3, $4, (SELECT unit_id FROM ledger.unit WHERE org_id = $1 AND code = $5), {}, {}, \
         $8, $9, $10, $11, $12) \
         ON CONFLICT {key} DO UPDATE SET code = excluded.code, name = excluded.name, kind = excluded.kind, \
         unit_id = excluded.unit_id, sales_account_id = excluded.sales_account_id, \
         purchase_account_id = excluded.purchase_account_id, sales_tax = excluded.sales_tax, \
         purchase_tax = excluded.purchase_tax, weight_e4 = excluded.weight_e4, volume_e4 = excluded.volume_e4",
        account(6),
        account(7)
    ))
    .bind(org.get())
    .bind(&saved.code)
    .bind(&saved.name)
    .bind(saved.kind.as_str())
    .bind(&saved.unit)
    .bind(saved.sales_account.as_deref())
    .bind(saved.purchase_account.as_deref())
    .bind(saved.sales_tax.as_deref())
    .bind(saved.purchase_tax.as_deref())
    .bind(saved.weight.map(|q| q.0))
    .bind(saved.volume.map(|q| q.0))
    .bind(saved.host_ref.as_deref())
    .execute(db)
    .await
    .map(drop)
}

/// Adds the unit, or renames the one with its code.
pub async fn save_unit(db: &mut PgConnection, org: OrgId, unit: &Unit) -> Result<(), sqlx::Error> {
    query(
        "INSERT INTO ledger.unit (org_id, code, name) VALUES ($1, $2, $3) \
         ON CONFLICT (org_id, code) DO UPDATE SET name = excluded.name",
    )
    .bind(org.get())
    .bind(&unit.code)
    .bind(&unit.name)
    .execute(db)
    .await
    .map(drop)
}

pub async fn unit_exists(db: &mut PgConnection, org: OrgId, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar("SELECT EXISTS (SELECT 1 FROM ledger.unit WHERE org_id = $1 AND code = $2)")
        .bind(org.get())
        .bind(code)
        .fetch_one(db)
        .await
}

/// Up to `limit` units by code, from just after `after`.
pub async fn units(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<Unit>, sqlx::Error> {
    let rows: Vec<(String, String)> = query_as(
        "SELECT code, name FROM ledger.unit WHERE org_id = $1 AND ($2::text IS NULL OR code > $2) ORDER BY code LIMIT $3",
    )
    .bind(org.get())
    .bind(after)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(code, name)| Unit { code, name }).collect())
}
