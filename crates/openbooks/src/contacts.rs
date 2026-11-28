use std::sync::Arc;

use openbooks_core::{ContactKind, Currency, Email, OrgId, Party, Refusal, Word};
use openbooks_db::audit::Record;
use openbooks_db::contacts::{self, Contact, Identity};
use sqlx::PgConnection;

use crate::members::{PAGE, Packs, pack_of, paged};
use crate::openapi::{Json, map, text};
use crate::{Ask, Error, Field, Host, Never, Openbooks, Operation, Page, Reply, Schema, Shape};

pub(crate) const CONTACTS: Operation = crate::read!("contacts", Word::Read, "Lists the customers, suppliers and employees by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<Contact>);

pub(crate) const SAVE_CONTACT: Operation = crate::closed!("contact.save", Word::Draft, "Adds a contact, or changes the one with its code; the company's pack checks a customer or supplier.", {
    code: String = "The contact's code, unique in the company.",
    name: String = "Its name.",
    kind: ContactKind = "Customer, supplier, both, or employee, who holds only a name and how to reach them.",
    address: Option<String> = "Its address.",
    email: Option<Email> = "Its email address.",
    phone: Option<String> = "Its phone number.",
    terms: Option<String> = "The code of the payment terms its documents default to.",
    currency: Option<String> = "The currency its documents default to, by code.",
    withholding: Option<String> = "The code of the tax its settlements withhold by default.",
    tax_id: Option<String> = "Its tax id, as plain text.",
    branch: Option<String> = "Its branch, as plain text.",
} -> ());

pub(crate) const SYNC_CONTACT: Operation = crate::closed!("contact.sync", Word::Draft, "Keeps a contact in step with the host's own record, by its reference; the company's pack checks a customer or supplier.", {
    host_ref: String = "The host's reference for the contact.",
    code: String = "The contact's code, unique in the company.",
    name: String = "Its name.",
    kind: ContactKind = "Customer, supplier, both, or employee, who holds only a name and how to reach them.",
    address: Option<String> = "Its address.",
    email: Option<Email> = "Its email address.",
    phone: Option<String> = "Its phone number.",
    terms: Option<String> = "The code of the payment terms its documents default to.",
    currency: Option<String> = "The currency its documents default to, by code.",
    withholding: Option<String> = "The code of the tax its settlements withhold by default.",
    tax_id: Option<String> = "Its tax id, as plain text.",
    branch: Option<String> = "Its branch, as plain text.",
} -> ());

pub(crate) const IDENTITY: Operation = crate::closed_read!("contact.identity", Word::Draft, "Shows a person a contact's tax id, branch and bank account.", {
    code: String = "The contact's code.",
} -> Identity);

pub(crate) const SET_BANK: Operation = crate::closed!("contact.bank", Word::Approve, "Sets the bank account a customer or supplier is paid to; an employee has none.", {
    code: String = "The contact's code.",
    bank_account: Option<String> = "The bank account; absent to clear it.",
} -> ());

fn field(name: &'static str, about: &'static str, schema: Schema) -> Field {
    Field { name, about, schema }
}

impl Shape for ContactKind {
    fn schema() -> Schema {
        Schema::Words(ContactKind::ALL.map(ContactKind::as_str).to_vec())
    }
}

impl Shape for Contact {
    fn schema() -> Schema {
        let text = Option::<String>::schema;
        Schema::Object(vec![
            field("code", "The contact's code.", String::schema()),
            field("name", "Its name.", String::schema()),
            field("kind", "Customer, supplier, both, or employee.", ContactKind::schema()),
            field("address", "Its address.", text()),
            field("email", "Its email address.", Option::<Email>::schema()),
            field("phone", "Its phone number.", text()),
            field(
                "terms",
                "The code of the payment terms its documents default to.",
                text(),
            ),
            field("currency", "The currency its documents default to.", text()),
            field(
                "withholding",
                "The code of the tax its settlements withhold by default.",
                text(),
            ),
            field("host_ref", "The host's reference, when the host keeps it.", text()),
        ])
    }
}

impl Shape for Identity {
    fn schema() -> Schema {
        let never = Option::<Never<String>>::schema;
        Schema::Object(vec![
            field("tax_id", "Its tax id, as plain text.", never()),
            field("branch", "Its branch, as plain text.", never()),
            field("bank_account", "The bank account it is paid to.", never()),
        ])
    }
}

fn json(c: &Contact) -> Json {
    let email = c.email.as_ref().map(|e| e.as_str().to_owned());
    let optional = [
        ("address", &c.address),
        ("email", &email),
        ("phone", &c.phone),
        ("terms", &c.terms),
        ("currency", &c.currency),
        ("withholding", &c.withholding),
        ("host_ref", &c.host_ref),
    ];
    let json = map([
        ("code", text(&c.code)),
        ("name", text(&c.name)),
        ("kind", text(c.kind.as_str())),
    ]);
    optional.into_iter().fold(json, |json, (key, value)| match value {
        Some(value) => json.with(key, text(value)),
        None => json,
    })
}

/// Answers the `contacts` tool: a page of contacts with no tax identity or bank account.
pub(crate) async fn serve(ask: Ask<'_>) -> Result<Reply, Error> {
    let (limit, after) = (i64::from(ask.rows()), ask.after()?);
    let rows = contacts::contacts(ask.db, ask.org, after.as_deref(), limit).await?;
    Ok(Reply::Rows(rows.iter().map(|c| (json(c), c.code.clone())).collect()))
}

async fn check(
    packs: &Packs,
    db: &mut PgConnection,
    org: OrgId,
    contact: &Contact,
    tax_id: Option<&String>,
    branch: Option<&String>,
) -> Result<(), Error> {
    let by = contact.host_ref.as_deref().unwrap_or(&contact.code);
    let stored = contacts::contact_by(db, org, by, contact.host_ref.is_some()).await?;
    if contact.kind == ContactKind::Employee {
        let banked = stored.is_some_and(|(_, identity)| identity.bank_account.is_some());
        let trade = [&contact.terms, &contact.currency, &contact.withholding];
        if banked || tax_id.or(branch).is_some() || trade.iter().any(|f| f.is_some()) {
            return Err(Error::EmployeeHoldsOnlyReach);
        }
        return Ok(());
    }
    if let Some(code) = &contact.terms
        && !contacts::terms_exist(db, org, code).await?
    {
        return Err(Error::NoSuchTerms(code.clone()));
    }
    let tax = pack_of(packs, db, org).await?;
    let tax = tax.tax();
    if let Some(code) = &contact.withholding
        && !tax.withheld().iter().any(|w| w.code == code)
    {
        return Err(Refusal::UnknownTaxCode.into());
    }
    let party = Party {
        name: contact.name.clone(),
        tax_id: tax_id.cloned(),
    };
    Ok(tax.check_party(&party)?)
}

impl<H: Host + 'static> Openbooks<H> {
    /// One page of the contacts by code, from just after the cursor `after`; it holds no tax identity or bank account.
    pub async fn contacts(&self, org: OrgId, after: Option<String>) -> Result<Page<Contact>, Error> {
        self.read(org, CONTACTS, move |db| {
            Box::pin(async move {
                let items = contacts::contacts(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |c| c.code.clone()))
            })
        })
        .await
    }

    /// Adds the contact with its tax id and branch, or changes the one with its code, once the pack passes it.
    pub async fn save_contact(
        &self,
        org: OrgId,
        contact: Contact,
        tax_id: Option<String>,
        branch: Option<String>,
    ) -> Result<(), Error> {
        let record = Record::Contact(contact.code.clone());
        let contact = Contact {
            host_ref: None,
            ..contact
        };
        self.put(org, SAVE_CONTACT, record, contact, tax_id, branch).await
    }

    /// Keeps the contact with the host's reference in step with the host's own record, once the pack passes it.
    pub async fn sync_contact(
        &self,
        org: OrgId,
        host_ref: String,
        contact: Contact,
        tax_id: Option<String>,
        branch: Option<String>,
    ) -> Result<(), Error> {
        let record = Record::SyncedContact(host_ref.clone());
        let contact = Contact {
            host_ref: Some(host_ref),
            ..contact
        };
        self.put(org, SYNC_CONTACT, record, contact, tax_id, branch).await
    }

    async fn put(
        &self,
        org: OrgId,
        op: Operation,
        record: Record,
        contact: Contact,
        tax_id: Option<String>,
        branch: Option<String>,
    ) -> Result<(), Error> {
        if let Some(code) = &contact.currency {
            Currency::new(code, 0)?;
        }
        let packs = Arc::clone(&self.packs);
        let host_ref = contact
            .host_ref
            .as_deref()
            .map_or_else(String::new, |r| format!("{r} "));
        let detail = format!("{host_ref}{} {} {}", contact.code, contact.kind.as_str(), contact.name);
        self.write(org, op, record, detail, None, move |db| {
            Box::pin(async move {
                check(&packs, db, org, &contact, tax_id.as_ref(), branch.as_ref()).await?;
                Ok(contacts::save(db, org, &contact, tax_id.as_deref(), branch.as_deref()).await?)
            })
        })
        .await
    }

    /// The contact's tax id, branch and bank account, for a person to see.
    pub async fn contact_identity(&self, org: OrgId, code: String) -> Result<Identity, Error> {
        self.read(org, IDENTITY, move |db| {
            Box::pin(async move {
                let found = contacts::contact_by(db, org, &code, false).await?;
                found.map(|(_, identity)| identity).ok_or(Error::NoSuchContact(code))
            })
        })
        .await
    }

    /// Sets or clears the bank account the customer or supplier with the code is paid to.
    pub async fn set_bank_account(&self, org: OrgId, code: String, bank_account: Option<String>) -> Result<(), Error> {
        let detail = format!("{code} {}", if bank_account.is_some() { "set" } else { "cleared" });
        let record = Record::Contact(code.clone());
        self.write(org, SET_BANK, record, detail, None, move |db| {
            Box::pin(async move {
                let found = contacts::contact_by(db, org, &code, false).await?;
                match found {
                    None => Err(Error::NoSuchContact(code)),
                    Some((contact, _)) if contact.kind == ContactKind::Employee => Err(Error::EmployeeHoldsOnlyReach),
                    Some(_) => Ok(contacts::set_bank(db, org, &code, bank_account.as_deref())
                        .await
                        .map(drop)?),
                }
            })
        })
        .await
    }
}
