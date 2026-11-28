use openbooks::testing::{TestHost, TestPack};
use openbooks::{BOOKS, Contact, Error, Identity, Openbooks, SignedIn, Terms};
use openbooks_core::{ContactKind, Currency, Email, OrgId, Refusal, Role, UserId, Word};
use serde_json::Value;
use sqlx::{PgPool, query_scalar};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, Email::new(address)?))
}

async fn books(pool: &PgPool) -> Res<(Openbooks<TestHost>, OrgId)> {
    let books = Openbooks::new(TestHost::new(pool.clone()))
        .pack(TestPack::default())
        .build();
    books.migrate(pool).await?;
    books.host().sign_in(person(1, "ann@example.com")?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, "test", None)
        .await?;
    Ok((books, org))
}

fn contact(code: &str, kind: ContactKind) -> Contact {
    Contact {
        code: code.to_owned(),
        name: format!("Contact {code}"),
        kind,
        address: Some("1 Main Road".to_owned()),
        email: Email::new(&format!("{}@example.com", code.to_lowercase())).ok(),
        phone: Some("555 0100".to_owned()),
        terms: None,
        currency: None,
        withholding: None,
        host_ref: None,
    }
}

async fn audits(pool: &PgPool, org: OrgId, record: &str) -> Res<i64> {
    let sql = "SELECT count(*) FROM ledger.audit WHERE org_id = $1 AND record = $2";
    Ok(query_scalar(sql).bind(org.get()).bind(record).fetch_one(pool).await?)
}

fn id(text: &str) -> Option<String> {
    text.parse().ok()
}

async fn banked_customer(books: &Openbooks<TestHost>, org: OrgId) -> Res<Contact> {
    let net = Terms {
        code: "NET30".to_owned(),
        name: "Net 30".to_owned(),
        days: 30,
        active: true,
    };
    books.save_terms(org, net).await?;
    let trading = Contact {
        terms: id("NET30"),
        currency: id("XTS"),
        withholding: id("W3"),
        ..contact("C1", ContactKind::Customer)
    };
    books
        .save_contact(org, trading.clone(), id("0105"), id("00001"))
        .await?;
    books.set_bank_account(org, "C1".to_owned(), id("BANK 123")).await?;
    Ok(trading)
}

#[sqlx::test(migrations = false)]
async fn the_pack_checks_a_customer_on_save_and_its_identity_is_read_on_its_own_route(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool).await?;
    let unnamed = books
        .save_contact(org, contact("C1", ContactKind::Customer), None, None)
        .await;
    assert!(matches!(unnamed, Err(Error::Refused(Refusal::Party(_)))), "{unnamed:?}");
    assert_eq!(audits(&pool, org, "contact:C1").await?, 0);
    let trading = banked_customer(&books, org).await?;
    assert_eq!(audits(&pool, org, "contact:C1").await?, 2);
    let refusals = [
        Contact {
            withholding: id("W9"),
            ..trading.clone()
        },
        Contact {
            terms: id("NET99"),
            ..trading.clone()
        },
    ];
    for refused in refusals {
        let outcome = books.save_contact(org, refused, id("0105"), None).await;
        let expected = matches!(
            outcome,
            Err(Error::Refused(Refusal::UnknownTaxCode) | Error::NoSuchTerms(_))
        );
        assert!(expected, "{outcome:?}");
    }
    assert_eq!(audits(&pool, org, "contact:C1").await?, 2);
    assert_eq!(books.contacts(org, None).await?.items, [trading]);
    let identity = Identity {
        tax_id: id("0105"),
        branch: id("00001"),
        bank_account: id("BANK 123"),
    };
    assert_eq!(books.contact_identity(org, "C1".to_owned()).await?, identity);
    let missing = books.contact_identity(org, "X".to_owned()).await;
    assert!(matches!(missing, Err(Error::NoSuchContact(_))), "{missing:?}");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_employee_holds_only_a_name_and_how_to_reach_them(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool).await?;
    let trading = banked_customer(&books, org).await?;
    let employee = contact("E1", ContactKind::Employee);
    books.save_contact(org, employee.clone(), None, None).await?;
    let with_terms = Contact {
        terms: id("NET30"),
        ..employee.clone()
    };
    let banked_employee = Contact {
        kind: ContactKind::Employee,
        terms: None,
        currency: None,
        withholding: None,
        ..trading.clone()
    };
    let paid = [
        books.save_contact(org, employee.clone(), id("0105"), None).await,
        books.save_contact(org, with_terms, None, None).await,
        books.set_bank_account(org, "E1".to_owned(), id("BANK 9")).await,
        books.save_contact(org, banked_employee, None, None).await,
    ];
    for outcome in paid {
        assert!(matches!(outcome, Err(Error::EmployeeHoldsOnlyReach)), "{outcome:?}");
    }
    assert_eq!(audits(&pool, org, "contact:E1").await?, 1);
    assert_eq!(audits(&pool, org, "contact:C1").await?, 2);
    assert_eq!(books.contacts(org, None).await?.items, [trading, employee]);
    assert_eq!(books.contact_identity(org, "E1".to_owned()).await?, Identity::default());
    Ok(())
}

fn answer<'a>(doc: &'a Value, op: &str) -> &'a Value {
    let pointer = format!("/paths/~1books~1{{org}}~1{op}/get/responses/200/content/application~1json/schema");
    doc.pointer(&pointer).unwrap_or(&Value::Null)
}

#[test]
fn the_contact_read_holds_no_never_tier_field_and_the_identity_is_a_closed_read_of_never_fields() -> Res<()> {
    let doc: Value = serde_json::from_str(&BOOKS.openapi())?;
    let contacts = answer(&doc, "contacts");
    let item = contacts.pointer("/properties/items/items/properties");
    let fields: Vec<&String> = item.and_then(Value::as_object).ok_or("no contact")?.keys().collect();
    assert!(!fields.is_empty() && !contacts.to_string().contains("x-openbooks-tier"));
    for absent in ["tax_id", "branch", "bank_account"] {
        assert!(!fields.iter().any(|f| *f == absent), "{absent} in {fields:?}");
    }
    let identity = answer(&doc, "contact.identity");
    for name in ["tax_id", "branch", "bank_account"] {
        let tier = identity.pointer(&format!("/properties/{name}/x-openbooks-tier"));
        assert_eq!(tier, Some(&Value::from("never")), "{name}");
    }
    let tool = doc.pointer("/paths/~1books~1{org}~1contact.identity/get/x-openbooks-tool");
    assert_eq!(tool, Some(&Value::Bool(false)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_host_syncs_a_contact_by_its_reference_and_staff_draft_contacts_but_set_no_bank_account(
    pool: PgPool,
) -> Res<()> {
    let (books, org) = books(&pool).await?;
    let supplier = contact("S1", ContactKind::Supplier);
    let crm = "crm-7".to_owned();
    books
        .sync_contact(org, crm.clone(), supplier.clone(), id("0999"), None)
        .await?;
    let renamed = Contact {
        code: "S1A".to_owned(),
        name: "Supplier renamed".to_owned(),
        kind: ContactKind::CustomerAndSupplier,
        ..supplier
    };
    books
        .sync_contact(org, crm.clone(), renamed.clone(), id("0999"), None)
        .await?;
    let synced = Contact {
        host_ref: Some(crm),
        ..renamed
    };
    assert_eq!(books.contacts(org, None).await?.items, [synced]);
    assert_eq!(audits(&pool, org, "contact:host:crm-7").await?, 2);

    for (n, role) in (2..).zip([Role::Staff, Role::Viewer]) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, Email::new(&address)?, role, None).await?;
        books.host().sign_in(person(n, &address)?);
        let code = format!("C{n}");
        let saved = books
            .save_contact(org, contact(&code, ContactKind::Customer), id("0105"), None)
            .await;
        let banked = books.set_bank_account(org, "S1A".to_owned(), id("BANK 1")).await;
        for (outcome, word) in [(saved, Word::Draft), (banked, Word::Approve)] {
            match outcome {
                Ok(()) => assert!(role.allows(word), "{role:?} did {word:?}"),
                Err(Error::Forbidden { word: needed }) => assert!(!role.allows(word) && needed == word),
                Err(other) => return Err(other.into()),
            }
        }
    }
    let codes: Vec<String> = books
        .contacts(org, None)
        .await?
        .items
        .into_iter()
        .map(|c| c.code)
        .collect();
    assert_eq!(codes, ["C2", "S1A"]);
    Ok(())
}
