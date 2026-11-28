use std::sync::Arc;
use std::time::UNIX_EPOCH;

use openbooks_core::{Currency, Email, OrgId, Role, Word};
use openbooks_db::members::{self, Audit, Member};
use sqlx::PgConnection;

use crate::{Error, Host, SignedIn, Work};

/// Every operation on a company, each declared with the one word it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Members,
    AddMember,
    ChangeRole,
    RemoveMember,
}

impl Op {
    pub const ALL: [Self; 4] = [Self::Members, Self::AddMember, Self::ChangeRole, Self::RemoveMember];

    pub fn word(self) -> Word {
        match self {
            Self::Members => Word::Read,
            Self::AddMember | Self::ChangeRole | Self::RemoveMember => Word::Own,
        }
    }

    fn action(self) -> Option<&'static str> {
        match self {
            Self::Members => None,
            Self::AddMember => Some("member.add"),
            Self::ChangeRole => Some("member.role"),
            Self::RemoveMember => Some("member.remove"),
        }
    }
}

/// The books, embedded in a host.
pub struct Openbooks<H> {
    host: Arc<H>,
}

/// Sets up the books for a host; `build` hands them over.
pub struct Builder<H> {
    host: H,
}

impl<H: Host + 'static> Builder<H> {
    pub fn build(self) -> Openbooks<H> {
        Openbooks {
            host: Arc::new(self.host),
        }
    }
}

impl<H: Host + 'static> Openbooks<H> {
    #[expect(
        clippy::new_ret_no_self,
        reason = "the books are built: Openbooks::new(host).build()"
    )]
    pub fn new(host: H) -> Builder<H> {
        Builder { host }
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    /// Opens a company with the person signed in as its first Owner.
    pub async fn create_company(&self, name: &str, base: Currency) -> Result<OrgId, Error> {
        let signed = self.host.who().ok_or(Error::SignedOut)?;
        let (host, name) = (Arc::clone(&self.host), name.to_owned());
        self.host
            .transact(move |db| {
                Box::pin(async move {
                    let org = members::create_org(db, &name, base).await?;
                    members::add(db, org, &signed.email, Role::Owner).await?;
                    members::tie(db, org, &signed.email, signed.user).await?;
                    audit(&*host, db, org, &signed, "company.create", &name).await?;
                    Ok(org)
                })
            })
            .await
    }

    pub async fn members(&self, org: OrgId) -> Result<Vec<Member>, Error> {
        self.on(org, Op::Members, String::new(), move |db| {
            Box::pin(async move { Ok(members::members(db, org).await?) })
        })
        .await
    }

    pub async fn add_member(&self, org: OrgId, email: Email, role: Role) -> Result<(), Error> {
        let detail = format!("{} as {}", email.as_str(), role.as_str());
        self.on(org, Op::AddMember, detail, move |db| {
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
        self.on(org, Op::ChangeRole, detail, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, role != Role::Owner).await?;
                Ok(members::set_role(db, org, &email, role).await?)
            })
        })
        .await
    }

    pub async fn remove_member(&self, org: OrgId, email: Email) -> Result<(), Error> {
        let detail = email.as_str().to_owned();
        self.on(org, Op::RemoveMember, detail, move |db| {
            Box::pin(async move {
                keep_an_owner(db, org, &email, true).await?;
                Ok(members::remove(db, org, &email).await?)
            })
        })
        .await
    }

    async fn on<T, F>(&self, org: OrgId, op: Op, detail: String, work: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, T> + Send + 'static,
    {
        let signed = self.host.who().ok_or(Error::SignedOut)?;
        let host = Arc::clone(&self.host);
        self.host
            .transact(move |db| {
                Box::pin(async move {
                    let member = caller(&*host, db, org, &signed).await?;
                    if !(member.role.allows(op.word()) && signed.keeps(op.word())) {
                        return Err(Error::Forbidden { word: op.word() });
                    }
                    let out = work(&mut *db).await?;
                    if let Some(action) = op.action() {
                        audit(&*host, db, org, &signed, action, &detail).await?;
                    }
                    Ok(out)
                })
            })
            .await
    }
}

async fn caller<H: Host>(host: &H, db: &mut PgConnection, org: OrgId, signed: &SignedIn) -> Result<Member, Error> {
    if let Some(member) = members::member_by_user(db, org, signed.user).await? {
        return Ok(member);
    }
    if let Some(member) = members::tie(db, org, &signed.email, signed.user).await? {
        audit(host, db, org, signed, "member.join", signed.email.as_str()).await?;
        return Ok(member);
    }
    let name = host.names(&[signed.user]).remove(&signed.user);
    Err(Error::NotMember {
        name: name.unwrap_or_else(|| signed.email.as_str().to_owned()),
    })
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

async fn audit<H: Host>(
    host: &H,
    db: &mut PgConnection,
    org: OrgId,
    signed: &SignedIn,
    action: &str,
    detail: &str,
) -> Result<(), Error> {
    let since = host
        .now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Host(e.to_string()))?;
    let at_micros = i64::try_from(since.as_micros()).map_err(|e| Error::Host(e.to_string()))?;
    let row = Audit {
        id: &host.new_id(),
        at_micros,
        user: signed.user,
        via: signed.via.as_str(),
        action,
        detail,
    };
    Ok(members::audit(db, org, row).await?)
}
