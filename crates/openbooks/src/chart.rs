use std::sync::Arc;

use openbooks_core::{AccountType, Currency, Date, IdempotencyKey, OrgId, Refusal, Word};
use openbooks_db::audit::Record;
use openbooks_db::chart::{self, Account, Dimension, DimensionValue, RoleAccount, Terms};

use crate::members::{PAGE, pack_of, paged};
use crate::openapi::{Json, map, text};
use crate::{AccountRole, Ask, Error, Field, Host, Openbooks, Operation, Page, Reply, Schema, Shape};

/// The roles the core's own postings look an account up by; a pack adds its own.
pub const CORE_ROLES: &[AccountRole] = &[
    AccountRole {
        key: "receivables",
        about: "The control account of what customers owe.",
    },
    AccountRole {
        key: "payables",
        about: "The control account of what is owed to suppliers.",
    },
    AccountRole {
        key: "retained_earnings",
        about: "Where a closed year's result is carried.",
    },
    AccountRole {
        key: "rounding",
        about: "Where the minor units a rounding leaves go.",
    },
];

pub(crate) const ACCOUNTS: Operation = crate::read!("accounts", Word::Read, "Lists the chart's accounts by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Account>);

pub(crate) const SAVE_ACCOUNT: Operation = crate::closed!("account.save", Word::Approve, "Adds an account to the chart, or changes the one with its code; one with postings keeps its type.", {
    code: String = "The account's code, unique in the chart.",
    name: String = "Its name.",
    account_type: AccountType = "What it holds.",
    parent: Option<String> = "The code of the account it sits under; absent at the top.",
    active: bool = "Whether new lines may use it.",
    currency: Option<String> = "The one currency it is kept in, by code; absent for any.",
} -> ());

pub(crate) const REQUIRE: Operation = crate::closed!("account.require", Word::Approve, "Says whether each line on an account needs a value of a dimension kind.", {
    account: String = "The account's code.",
    slot: u16 = "The kind's slot, 1 to 3.",
    required: bool = "Whether a line needs a value of it.",
} -> ());

pub(crate) const ACCOUNT_ROLES: Operation = crate::read!("account_roles", Word::Read, "Lists which account plays each role on a day.", {
    on: Date = "The day.",
} -> Page<RoleAccount>);

pub(crate) const GIVE_ROLE: Operation = crate::closed!("account.role", Word::Approve, "Gives a role the core or the company's pack declares to an account from a day on.", {
    role: String = "The role's key.",
    from: Date = "The first day the account plays it.",
    account: String = "The account's code.",
} -> ());

pub(crate) const DIMENSIONS: Operation = crate::read!("dimensions", Word::Read, "Lists the company's dimension kinds by slot: Project first, then up to two more.", {} -> Page<Dimension>);

pub(crate) const ADD_DIMENSION: Operation = crate::closed!("dimension.add", Word::Approve, "Turns on a dimension kind in the next free slot; a company has at most three.", {
    name: String = "Its name: Cost centre, Department, or one of the company's own.",
} -> ())
.creates();

pub(crate) const RENAME_DIMENSION: Operation = crate::closed!("dimension.rename", Word::Approve, "Renames a dimension kind the company turned on.", {
    slot: u16 = "The kind's slot, 2 or 3.",
    name: String = "Its new name.",
} -> ());

pub(crate) const REMOVE_DIMENSION: Operation = crate::closed!("dimension.remove", Word::Approve, "Turns off a dimension kind no posted line carries a value of.", {
    slot: u16 = "The kind's slot, 2 or 3.",
} -> ());

pub(crate) const VALUES: Operation = crate::read!("dimension_values", Word::Read, "Lists a dimension kind's values by code, a page at a time.", {
    slot: u16 = "The kind's slot, 1 to 3.",
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<DimensionValue>);

pub(crate) const SAVE_VALUE: Operation = crate::closed!("dimension_value.save", Word::Approve, "Adds a value to a dimension kind, or changes the one with its code unless the host keeps it.", {
    slot: u16 = "The kind's slot, 1 to 3.",
    code: String = "The value's code, unique in its kind.",
    name: String = "Its name.",
    active: bool = "Whether new lines may carry it.",
} -> ());

pub(crate) const SYNC_VALUE: Operation = crate::closed!("dimension_value.sync", Word::Approve, "Keeps a value of a dimension kind in step with the host's own record, by its reference.", {
    slot: u16 = "The kind's slot, 1 to 3.",
    host_ref: String = "The host's reference for the value.",
    code: String = "The value's code, unique in its kind.",
    name: String = "Its name.",
    active: bool = "Whether new lines may carry it.",
} -> ());

pub(crate) const TERMS: Operation = crate::read!("terms", Word::Read, "Lists the company's payment terms by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Terms>);

pub(crate) const SAVE_TERMS: Operation = crate::closed!("terms.save", Word::Approve, "Adds a payment term, or changes the one with its code.", {
    code: String = "The term's code.",
    name: String = "Its name.",
    days: u16 = "How many days after the document's date it is due.",
    active: bool = "Whether new documents may use it.",
} -> ());

fn field(name: &'static str, about: &'static str, schema: Schema) -> Field {
    Field { name, about, schema }
}

impl Shape for Account {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("code", "The account's code.", String::schema()),
            field("name", "Its name.", String::schema()),
            field("account_type", "What it holds.", AccountType::schema()),
            field(
                "parent",
                "The code of the account it sits under.",
                Option::<String>::schema(),
            ),
            field("active", "Whether new lines may use it.", bool::schema()),
            field(
                "currency",
                "The one currency it is kept in.",
                Option::<String>::schema(),
            ),
        ])
    }
}

impl Shape for RoleAccount {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("role", "The role's key.", String::schema()),
            field("from", "The day the account took it on.", Date::schema()),
            field("account", "The account's code.", String::schema()),
        ])
    }
}

impl Shape for Dimension {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("slot", "The kind's slot, 1 to 3.", u16::schema()),
            field("name", "Its name.", String::schema()),
        ])
    }
}

impl Shape for DimensionValue {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("code", "The value's code.", String::schema()),
            field("name", "Its name.", String::schema()),
            field("active", "Whether new lines may carry it.", bool::schema()),
            field(
                "host_ref",
                "The host's reference, when the host keeps it.",
                Option::<String>::schema(),
            ),
        ])
    }
}

impl Shape for Terms {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("code", "The term's code.", String::schema()),
            field("name", "Its name.", String::schema()),
            field(
                "days",
                "How many days after the document's date it is due.",
                u16::schema(),
            ),
            field("active", "Whether new documents may use it.", bool::schema()),
        ])
    }
}

fn any_slot(slot: u16) -> Result<u16, Error> {
    if (1..=3).contains(&slot) {
        Ok(slot)
    } else {
        Err(Error::NoSuchDimension(slot))
    }
}

fn own_slot(slot: u16) -> Result<u16, Error> {
    match slot {
        1 => Err(Error::ProjectDimension),
        2 | 3 => Ok(slot),
        _ => Err(Error::NoSuchDimension(slot)),
    }
}

impl<H: Host + 'static> Openbooks<H> {
    /// One page of the chart by code, from just after the cursor `after`.
    pub async fn accounts(&self, org: OrgId, after: Option<String>) -> Result<Page<Account>, Error> {
        self.read(org, ACCOUNTS, move |db| {
            Box::pin(async move {
                let items = chart::accounts(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |a| a.code.clone()))
            })
        })
        .await
    }

    /// Adds the account or changes the one with its code; an account with postings keeps its type.
    pub async fn save_account(&self, org: OrgId, account: Account) -> Result<(), Error> {
        if let Some(code) = &account.currency {
            Currency::new(code, 0)?;
        }
        let detail = format!("{} {} {}", account.code, account.account_type.as_str(), account.name);
        let record = Record::Account(account.code.clone());
        self.write(org, SAVE_ACCOUNT, record, detail, None, move |db| {
            Box::pin(async move {
                let code = account.code.as_str();
                if let Some(parent) = &account.parent {
                    if chart::account_by_code(db, org, parent).await?.is_none() {
                        return Err(Error::NoSuchAccount(parent.clone()));
                    }
                    if chart::above_or_is(db, org, parent, code).await? {
                        return Err(Error::ParentLoop(code.to_owned()));
                    }
                }
                let old = chart::account_by_code(db, org, code).await?;
                if old.is_some_and(|old| old.account_type != account.account_type)
                    && chart::posted_to(db, org, code).await?
                {
                    return Err(Error::AccountPosted(code.to_owned()));
                }
                Ok(chart::save_account(db, org, &account).await?)
            })
        })
        .await
    }

    /// Says whether each line on the account needs a value of the kind in `slot`.
    pub async fn require_dimension(&self, org: OrgId, account: String, slot: u16, required: bool) -> Result<(), Error> {
        let slot = any_slot(slot)?;
        let detail = format!(
            "{account} slot {slot} {}",
            if required { "required" } else { "optional" }
        );
        let record = Record::Account(account.clone());
        self.write(org, REQUIRE, record, detail, None, move |db| {
            Box::pin(async move {
                if chart::account_by_code(db, org, &account).await?.is_none() {
                    return Err(Error::NoSuchAccount(account));
                }
                if chart::require(db, org, &account, slot, required).await? {
                    Ok(())
                } else {
                    Err(Error::NoSuchDimension(slot))
                }
            })
        })
        .await
    }

    /// Which account plays each role on the day `on`.
    pub async fn account_roles(&self, org: OrgId, on: Date) -> Result<Page<RoleAccount>, Error> {
        self.read(org, ACCOUNT_ROLES, move |db| {
            Box::pin(async move {
                let items = chart::roles_on(db, org, on).await?;
                Ok(Page { items, next: None })
            })
        })
        .await
    }

    /// Gives the account a role the core or the company's pack declares, from the day `from` on.
    pub async fn give_role(&self, org: OrgId, role: String, from: Date, account: String) -> Result<(), Error> {
        let packs = Arc::clone(&self.packs);
        let detail = format!("{role} to {account}");
        let record = Record::AccountRole(role.clone());
        self.write(org, GIVE_ROLE, record, detail, None, move |db| {
            Box::pin(async move {
                let pack = pack_of(&packs, db, org).await?;
                if !CORE_ROLES.iter().chain(pack.account_roles()).any(|r| r.key == role) {
                    return Err(Error::NoSuchAccountRole(role));
                }
                if chart::give_role(db, org, &role, from, &account).await? {
                    Ok(())
                } else {
                    Err(Error::NoSuchAccount(account))
                }
            })
        })
        .await
    }

    /// The company's dimension kinds by slot.
    pub async fn dimensions(&self, org: OrgId) -> Result<Page<Dimension>, Error> {
        self.read(org, DIMENSIONS, move |db| {
            Box::pin(async move {
                let items = chart::dimensions(db, org).await?;
                Ok(Page { items, next: None })
            })
        })
        .await
    }

    /// Turns on a dimension kind in the next free slot; sent again with its key, it turns on one.
    pub async fn add_dimension(&self, org: OrgId, name: String, key: Option<IdempotencyKey>) -> Result<(), Error> {
        let detail = name.clone();
        self.write(org, ADD_DIMENSION, Record::Dimensions, detail, key, move |db| {
            Box::pin(async move {
                let added = chart::add_dimension(db, org, &name).await?;
                added.map(drop).ok_or(Error::DimensionsFull)
            })
        })
        .await
    }

    pub async fn rename_dimension(&self, org: OrgId, slot: u16, name: String) -> Result<(), Error> {
        let slot = own_slot(slot)?;
        let detail = format!("slot {slot} to {name}");
        self.write(org, RENAME_DIMENSION, Record::Dimensions, detail, None, move |db| {
            Box::pin(async move {
                if chart::change_dimension(db, org, slot, &name, false).await? {
                    Ok(())
                } else {
                    Err(Error::NoSuchDimension(slot))
                }
            })
        })
        .await
    }

    /// Turns off a dimension kind; one whose values are on posted lines can only be renamed.
    pub async fn remove_dimension(&self, org: OrgId, slot: u16) -> Result<(), Error> {
        let slot = own_slot(slot)?;
        let detail = format!("slot {slot}");
        self.write(org, REMOVE_DIMENSION, Record::Dimensions, detail, None, move |db| {
            Box::pin(async move {
                if chart::dimension_posted(db, org, slot).await? {
                    return Err(Error::DimensionPosted(slot));
                }
                let kind = chart::dimensions(db, org).await?.into_iter().find(|k| k.slot == slot);
                let kind = kind.ok_or(Error::NoSuchDimension(slot))?;
                chart::change_dimension(db, org, slot, &kind.name, true).await?;
                Ok(())
            })
        })
        .await
    }

    /// One page of the values of the kind in `slot` by code, from just after the cursor `after`.
    pub async fn dimension_values(
        &self,
        org: OrgId,
        slot: u16,
        after: Option<String>,
    ) -> Result<Page<DimensionValue>, Error> {
        let slot = any_slot(slot)?;
        self.read(org, VALUES, move |db| {
            Box::pin(async move {
                let items = chart::values(db, org, slot, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |v| v.code.clone()))
            })
        })
        .await
    }

    /// Adds a value to the kind in `slot`, or changes the one with its code; one the host keeps is refused.
    pub async fn save_value(
        &self,
        org: OrgId,
        slot: u16,
        code: String,
        name: String,
        active: bool,
    ) -> Result<(), Error> {
        let slot = any_slot(slot)?;
        let detail = format!("{code} {name}");
        let record = Record::Value(slot, code.clone());
        let value = DimensionValue {
            code,
            name,
            active,
            host_ref: None,
        };
        self.write(org, SAVE_VALUE, record, detail, None, move |db| {
            Box::pin(async move {
                let old = chart::value_by_code(db, org, slot, &value.code).await?;
                if old.is_some_and(|old| old.host_ref.is_some()) {
                    return Err(Error::Synced(value.code));
                }
                if chart::save_value(db, org, slot, &value).await? {
                    Ok(())
                } else {
                    Err(Error::NoSuchDimension(slot))
                }
            })
        })
        .await
    }

    /// Keeps the value with the host's reference in step with the host's own record.
    pub async fn sync_value(
        &self,
        org: OrgId,
        slot: u16,
        host_ref: String,
        value: DimensionValue,
    ) -> Result<(), Error> {
        let slot = any_slot(slot)?;
        let detail = format!("{host_ref} {} {}", value.code, value.name);
        let record = Record::Synced(slot, host_ref.clone());
        let value = DimensionValue {
            host_ref: Some(host_ref),
            ..value
        };
        self.write(org, SYNC_VALUE, record, detail, None, move |db| {
            Box::pin(async move {
                if chart::save_value(db, org, slot, &value).await? {
                    Ok(())
                } else {
                    Err(Error::NoSuchDimension(slot))
                }
            })
        })
        .await
    }

    /// One page of the payment terms by code, from just after the cursor `after`.
    pub async fn terms(&self, org: OrgId, after: Option<String>) -> Result<Page<Terms>, Error> {
        self.read(org, TERMS, move |db| {
            Box::pin(async move {
                let items = chart::terms(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |t| t.code.clone()))
            })
        })
        .await
    }

    /// Adds the payment term or changes the one with its code.
    pub async fn save_terms(&self, org: OrgId, terms: Terms) -> Result<(), Error> {
        let detail = format!("{} {} days", terms.code, terms.days);
        let record = Record::Terms(terms.code.clone());
        self.write(org, SAVE_TERMS, record, detail, None, move |db| {
            Box::pin(async move { Ok(chart::save_terms(db, org, &terms).await?) })
        })
        .await
    }
}

pub(crate) async fn serve(ask: Ask<'_>) -> Result<Reply, Error> {
    let (n, after, org) = (i64::from(ask.rows()), ask.after()?, ask.org);
    let after = after.as_deref();
    let slot = ask.argument("slot").map(whole).transpose()?;
    let on = ask.argument("on").map(day).transpose()?;
    let reply = match ask.tool {
        "accounts" => rows(&chart::accounts(ask.db, org, after, n).await?, account, |a| {
            a.code.clone()
        }),
        "terms" => rows(&chart::terms(ask.db, org, after, n).await?, terms, |t| t.code.clone()),
        "dimension_values" => {
            let slot = any_slot(slot.ok_or(Refusal::BadId)?)?;
            rows(&chart::values(ask.db, org, slot, after, n).await?, value, |v| {
                v.code.clone()
            })
        }
        "dimensions" => Reply::One(Json::List(
            chart::dimensions(ask.db, org).await?.iter().map(dimension).collect(),
        )),
        "account_roles" => {
            let roles = chart::roles_on(ask.db, org, on.ok_or(Refusal::BadDate)?).await?;
            Reply::One(Json::List(roles.iter().map(role).collect()))
        }
        _ => return crate::items::serve(ask).await,
    };
    Ok(reply)
}

pub(crate) fn rows<T>(items: &[T], json: fn(&T) -> Json, cursor: fn(&T) -> String) -> Reply {
    Reply::Rows(items.iter().map(|i| (json(i), cursor(i))).collect())
}

fn whole(value: &str) -> Result<u16, Error> {
    Ok(value.parse().map_err(|_| Refusal::BadId)?)
}

fn day(value: &str) -> Result<Date, Error> {
    let parts: Vec<u16> = value
        .split('-')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| Refusal::BadDate)?;
    let part = |n: u16| u8::try_from(n).map_err(|_| Refusal::BadDate);
    match parts[..] {
        [year, month, day] => Ok(Date::new(year, part(month)?, part(day)?)?),
        _ => Err(Refusal::BadDate.into()),
    }
}

fn date(d: Date) -> Json {
    let month = d.month();
    text(&format!("{:04}-{:02}-{:02}", month.year(), month.month(), d.day()))
}

pub(crate) fn maybe(json: Json, key: &str, value: Option<&String>) -> Json {
    match value {
        Some(value) => json.with(key, text(value)),
        None => json,
    }
}

fn account(a: &Account) -> Json {
    let json = map([
        ("code", text(&a.code)),
        ("name", text(&a.name)),
        ("account_type", text(a.account_type.as_str())),
        ("active", Json::Bool(a.active)),
    ]);
    maybe(
        maybe(json, "parent", a.parent.as_ref()),
        "currency",
        a.currency.as_ref(),
    )
}

fn role(r: &RoleAccount) -> Json {
    map([
        ("role", text(&r.role)),
        ("from", date(r.from)),
        ("account", text(&r.account)),
    ])
}

fn dimension(d: &Dimension) -> Json {
    map([("slot", Json::Num(i64::from(d.slot))), ("name", text(&d.name))])
}

fn value(v: &DimensionValue) -> Json {
    let json = map([
        ("code", text(&v.code)),
        ("name", text(&v.name)),
        ("active", Json::Bool(v.active)),
    ]);
    maybe(json, "host_ref", v.host_ref.as_ref())
}

fn terms(t: &Terms) -> Json {
    map([
        ("code", text(&t.code)),
        ("name", text(&t.name)),
        ("days", Json::Num(i64::from(t.days))),
        ("active", Json::Bool(t.active)),
    ])
}
