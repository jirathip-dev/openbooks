use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use openbooks_core::{Currency, Email, IdempotencyKey, OrgId, Refusal, Role, UserId, Word};
use openbooks_db::SchemaError;
use openbooks_db::audit::{self, Audit, AuditRow, Filter, Record};
use openbooks_db::events::{self, Claim};
use openbooks_db::members::{self, Member};
use sqlx::{PgConnection, PgPool};

use crate::chart::{
    ACCOUNT_ROLES, ACCOUNTS, ADD_DIMENSION, DIMENSIONS, GIVE_ROLE, REMOVE_DIMENSION, RENAME_DIMENSION, REQUIRE,
    SAVE_ACCOUNT, SAVE_TERMS, SAVE_VALUE, SYNC_VALUE, TERMS, VALUES,
};
use crate::contacts::{CONTACTS, IDENTITY, SAVE_CONTACT, SET_BANK, SYNC_CONTACT};
use crate::openapi::{Json, map, text};
use crate::tools::{answer, listing};
use crate::{Ask, Error, Hooks, Host, Kind, Mount, NoPack, Operation, Pack, Page, Reply, Serve, SignedIn, Tools, Work};

pub(crate) type Packs = Arc<BTreeMap<&'static str, Arc<dyn Pack>>>;

/// The member, chart, dimension, terms and contact operations, mounted under `books`.
pub const BOOKS: Mount = Mount {
    key: "books",
    operations: &[
        MEMBERS,
        ADD_MEMBER,
        CHANGE_ROLE,
        REMOVE_MEMBER,
        AUDIT,
        ACCOUNTS,
        SAVE_ACCOUNT,
        REQUIRE,
        ACCOUNT_ROLES,
        GIVE_ROLE,
        DIMENSIONS,
        ADD_DIMENSION,
        RENAME_DIMENSION,
        REMOVE_DIMENSION,
        VALUES,
        SAVE_VALUE,
        SYNC_VALUE,
        TERMS,
        SAVE_TERMS,
        CONTACTS,
        SAVE_CONTACT,
        SYNC_CONTACT,
        IDENTITY,
        SET_BANK,
        TOOLS,
    ],
    serve: serve_books,
};

fn serve_books(ask: Ask<'_>) -> Work<'_, Reply> {
    Box::pin(async move {
        if ask.tool == CONTACTS.name() {
            return crate::contacts::serve(ask).await;
        }
        if ask.tool != MEMBERS.name() {
            return crate::chart::serve(ask).await;
        }
        let (limit, after) = (i64::from(ask.rows()), ask.after()?);
        let rows = members::members(ask.db, ask.org, after.as_deref(), limit).await?;
        Ok(Reply::Rows(
            rows.iter().map(|m| (member(m), m.email.as_str().to_owned())).collect(),
        ))
    })
}

pub(crate) const PAGE: u16 = 100;

const MEMBERS: Operation = crate::read!("members", Word::Read, "Lists the company's members by email, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Member>);

const ADD_MEMBER: Operation = crate::closed!("member.add", Word::Own, "Adds a person by email with a role.", {
    email: Email = "The address they will sign in with.",
    role: Role = "The role they get.",
} -> ())
.creates();

const CHANGE_ROLE: Operation = crate::closed!("member.role", Word::Own, "Gives a member another role; the last owner stays one.", {
    email: Email = "The member's address.",
    role: Role = "Their new role.",
} -> ());

const REMOVE_MEMBER: Operation = crate::closed!("member.remove", Word::Own, "Takes a member off the list; the last owner stays.", {
    email: Email = "The member's address.",
} -> ());

const AUDIT: Operation = crate::closed_read!("audit", Word::Approve, "Lists the company's audit rows, oldest first, a page at a time.", {
    record: Option<String> = "Only the rows on this record, by its key.",
    user: Option<UserId> = "Only the changes made for this person.",
    action: Option<String> = "Only this kind of change.",
    from: Option<SystemTime> = "Only the changes saved at or after this time.",
    until: Option<SystemTime> = "Only the changes saved before this time.",
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<AuditRow>);

const TOOLS: Operation = crate::closed_read!("tools", Word::Read, "Lists the tools offered to the caller, as an MCP tool list in JSON; a tool is called at its own route.", {} -> String);

/// The books, embedded in a host.
pub struct Openbooks<H> {
    host: Arc<H>,
    hooks: Arc<dyn Hooks>,
    tools: Arc<Tools>,
    pub(crate) packs: Packs,
}

/// Sets up the books for a host and its hooks; `build` hands them over.
pub struct Builder<H> {
    host: H,
    hooks: Arc<dyn Hooks>,
    tools: Tools,
    packs: BTreeMap<&'static str, Arc<dyn Pack>>,
}

impl<H: Host + 'static> Builder<H> {
    #[must_use]
    pub fn hooks(self, hooks: impl Hooks + 'static) -> Self {
        Self {
            hooks: Arc::new(hooks),
            ..self
        }
    }

    /// Replaces the tools built from `BOOKS` alone, as with a pack's mount or another switch.
    #[must_use]
    pub fn tools(self, tools: Tools) -> Self {
        Self { tools, ..self }
    }

    /// Offers a pack by its name to the companies of these books; `none` is offered already.
    #[must_use]
    pub fn pack(mut self, pack: impl Pack + 'static) -> Self {
        self.packs.insert(pack.name(), Arc::new(pack));
        self
    }

    pub fn build(self) -> Openbooks<H> {
        Openbooks {
            host: Arc::new(self.host),
            hooks: self.hooks,
            tools: Arc::new(self.tools),
            packs: Arc::new(self.packs),
        }
    }
}

impl<H: Host + 'static> Openbooks<H> {
    #[expect(
        clippy::new_ret_no_self,
        reason = "the books are built: Openbooks::new(host).build()"
    )]
    pub fn new(host: H) -> Builder<H> {
        let none: Arc<dyn Pack> = Arc::new(NoPack);
        Builder {
            host,
            hooks: Arc::new(()),
            tools: Tools::new(&[BOOKS]),
            packs: BTreeMap::from([(none.name(), none)]),
        }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    /// Applies the core's migrations, then each offered pack's in its own schema.
    pub async fn migrate(&self, pool: &PgPool) -> Result<(), SchemaError> {
        openbooks_db::migrate(pool).await?;
        for pack in self.packs.values() {
            openbooks_db::migrate_pack(pool, pack.name(), pack.migrations()).await?;
        }
        Ok(())
    }

    /// Opens a company under one offered pack with the person signed in as its first Owner; sent again with its key, it opens one.
    pub async fn create_company(
        &self,
        name: &str,
        base: Currency,
        pack: &str,
        key: Option<IdempotencyKey>,
    ) -> Result<OrgId, Error> {
        let call = self.call()?;
        let pack = self
            .packs
            .get(pack)
            .ok_or_else(|| Error::NoSuchPack(pack.to_owned()))?
            .name();
        let (name, request) = (name.to_owned(), format!("company.create {base} {pack} {name}"));
        let org = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    if let Some(org) = call.replay(db, key.as_ref(), &request).await? {
                        return Ok(org);
                    }
                    let org = members::create_org(db, &name, base, pack).await?;
                    members::add(db, org, &call.signed.email, Role::Owner).await?;
                    members::tie(db, org, &call.signed.email, call.signed.user).await?;
                    call.record(db, org, "company.create", &name, &Record::Company, None)
                        .await?;
                    call.remember(db, key.as_ref(), org.store()).await?;
                    Ok(org)
                })
            })
            .await?;
        self.deliver(org).await.ok();
        Ok(org)
    }

    /// One page of the company's members by email, from just after the cursor `after`.
    pub async fn members(&self, org: OrgId, after: Option<String>) -> Result<Page<Member>, Error> {
        self.read(org, MEMBERS, move |db| {
            Box::pin(async move {
                let items = members::members(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |m| m.email.as_str().to_owned()))
            })
        })
        .await
    }

    /// The tools offered to the person signed in, as the MCP tool list the local and remote servers serve.
    pub async fn tools(&self, org: OrgId) -> Result<String, Error> {
        Ok(listing(&self.offered(org).await?))
    }

    /// Calls an offered tool with its arguments by name through its mount's serve, answered in the agent view: 50 rows, a size cap, free text scrubbed.
    pub async fn call_tool(&self, org: OrgId, tool: &str, arguments: &[(&str, &str)]) -> Result<String, Error> {
        let offered = self.offered(org).await?;
        let op = offered.into_iter().find(|op| op.name() == tool);
        let found = op.and_then(|op| Some((op, self.tools.serve(&op)?)));
        let (op, serve) = found.ok_or_else(|| Error::NoSuchTool(tool.to_owned()))?;
        let arguments: Vec<(String, String)> = arguments
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let detail = arguments
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let work = served(serve, org, op, arguments);
        match op.kind() {
            Kind::Write => self.write(org, op, Record::Company, detail, None, work).await,
            Kind::Read | Kind::ClosedRead | Kind::Closed => self.read(org, op, work).await,
        }
    }

    async fn offered(&self, org: OrgId) -> Result<Vec<Operation>, Error> {
        let (tools, signed) = (Arc::clone(&self.tools), self.call()?.signed);
        self.read(org, TOOLS, move |db| {
            Box::pin(async move {
                let member = members::member_by_user(db, org, signed.user).await?;
                Ok(member.map_or_else(Vec::new, |m| tools.offered(m.role, &signed)))
            })
        })
        .await
    }

    /// One page of the company's audit rows the filter keeps, from just after the cursor `after`; Owners and Accountants only.
    pub async fn audit(&self, org: OrgId, filter: Filter, after: Option<String>) -> Result<Page<AuditRow>, Error> {
        let after = after
            .map(|a| a.parse::<i64>())
            .transpose()
            .map_err(|_| Refusal::BadId)?;
        self.read(org, AUDIT, move |db| {
            Box::pin(async move {
                let items = audit::list(db, org, &filter, after, i64::from(PAGE) + 1).await?;
                Ok(paged(items, |row| row.seq.to_string()))
            })
        })
        .await
    }

    /// Adds a person by email with a role; sent again with its key, it adds them once.
    pub async fn add_member(
        &self,
        org: OrgId,
        email: Email,
        role: Role,
        key: Option<IdempotencyKey>,
    ) -> Result<(), Error> {
        let detail = format!("{} as {}", email.as_str(), role.as_str());
        let record = Record::Member(email.clone());
        self.write(org, ADD_MEMBER, record, detail, key, move |db| {
            Box::pin(async move {
                members::lock_org(db, org).await?;
                if members::add(db, org, &email, role).await? {
                    Ok(())
                } else {
                    Err(Error::AlreadyMember(email))
                }
            })
        })
        .await
    }

    pub async fn change_role(&self, org: OrgId, email: Email, role: Role) -> Result<(), Error> {
        let detail = format!("{} to {}", email.as_str(), role.as_str());
        let record = Record::Member(email.clone());
        self.write(org, CHANGE_ROLE, record, detail, None, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, role != Role::Owner).await?;
                Ok(members::set_role(db, org, &email, role).await?)
            })
        })
        .await
    }

    pub async fn remove_member(&self, org: OrgId, email: Email) -> Result<(), Error> {
        let detail = email.as_str().to_owned();
        let record = Record::Member(email.clone());
        self.write(org, REMOVE_MEMBER, record, detail, None, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, true).await?;
                Ok(members::remove(db, org, &email).await?)
            })
        })
        .await
    }

    /// Hands the company's undelivered events to `after_commit` in order; a failed one waits, with those after it, for the next call.
    pub async fn deliver(&self, org: OrgId) -> Result<(), Error> {
        let (hooks, packs) = (Arc::clone(&self.hooks), Arc::clone(&self.packs));
        let failed = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    let pending = events::pending(db, org).await?;
                    if pending.is_empty() {
                        return Ok(None);
                    }
                    let pack = pack_of(&packs, db, org).await?;
                    for event in pending {
                        let delivered = match pack.after_commit(&event).await {
                            Ok(()) => hooks.after_commit(&event).await,
                            Err(error) => Err(error),
                        };
                        if let Err(error) = delivered {
                            return Ok(Some(error));
                        }
                        events::delivered(db, org, event.id).await?;
                    }
                    Ok(None)
                })
            })
            .await?;
        failed.map_or(Ok(()), Err)
    }

    fn call(&self) -> Result<Call<H>, Error> {
        Ok(Call {
            host: Arc::clone(&self.host),
            hooks: Arc::clone(&self.hooks),
            packs: Arc::clone(&self.packs),
            signed: self.host.who().ok_or(Error::SignedOut)?,
        })
    }

    pub(crate) async fn read<T, F>(&self, org: OrgId, op: Operation, work: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, T> + Send + 'static,
    {
        let call = self.call()?;
        let out = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    call.admit(db, org, op).await?;
                    work(&mut *db).await
                })
            })
            .await?;
        self.deliver(org).await.ok();
        Ok(out)
    }

    pub(crate) async fn write<T, F>(
        &self,
        org: OrgId,
        op: Operation,
        record: Record,
        detail: String,
        key: Option<IdempotencyKey>,
        work: F,
    ) -> Result<T, Error>
    where
        T: Answer + Send + 'static,
        F: for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, T> + Send + 'static,
    {
        let call = self.call()?;
        let request = format!("{} {} {detail}", op.name(), org.get());
        let out = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    call.admit(db, org, op).await?;
                    if let Some(out) = call.replay(db, key.as_ref(), &request).await? {
                        return Ok(out);
                    }
                    members::lock_org(db, org).await?;
                    let before = audit::snapshot(db, org, &record).await?;
                    let out = work(&mut *db).await?;
                    call.record(db, org, op.name(), &detail, &record, before).await?;
                    call.remember(db, key.as_ref(), out.store()).await?;
                    Ok(out)
                })
            })
            .await?;
        self.deliver(org).await.ok();
        Ok(out)
    }
}

fn served(
    serve: Serve,
    org: OrgId,
    op: Operation,
    arguments: Vec<(String, String)>,
) -> impl for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, String> + Send + 'static {
    move |db| {
        Box::pin(async move {
            let ask = Ask {
                db,
                org,
                tool: op.name(),
                arguments: &arguments,
            };
            Ok(answer(serve(ask).await?))
        })
    }
}

fn member(m: &Member) -> Json {
    let json = map([("email", text(m.email.as_str())), ("role", text(m.role.as_str()))]);
    match m.user {
        Some(user) => json.with("user", Json::Num(user.get())),
        None => json,
    }
}

pub(crate) fn paged<T>(mut items: Vec<T>, cursor: impl Fn(&T) -> String) -> Page<T> {
    let more = items.len() > usize::from(PAGE);
    items.truncate(usize::from(PAGE));
    let next = more.then(|| items.last().map(cursor)).flatten();
    Page { items, next }
}

pub(crate) trait Answer: Sized {
    fn store(&self) -> Option<String>;

    fn load(stored: Option<&str>) -> Result<Self, Error>;
}

impl Answer for () {
    fn store(&self) -> Option<String> {
        None
    }

    fn load(_: Option<&str>) -> Result<Self, Error> {
        Ok(())
    }
}

impl Answer for String {
    fn store(&self) -> Option<String> {
        Some(self.clone())
    }

    fn load(stored: Option<&str>) -> Result<Self, Error> {
        Ok(stored.unwrap_or_default().to_owned())
    }
}

impl Answer for OrgId {
    fn store(&self) -> Option<String> {
        Some(self.get().to_string())
    }

    fn load(stored: Option<&str>) -> Result<Self, Error> {
        let id = stored.and_then(|s| s.parse().ok()).ok_or(Refusal::BadId)?;
        Ok(OrgId::new(id)?)
    }
}

struct Call<H> {
    host: Arc<H>,
    hooks: Arc<dyn Hooks>,
    packs: Packs,
    signed: SignedIn,
}

impl<H: Host> Call<H> {
    async fn admit(&self, db: &mut PgConnection, org: OrgId, op: Operation) -> Result<(), Error> {
        let member = self.caller(db, org).await?;
        if member.role.allows(op.word()) && self.signed.keeps(op.word()) {
            Ok(())
        } else {
            Err(Error::Forbidden { word: op.word() })
        }
    }

    async fn caller(&self, db: &mut PgConnection, org: OrgId) -> Result<Member, Error> {
        let signed = &self.signed;
        if let Some(member) = members::member_by_user(db, org, signed.user).await? {
            return Ok(member);
        }
        let record = Record::Member(signed.email.clone());
        let before = audit::snapshot(db, org, &record).await?;
        if let Some(member) = members::tie(db, org, &signed.email, signed.user).await? {
            let detail = signed.email.as_str();
            self.record(db, org, "member.join", detail, &record, before).await?;
            return Ok(member);
        }
        let name = self.host.names(&[signed.user]).remove(&signed.user);
        Err(Error::NotMember {
            name: name.unwrap_or_else(|| signed.email.as_str().to_owned()),
        })
    }

    async fn replay<T: Answer>(
        &self,
        db: &mut PgConnection,
        key: Option<&IdempotencyKey>,
        request: &str,
    ) -> Result<Option<T>, Error> {
        let Some(key) = key else { return Ok(None) };
        match events::claim(db, self.signed.user, key, request).await? {
            Claim::New => Ok(None),
            Claim::Answered(answer) => T::load(answer.as_deref()).map(Some),
            Claim::Reused => Err(Error::KeyReused),
        }
    }

    async fn remember(
        &self,
        db: &mut PgConnection,
        key: Option<&IdempotencyKey>,
        out: Option<String>,
    ) -> Result<(), Error> {
        let Some(key) = key else { return Ok(()) };
        Ok(events::answer(db, self.signed.user, key, out.as_deref()).await?)
    }

    async fn record(
        &self,
        db: &mut PgConnection,
        org: OrgId,
        action: &str,
        detail: &str,
        record: &Record,
        before: Option<String>,
    ) -> Result<(), Error> {
        members::lock_org(db, org).await?;
        let id = self.host.new_id();
        let row = Audit {
            id: &id,
            at: self.host.now(),
            user: self.signed.user,
            via: self.signed.via.as_str(),
            action,
            detail,
            record,
            before: before.as_deref(),
        };
        audit::write(db, org, row).await?;
        let event = events::write(db, org, &id, action, detail, self.signed.via.origin()).await?;
        self.hooks.before_commit(db, &event).await?;
        pack_of(&self.packs, db, org).await?.before_commit(db, &event).await
    }
}

pub(crate) async fn pack_of(packs: &Packs, db: &mut PgConnection, org: OrgId) -> Result<Arc<dyn Pack>, Error> {
    let name = members::org_pack(db, org).await?.ok_or(Refusal::BadId)?;
    packs.get(name.as_str()).cloned().ok_or(Error::NoSuchPack(name))
}

async fn keep_an_owner(db: &mut PgConnection, org: OrgId, email: &Email, leaves: bool) -> Result<(), Error> {
    members::lock_org(db, org).await?;
    let member = members::member_by_email(db, org, email).await?;
    let member = member.ok_or_else(|| Error::NoSuchMember(email.clone()))?;
    if leaves && member.role == Role::Owner && members::owners(db, org).await? <= 1 {
        return Err(Error::LastOwner);
    }
    Ok(())
}
