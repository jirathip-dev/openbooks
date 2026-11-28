use std::sync::Arc;
use std::time::UNIX_EPOCH;

use openbooks_core::{Currency, Email, IdempotencyKey, OrgId, Refusal, Role, Word};
use openbooks_db::events::{self, Claim};
use openbooks_db::members::{self, Audit, Member};
use sqlx::PgConnection;

use crate::{Error, Hooks, Host, Mount, Operation, Page, SignedIn, Work};

/// The member operations, mounted under `books`.
pub const BOOKS: Mount = Mount {
    key: "books",
    operations: &[MEMBERS, ADD_MEMBER, CHANGE_ROLE, REMOVE_MEMBER],
};

const PAGE: u16 = 100;

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

/// The books, embedded in a host.
pub struct Openbooks<H> {
    host: Arc<H>,
    hooks: Arc<dyn Hooks>,
}

/// Sets up the books for a host and its hooks; `build` hands them over.
pub struct Builder<H> {
    host: H,
    hooks: Arc<dyn Hooks>,
}

impl<H: Host + 'static> Builder<H> {
    #[must_use]
    pub fn hooks(self, hooks: impl Hooks + 'static) -> Self {
        Self {
            hooks: Arc::new(hooks),
            ..self
        }
    }

    pub fn build(self) -> Openbooks<H> {
        Openbooks {
            host: Arc::new(self.host),
            hooks: self.hooks,
        }
    }
}

impl<H: Host + 'static> Openbooks<H> {
    #[expect(
        clippy::new_ret_no_self,
        reason = "the books are built: Openbooks::new(host).build()"
    )]
    pub fn new(host: H) -> Builder<H> {
        Builder {
            host,
            hooks: Arc::new(()),
        }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    /// Opens a company with the person signed in as its first Owner; sent again with its key, it opens one.
    pub async fn create_company(
        &self,
        name: &str,
        base: Currency,
        key: Option<IdempotencyKey>,
    ) -> Result<OrgId, Error> {
        let call = self.call()?;
        let (name, request) = (name.to_owned(), format!("company.create {base} {name}"));
        let org = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    if let Some(org) = call.replay(db, key.as_ref(), &request).await? {
                        return Ok(org);
                    }
                    let org = members::create_org(db, &name, base).await?;
                    members::add(db, org, &call.signed.email, Role::Owner).await?;
                    members::tie(db, org, &call.signed.email, call.signed.user).await?;
                    call.record(db, org, "company.create", &name).await?;
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
        let call = self.call()?;
        let page = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    call.admit(db, org, MEMBERS).await?;
                    let mut items = members::members(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                    let more = items.len() > usize::from(PAGE);
                    items.truncate(usize::from(PAGE));
                    let next = more
                        .then(|| items.last().map(|m| m.email.as_str().to_owned()))
                        .flatten();
                    Ok(Page { items, next })
                })
            })
            .await?;
        self.deliver(org).await.ok();
        Ok(page)
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
        self.write(org, ADD_MEMBER, detail, key, move |db| {
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
        self.write(org, CHANGE_ROLE, detail, None, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, role != Role::Owner).await?;
                Ok(members::set_role(db, org, &email, role).await?)
            })
        })
        .await
    }

    pub async fn remove_member(&self, org: OrgId, email: Email) -> Result<(), Error> {
        let detail = email.as_str().to_owned();
        self.write(org, REMOVE_MEMBER, detail, None, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, true).await?;
                Ok(members::remove(db, org, &email).await?)
            })
        })
        .await
    }

    /// Hands the company's undelivered events to `after_commit` in order; a failed one waits, with those after it, for the next call.
    pub async fn deliver(&self, org: OrgId) -> Result<(), Error> {
        let hooks = Arc::clone(&self.hooks);
        let failed = self
            .host
            .transact(move |db| {
                Box::pin(async move {
                    for event in events::pending(db, org).await? {
                        if let Err(error) = hooks.after_commit(&event).await {
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
            signed: self.host.who().ok_or(Error::SignedOut)?,
        })
    }

    async fn write<T, F>(
        &self,
        org: OrgId,
        op: Operation,
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
                    let out = work(&mut *db).await?;
                    call.record(db, org, op.name(), &detail).await?;
                    call.remember(db, key.as_ref(), out.store()).await?;
                    Ok(out)
                })
            })
            .await?;
        self.deliver(org).await.ok();
        Ok(out)
    }
}

trait Answer: Sized {
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
        if let Some(member) = members::tie(db, org, &signed.email, signed.user).await? {
            self.record(db, org, "member.join", signed.email.as_str()).await?;
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

    async fn record(&self, db: &mut PgConnection, org: OrgId, action: &str, detail: &str) -> Result<(), Error> {
        members::lock_org(db, org).await?;
        let since = self
            .host
            .now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| Error::Host(e.to_string()))?;
        let at_micros = i64::try_from(since.as_micros()).map_err(|e| Error::Host(e.to_string()))?;
        let id = self.host.new_id();
        let row = Audit {
            id: &id,
            at_micros,
            user: self.signed.user,
            via: self.signed.via.as_str(),
            action,
            detail,
        };
        members::audit(db, org, row).await?;
        let event = events::write(db, org, &id, action, detail, self.signed.via.origin()).await?;
        self.hooks.before_commit(db, &event).await
    }
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
