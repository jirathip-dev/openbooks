use std::collections::BTreeSet;
use std::time::Duration;

use openbooks::testing::TestHost;
use openbooks::{
    Account, AuditRow, BOOKS, DimensionValue, Error, Filter, Host, Member, Openbooks, Operation, Record, Scope,
    SignedIn, Terms, Via,
};
use openbooks_core::{AccountType, Currency, Date, Email, OrgId, Origin, ProjectId, Refusal, Role, UserId, Word};
use serde_json::{Value, json};
use sqlx::{PgPool, query_scalar, raw_sql};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn email(text: &str) -> Res<Email> {
    Ok(Email::new(text)?)
}

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, email(address)?))
}

async fn books(pool: PgPool) -> Res<(Openbooks<TestHost>, OrgId)> {
    openbooks_db::migrate(&pool).await?;
    let books = Openbooks::new(TestHost::new(pool)).build();
    books.host().sign_in(person(1, "ann@example.com")?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, "none", None)
        .await?;
    Ok((books, org))
}

async fn rows(pool: &PgPool, table: &str) -> Res<i64> {
    Ok(query_scalar(&format!("SELECT count(*) FROM ledger.{table}"))
        .fetch_one(pool)
        .await?)
}

async fn audits(pool: &PgPool) -> Res<Vec<String>> {
    let sql = "SELECT action FROM ledger.audit ORDER BY length(audit_id), audit_id";
    Ok(query_scalar(sql).fetch_all(pool).await?)
}

#[sqlx::test(migrations = false)]
async fn a_write_under_a_host_that_rolls_back_leaves_no_row(pool: PgPool) -> Res<()> {
    openbooks_db::migrate(&pool).await?;
    let books = Openbooks::new(TestHost::new(pool.clone())).build();
    books.host().sign_in(person(1, "ann@example.com")?);
    books.host().roll_back(true);
    books
        .create_company("Acme", Currency::new("XTS", 2)?, "none", None)
        .await?;
    for table in ["org", "member", "audit", "event"] {
        assert_eq!(rows(&pool, table).await?, 0, "{table}");
    }
    books.host().roll_back(false);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, "none", None)
        .await?;
    books.host().roll_back(true);
    books
        .add_member(org, email("bob@example.com")?, Role::Staff, None)
        .await?;
    assert_eq!(
        (rows(&pool, "member").await?, audits(&pool).await?),
        (1, vec!["company.create".to_owned()])
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn each_operation_is_declared_with_its_word_and_each_role_gets_what_its_words_allow(pool: PgPool) -> Res<()> {
    let (read, approve) = (Word::Read, Word::Approve);
    let chart = [
        read, approve, approve, read, approve, read, approve, approve, approve, read, approve, approve,
    ];
    let words = [
        [read, Word::Own, Word::Own, Word::Own, approve].as_slice(),
        &chart,
        &[read, approve],
    ]
    .concat();
    assert_eq!(BOOKS.operations.iter().map(Operation::word).collect::<Vec<_>>(), words);
    let (books, org) = books(pool).await?;
    for (n, role) in (2..).zip(Role::ALL) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, email(&address)?, role, None).await?;
        books.host().sign_in(person(n, &address)?);
        let read = books.members(org, None).await.map(drop);
        let own = books
            .add_member(org, email(&format!("new{n}@example.com"))?, Role::Viewer, None)
            .await;
        for (outcome, word) in [(read, Word::Read), (own, Word::Own)] {
            match outcome {
                Ok(()) => assert!(role.allows(word), "{role:?} did {word:?}"),
                Err(Error::Forbidden { word: needed }) => assert!(!role.allows(word) && needed == word),
                Err(other) => return Err(other.into()),
            }
        }
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_host_narrows_an_owner_as_an_assistant_or_to_projects_and_never_widens_a_viewer(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    let ann = person(1, "ann@example.com")?;
    let projects = Scope::Projects(BTreeSet::from([ProjectId::new(7)?]));
    for narrowed in [
        SignedIn {
            via: Via::Assistant,
            ..ann.clone()
        },
        SignedIn {
            scope: projects.clone(),
            ..ann.clone()
        },
    ] {
        books.host().sign_in(narrowed);
        assert_eq!(books.members(org, None).await?.items.len(), 1);
        let added = books
            .add_member(org, email("bob@example.com")?, Role::Staff, None)
            .await;
        assert!(matches!(added, Err(Error::Forbidden { word: Word::Own })), "{added:?}");
    }
    assert!(projects.covers(ProjectId::new(7)?) && !projects.covers(ProjectId::new(8)?));
    books.host().sign_in(ann);
    books
        .add_member(org, email("vic@example.com")?, Role::Viewer, None)
        .await?;
    books.host().sign_in(SignedIn {
        via: Via::Host(Origin::new("payroll-sync")?),
        ..person(3, "vic@example.com")?
    });
    let added = books
        .add_member(org, email("eve@example.com")?, Role::Owner, None)
        .await;
    assert!(matches!(added, Err(Error::Forbidden { word: Word::Own })), "{added:?}");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_signed_in_person_who_is_no_member_is_refused_by_name(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    books.host().sign_in(person(9, "mallory@example.com")?);
    let refused = books.members(org, None).await.map(drop).map_err(|e| e.to_string());
    assert_eq!(
        refused,
        Err("mallory@example.com is not a member of this company".to_owned())
    );
    books.host().name(UserId::new(9)?, "Mallory Stone");
    let refused = books.members(org, None).await.map(drop).map_err(|e| e.to_string());
    assert_eq!(refused, Err("Mallory Stone is not a member of this company".to_owned()));
    books.host().sign_out();
    assert!(matches!(books.members(org, None).await, Err(Error::SignedOut)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_member_added_by_email_is_tied_to_their_user_id_at_first_sign_in(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool.clone()).await?;
    books
        .add_member(org, email("Bob@Example.com")?, Role::Staff, None)
        .await?;
    let bob = |user| {
        Res::Ok(Member {
            email: email("bob@example.com")?,
            user,
            role: Role::Staff,
        })
    };
    assert!(books.members(org, None).await?.items.contains(&bob(None)?));
    books.host().sign_in(person(2, "bob@example.com")?);
    assert!(
        books
            .members(org, None)
            .await?
            .items
            .contains(&bob(Some(UserId::new(2)?))?)
    );
    books.members(org, None).await?;
    let actions = ["company.create", "member.add", "member.join"];
    assert_eq!(audits(&pool).await?, actions.map(str::to_owned));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_last_owner_cannot_be_removed_or_lowered(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool.clone()).await?;
    let (ann, bob) = (email("ann@example.com")?, email("bob@example.com")?);
    let lowered = books.change_role(org, ann.clone(), Role::Accountant).await;
    assert!(matches!(lowered, Err(Error::LastOwner)), "{lowered:?}");
    assert!(matches!(
        books.remove_member(org, ann.clone()).await,
        Err(Error::LastOwner)
    ));
    books.add_member(org, bob.clone(), Role::Owner, None).await?;
    books.change_role(org, ann.clone(), Role::Accountant).await?;
    books.host().sign_in(person(2, "bob@example.com")?);
    assert!(matches!(
        books.remove_member(org, bob.clone()).await,
        Err(Error::LastOwner)
    ));
    books.remove_member(org, ann.clone()).await?;
    let left = books.members(org, None).await?.items;
    assert_eq!(
        left.iter().map(|m| (&m.email, m.role)).collect::<Vec<_>>(),
        [(&bob, Role::Owner)]
    );
    let actions = [
        "company.create",
        "member.add",
        "member.role",
        "member.join",
        "member.remove",
    ];
    assert_eq!(audits(&pool).await?, actions.map(str::to_owned));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn members_come_a_page_at_a_time_and_the_cursor_reaches_the_last(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    for n in 0..100 {
        books
            .add_member(org, email(&format!("m{n:03}@example.com"))?, Role::Viewer, None)
            .await?;
    }
    let first = books.members(org, None).await?;
    let last = first.items.last().map(|m| m.email.as_str().to_owned());
    assert_eq!(
        (first.items.len(), &first.next, &last),
        (100, &last, &Some("m098@example.com".to_owned()))
    );
    let second = books.members(org, first.next).await?;
    let emails: Vec<&str> = second.items.iter().map(|m| m.email.as_str()).collect();
    assert_eq!((emails, second.next), (vec!["m099@example.com"], None));
    Ok(())
}

async fn log(books: &Openbooks<TestHost>, org: OrgId, filter: Filter) -> Res<Vec<AuditRow>> {
    Ok(books.audit(org, filter, None).await?.items)
}

fn json(text: Option<&String>) -> Res<Option<Value>> {
    Ok(text.map(|t| serde_json::from_str(t)).transpose()?)
}

fn wes(role: &str, removed: bool) -> Value {
    json!({"email": "wes@example.com", "user_id": null, "role": role, "removed": removed})
}

type Seen = (String, Option<Value>, Option<Value>);

async fn call(books: &Openbooks<TestHost>, org: OrgId, op: &Operation) -> Res<Seen> {
    let address = email("wes@example.com")?;
    let record = "member:wes@example.com".to_owned();
    match op.name() {
        "member.add" => books.add_member(org, address, Role::Staff, None).await?,
        "member.role" => books.change_role(org, address, Role::Viewer).await?,
        "member.remove" => books.remove_member(org, address).await?,
        _ => return set_up(books, org, op).await,
    }
    Ok(match op.name() {
        "member.add" => (record, None, Some(wes("staff", false))),
        "member.role" => (record, Some(wes("staff", false)), Some(wes("viewer", false))),
        _ => (record, Some(wes("viewer", false)), Some(wes("viewer", true))),
    })
}

fn cash(requires: &Value) -> Value {
    json!({"code": "1000", "name": "Cash", "type": "asset", "parent_id": null, "active": true, "currency": null, "requires": requires})
}

fn kinds(names: &[&str]) -> Value {
    Value::Array(
        (1..)
            .zip(names)
            .map(|(slot, name)| json!({"slot": slot, "name": name}))
            .collect(),
    )
}

fn value(code: &str, name: &str, host_ref: &Value) -> Value {
    json!({"code": code, "name": name, "active": true, "host_ref": host_ref})
}

async fn set_up(books: &Openbooks<TestHost>, org: OrgId, op: &Operation) -> Res<Seen> {
    let text = str::to_owned;
    let day = Date::new(2026, 1, 1)?;
    let (record, before, after) = match op.name() {
        "account.save" => {
            let cash_account = Account {
                code: text("1000"),
                name: text("Cash"),
                account_type: AccountType::Asset,
                parent: None,
                active: true,
                currency: None,
            };
            books.save_account(org, cash_account).await?;
            ("account:1000", None, Some(cash(&Value::Null)))
        }
        "account.require" => {
            books.require_dimension(org, text("1000"), 1, true).await?;
            ("account:1000", Some(cash(&Value::Null)), Some(cash(&json!([1]))))
        }
        "account.role" => {
            books.give_role(org, text("receivables"), day, text("1000")).await?;
            (
                "role:receivables",
                None,
                Some(json!([{"from": "2026-01-01", "account": "1000"}])),
            )
        }
        _ => return dimensions(books, org, op).await,
    };
    Ok((record.to_owned(), before, after))
}

async fn dimensions(books: &Openbooks<TestHost>, org: OrgId, op: &Operation) -> Res<Seen> {
    let text = str::to_owned;
    let (record, before, after) = match op.name() {
        "dimension.add" => {
            books.add_dimension(org, text("Cost centre"), None).await?;
            (
                "dimensions",
                Some(kinds(&["Project"])),
                Some(kinds(&["Project", "Cost centre"])),
            )
        }
        "dimension.rename" => {
            books.rename_dimension(org, 2, text("Department")).await?;
            (
                "dimensions",
                Some(kinds(&["Project", "Cost centre"])),
                Some(kinds(&["Project", "Department"])),
            )
        }
        "dimension.remove" => {
            books.remove_dimension(org, 2).await?;
            (
                "dimensions",
                Some(kinds(&["Project", "Department"])),
                Some(kinds(&["Project"])),
            )
        }
        "dimension_value.save" => {
            books.save_value(org, 1, text("P1"), text("Bridge"), true).await?;
            ("value:1:P1", None, Some(value("P1", "Bridge", &Value::Null)))
        }
        "dimension_value.sync" => {
            let synced = DimensionValue {
                code: text("P7"),
                name: text("Tunnel"),
                active: true,
                host_ref: None,
            };
            books.sync_value(org, 1, text("job-7"), synced).await?;
            ("value:1:host:job-7", None, Some(value("P7", "Tunnel", &json!("job-7"))))
        }
        "terms.save" => {
            let net = Terms {
                code: text("NET"),
                name: text("Net"),
                days: 30,
                active: true,
            };
            books.save_terms(org, net).await?;
            (
                "terms:NET",
                None,
                Some(json!({"code": "NET", "name": "Net", "days": 30, "active": true})),
            )
        }
        other => return Err(format!("{other} is a declared write this walk does not call").into()),
    };
    Ok((record.to_owned(), before, after))
}

fn without_ids(snapshot: Option<Value>) -> Option<Value> {
    snapshot.map(|mut snapshot| {
        if let Some(fields) = snapshot.as_object_mut() {
            fields.retain(|key, _| !key.ends_with("_id") || key == "parent_id" || key == "user_id");
        }
        snapshot
    })
}

#[sqlx::test(migrations = false)]
async fn every_declared_write_leaves_exactly_one_audit_row_with_the_record_before_and_after(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    let [opened] = log(&books, org, Filter::default())
        .await?
        .try_into()
        .map_err(|_| "not one row")?;
    let company = json(opened.after.as_ref())?.ok_or("no company after")?;
    assert_eq!(
        (opened.action.as_str(), opened.record.as_str(), opened.before),
        ("company.create", "company", None)
    );
    assert_eq!(
        (company.get("name"), company.get("base_currency")),
        (Some(&json!("Acme")), Some(&json!("XTS")))
    );
    let writes: Vec<&Operation> = BOOKS.operations.iter().filter(|op| !op.is_read()).collect();
    assert_eq!(writes.len(), 12);
    for op in writes {
        let seen = log(&books, org, Filter::default()).await?.len();
        let (record, before, after) = call(&books, org, op).await?;
        let rows = log(&books, org, Filter::default()).await?;
        assert_eq!(
            rows.len(),
            seen + 1,
            "{} left {} audit rows",
            op.name(),
            rows.len() - seen
        );
        let row = rows.last().ok_or("no row")?;
        assert_eq!((row.action.as_str(), row.record.as_str()), (op.name(), record.as_str()));
        let seen = (
            without_ids(json(row.before.as_ref())?),
            without_ids(json(row.after.as_ref())?),
        );
        assert_eq!(seen, (before, after), "{}", op.name());
    }
    books
        .add_member(org, email("joe@example.com")?, Role::Staff, None)
        .await?;
    books.host().sign_in(person(5, "joe@example.com")?);
    books.members(org, None).await?;
    books.members(org, None).await?;
    books.host().sign_in(person(1, "ann@example.com")?);
    let rows = log(&books, org, Filter::default()).await?;
    let joined = rows.last().ok_or("no row")?;
    assert_eq!(
        (rows.len(), joined.action.as_str(), joined.user.get()),
        (15, "member.join", 5)
    );
    let tied = |user| json!({"email": "joe@example.com", "user_id": user, "role": "staff", "removed": false});
    assert_eq!(
        (json(joined.before.as_ref())?, json(joined.after.as_ref())?),
        (Some(tied(Value::Null)), Some(tied(json!(5))))
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_database_refuses_any_change_to_an_audit_row_or_its_removal(pool: PgPool) -> Res<()> {
    let _books = books(pool.clone()).await?;
    for sql in [
        "UPDATE ledger.audit SET detail = 'nothing happened'",
        "DELETE FROM ledger.event; DELETE FROM ledger.audit",
        "TRUNCATE ledger.audit CASCADE",
    ] {
        let refused = raw_sql(sql).execute(&pool).await.map(drop);
        let message = refused.map_err(|e| e.as_database_error().map(|d| d.message().to_owned()));
        assert_eq!(
            message,
            Err(Some("an audit row is never changed or removed".to_owned())),
            "{sql}"
        );
    }
    let kept: Vec<String> = query_scalar("SELECT detail FROM ledger.audit").fetch_all(&pool).await?;
    assert_eq!(kept, ["Acme"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn only_owners_and_accountants_read_the_log_and_never_through_an_assistant(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    for (n, role) in (2..).zip(Role::ALL) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, email(&address)?, role, None).await?;
        books.host().sign_in(person(n, &address)?);
        let read = books.audit(org, Filter::default(), None).await;
        match (role, read) {
            (Role::Owner | Role::Accountant, Ok(page)) => {
                let first = page.items.first().map(|row| row.action.as_str());
                assert_eq!(first, Some("company.create"));
            }
            (Role::Staff | Role::Viewer, Err(Error::Forbidden { word: Word::Approve })) => {}
            (role, read) => return Err(format!("{role:?} read the log: {read:?}").into()),
        }
    }
    books.host().sign_in(SignedIn {
        via: Via::Assistant,
        ..person(1, "ann@example.com")?
    });
    let read = books.audit(org, Filter::default(), None).await;
    assert!(
        matches!(read, Err(Error::Forbidden { word: Word::Approve })),
        "{read:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_log_filters_by_record_person_and_kind_and_names_the_way_each_change_came(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    books
        .add_member(org, email("bob@example.com")?, Role::Owner, None)
        .await?;
    for (via, address) in [(Via::Pack, "pat@example.com"), (Via::Schedule, "sam@example.com")] {
        books.host().sign_in(SignedIn {
            via,
            ..person(2, "bob@example.com")?
        });
        books.add_member(org, email(address)?, Role::Viewer, None).await?;
    }
    books.host().sign_in(person(1, "ann@example.com")?);
    books.change_role(org, email("pat@example.com")?, Role::Staff).await?;
    let found = async |filter| {
        let rows = log(&books, org, filter).await?;
        Res::Ok(
            rows.into_iter()
                .map(|r| format!("{} {} {}", r.action, r.via, r.record))
                .collect::<Vec<_>>(),
        )
    };
    let everything = found(Filter::default()).await?;
    assert_eq!(
        everything,
        [
            "company.create person company",
            "member.add person member:bob@example.com",
            "member.join pack member:bob@example.com",
            "member.add pack member:pat@example.com",
            "member.add schedule member:sam@example.com",
            "member.role person member:pat@example.com",
        ]
    );
    let pat = Filter {
        record: Some(Record::Member(email("pat@example.com")?)),
        ..Filter::default()
    };
    assert_eq!(
        found(pat).await?,
        [
            "member.add pack member:pat@example.com",
            "member.role person member:pat@example.com"
        ]
    );
    let bob = Filter {
        user: Some(UserId::new(2)?),
        action: Some("member.add".to_owned()),
        ..Filter::default()
    };
    assert_eq!(
        found(bob).await?,
        [
            "member.add pack member:pat@example.com",
            "member.add schedule member:sam@example.com"
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_log_keeps_the_changes_in_a_time_span_and_goes_on_from_a_cursor(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool).await?;
    books
        .add_member(org, email("bob@example.com")?, Role::Owner, None)
        .await?;
    let found = async |filter| Res::Ok(log(&books, org, filter).await?.len());
    let now = books.host().now();
    let span = |from, until| Filter {
        from: Some(from),
        until: Some(until),
        ..Filter::default()
    };
    assert_eq!(found(span(now, now + Duration::from_micros(1))).await?, 2);
    assert_eq!(found(span(now - Duration::from_secs(60), now)).await?, 0);
    let rows = log(&books, org, Filter::default()).await?;
    let first = rows.first().ok_or("no row")?.seq.to_string();
    let rest = books.audit(org, Filter::default(), Some(first)).await?;
    assert_eq!((rest.items.len(), rest.next), (1, None));
    let bad = books.audit(org, Filter::default(), Some("x".to_owned())).await;
    assert!(matches!(bad, Err(Error::Refused(Refusal::BadId))), "{bad:?}");
    Ok(())
}
