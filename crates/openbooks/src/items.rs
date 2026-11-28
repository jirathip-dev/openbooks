use std::sync::Arc;

use openbooks_core::{ItemKind, OrgId, Quantity, Refusal, Word};
use openbooks_db::audit::Record;
use openbooks_db::chart;
use openbooks_db::items::{self, Item, Unit};
use sqlx::PgConnection;

use crate::chart::{maybe, rows};
use crate::members::{PAGE, Packs, pack_of, paged};
use crate::openapi::{Json, map, text};
use crate::{Ask, Error, Field, Host, Openbooks, Operation, Page, Reply, Schema, Shape};

pub(crate) const UNITS: Operation = crate::read!("units", Word::Read, "Lists the company's units by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Unit>);

pub(crate) const SAVE_UNIT: Operation = crate::closed!("unit.save", Word::Approve, "Adds a unit to the company's list, or renames the one with its code.", {
    code: String = "The unit's code, unique in the company.",
    name: String = "Its name.",
} -> ());

pub(crate) const ITEMS: Operation = crate::read!("items", Word::Read, "Lists the catalogue's goods and services by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Item>);

pub(crate) const SAVE_ITEM: Operation = crate::closed!("item.save", Word::Approve, "Adds an item to the catalogue, or changes the one with its code unless the host keeps it; one on a posted line keeps its kind and unit.", {
    code: String = "The item's code, unique in the catalogue.",
    name: String = "Its name.",
    kind: ItemKind = "Whether it is stock, non-stock goods or a service.",
    unit: String = "The code of the one unit it is counted in.",
    sales_account: Option<String> = "The account a sale of it goes to by default.",
    purchase_account: Option<String> = "The account a purchase of it goes to by default.",
    sales_tax: Option<String> = "The tax code a sale of it carries by default.",
    purchase_tax: Option<String> = "The tax code a purchase of it carries by default.",
    weight: Option<Quantity> = "The weight of one unit in kilograms.",
    volume: Option<Quantity> = "The volume of one unit in litres.",
} -> ());

pub(crate) const SYNC_ITEM: Operation = crate::closed!("item.sync", Word::Approve, "Keeps an item in step with the host's own record, by its reference; one on a posted line keeps its kind and unit.", {
    host_ref: String = "The host's reference for the item.",
    code: String = "The item's code, unique in the catalogue.",
    name: String = "Its name.",
    kind: ItemKind = "Whether it is stock, non-stock goods or a service.",
    unit: String = "The code of the one unit it is counted in.",
    sales_account: Option<String> = "The account a sale of it goes to by default.",
    purchase_account: Option<String> = "The account a purchase of it goes to by default.",
    sales_tax: Option<String> = "The tax code a sale of it carries by default.",
    purchase_tax: Option<String> = "The tax code a purchase of it carries by default.",
    weight: Option<Quantity> = "The weight of one unit in kilograms.",
    volume: Option<Quantity> = "The volume of one unit in litres.",
} -> ());

fn field(name: &'static str, about: &'static str, schema: Schema) -> Field {
    Field { name, about, schema }
}

impl Shape for Unit {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("code", "The unit's code.", String::schema()),
            field("name", "Its name.", String::schema()),
        ])
    }
}

impl Shape for Item {
    fn schema() -> Schema {
        let maybe = Option::<String>::schema;
        Schema::Object(vec![
            field("code", "The item's code.", String::schema()),
            field("name", "Its name.", String::schema()),
            field(
                "kind",
                "Whether it is stock, non-stock goods or a service.",
                ItemKind::schema(),
            ),
            field("unit", "The code of the unit it is counted in.", String::schema()),
            field("sales_account", "The account a sale of it goes to by default.", maybe()),
            field(
                "purchase_account",
                "The account a purchase of it goes to by default.",
                maybe(),
            ),
            field("sales_tax", "The tax code a sale of it carries by default.", maybe()),
            field(
                "purchase_tax",
                "The tax code a purchase of it carries by default.",
                maybe(),
            ),
            field(
                "weight",
                "The weight of one unit in kilograms.",
                Option::<Quantity>::schema(),
            ),
            field(
                "volume",
                "The volume of one unit in litres.",
                Option::<Quantity>::schema(),
            ),
            field("host_ref", "The host's reference, when the host keeps it.", maybe()),
        ])
    }
}

async fn check_item(packs: &Packs, db: &mut PgConnection, org: OrgId, item: &Item) -> Result<(), Error> {
    if !items::unit_exists(db, org, &item.unit).await? {
        return Err(Error::NoSuchUnit(item.unit.clone()));
    }
    for code in [&item.sales_account, &item.purchase_account].into_iter().flatten() {
        if chart::account_by_code(db, org, code).await?.is_none() {
            return Err(Error::NoSuchAccount(code.clone()));
        }
    }
    let pack = pack_of(packs, db, org).await?;
    let tax = pack.tax();
    let known = |code: &str| tax.added().iter().chain(tax.withheld()).any(|k| k.code == code);
    for code in [&item.sales_tax, &item.purchase_tax].into_iter().flatten() {
        if !known(code) {
            return Err(Refusal::UnknownTaxCode.into());
        }
    }
    Ok(())
}

async fn keep_kind_and_unit(db: &mut PgConnection, org: OrgId, old: Option<Item>, new: &Item) -> Result<(), Error> {
    let Some(old) = old else { return Ok(()) };
    if (old.kind != new.kind || old.unit != new.unit) && items::item_posted(db, org, &old.code).await? {
        return Err(Error::ItemPosted(old.code));
    }
    Ok(())
}

impl<H: Host + 'static> Openbooks<H> {
    /// One page of the company's units by code, from just after the cursor `after`.
    pub async fn units(&self, org: OrgId, after: Option<String>) -> Result<Page<Unit>, Error> {
        self.read(org, UNITS, move |db| {
            Box::pin(async move {
                let items = items::units(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |u| u.code.clone()))
            })
        })
        .await
    }

    /// Adds the unit or renames the one with its code.
    pub async fn save_unit(&self, org: OrgId, unit: Unit) -> Result<(), Error> {
        let detail = format!("{} {}", unit.code, unit.name);
        let record = Record::Unit(unit.code.clone());
        self.write(org, SAVE_UNIT, record, detail, None, move |db| {
            Box::pin(async move { Ok(items::save_unit(db, org, &unit).await?) })
        })
        .await
    }

    /// One page of the catalogue by code, from just after the cursor `after`.
    pub async fn items(&self, org: OrgId, after: Option<String>) -> Result<Page<Item>, Error> {
        self.read(org, ITEMS, move |db| {
            Box::pin(async move {
                let items = items::items(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |i| i.code.clone()))
            })
        })
        .await
    }

    /// Adds the item or changes the one with its code; one the host keeps is refused.
    pub async fn save_item(&self, org: OrgId, item: Item) -> Result<(), Error> {
        let packs = Arc::clone(&self.packs);
        let detail = format!("{} {} {}", item.code, item.kind.as_str(), item.name);
        let record = Record::Item(item.code.clone());
        let item = Item { host_ref: None, ..item };
        self.write(org, SAVE_ITEM, record, detail, None, move |db| {
            Box::pin(async move {
                let old = items::item_by_code(db, org, &item.code).await?;
                if old.as_ref().is_some_and(|old| old.host_ref.is_some()) {
                    return Err(Error::Synced(item.code));
                }
                check_item(&packs, db, org, &item).await?;
                keep_kind_and_unit(db, org, old, &item).await?;
                Ok(items::save_item(db, org, &item).await?)
            })
        })
        .await
    }

    /// Keeps the item with the host's reference in step with the host's own record.
    pub async fn sync_item(&self, org: OrgId, host_ref: String, item: Item) -> Result<(), Error> {
        let packs = Arc::clone(&self.packs);
        let detail = format!("{host_ref} {} {} {}", item.code, item.kind.as_str(), item.name);
        let record = Record::SyncedItem(host_ref.clone());
        let item = Item {
            host_ref: Some(host_ref.clone()),
            ..item
        };
        self.write(org, SYNC_ITEM, record, detail, None, move |db| {
            Box::pin(async move {
                let old = items::item_by_host_ref(db, org, &host_ref).await?;
                check_item(&packs, db, org, &item).await?;
                keep_kind_and_unit(db, org, old, &item).await?;
                Ok(items::save_item(db, org, &item).await?)
            })
        })
        .await
    }
}

fn unit(u: &Unit) -> Json {
    map([("code", text(&u.code)), ("name", text(&u.name))])
}

fn quantity(json: Json, key: &str, value: Option<Quantity>) -> Json {
    match value {
        Some(q) => json.with(key, Json::Num(q.0)),
        None => json,
    }
}

fn item(i: &Item) -> Json {
    let mut json = map([
        ("code", text(&i.code)),
        ("name", text(&i.name)),
        ("kind", text(i.kind.as_str())),
        ("unit", text(&i.unit)),
    ]);
    for (key, value) in [
        ("sales_account", &i.sales_account),
        ("purchase_account", &i.purchase_account),
        ("sales_tax", &i.sales_tax),
        ("purchase_tax", &i.purchase_tax),
        ("host_ref", &i.host_ref),
    ] {
        json = maybe(json, key, value.as_ref());
    }
    quantity(quantity(json, "weight", i.weight), "volume", i.volume)
}

pub(crate) async fn serve(ask: Ask<'_>) -> Result<Reply, Error> {
    let (n, after, org) = (i64::from(ask.rows()), ask.after()?, ask.org);
    let after = after.as_deref();
    match ask.tool {
        "units" => Ok(rows(&items::units(ask.db, org, after, n).await?, unit, |u| {
            u.code.clone()
        })),
        "items" => Ok(rows(&items::items(ask.db, org, after, n).await?, item, |i| {
            i.code.clone()
        })),
        other => Err(Error::NoSuchTool(other.to_owned())),
    }
}
