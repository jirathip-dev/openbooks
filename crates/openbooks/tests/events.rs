use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use openbooks::testing::TestHost;
use openbooks::{Error, Event, Hooks, Openbooks, SignedIn, Via, Work};
use openbooks_core::{Currency, Email, IdempotencyKey, OrgId, Origin, Role, UserId};
use sqlx::{PgConnection, PgPool, query_as, query_scalar};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Seen {
    refuse: Mutex<Option<&'static str>>,
    down: AtomicBool,
    before: Mutex<Vec<String>>,
    after: Mutex<Vec<(String, Option<Origin>)>>,
}

#[derive(Clone, Default)]
struct Recorder(Arc<Seen>);

impl Hooks for Recorder {
    fn before_commit<'a>(&'a self, db: &'a mut PgConnection, event: &'a Event) -> Work<'a, ()> {
        Box::pin(async move {
            let audited: i64 = query_scalar("SELECT count(*) FROM ledger.audit WHERE org_id = $1")
                .bind(event.org.get())
                .fetch_one(db)
                .await?;
            let line = format!("{} after {audited} audit rows", event.action);
            self.0.before.lock().unwrap_or_else(PoisonError::into_inner).push(line);
            let refuse = *self.0.refuse.lock().unwrap_or_else(PoisonError::into_inner);
            match refuse {
                Some(action) if action == event.action => Err(Error::HookRefused(format!("no {action}"))),
                _ => Ok(()),
            }
        })
    }

    fn after_commit<'a>(&'a self, event: &'a Event) -> Work<'a, ()> {
        Box::pin(async move {
            if self.0.down.load(Ordering::SeqCst) {
                return Err(Error::Host("the outbox is down".to_owned()));
            }
            let mut after = self.0.after.lock().unwrap_or_else(PoisonError::into_inner);
            after.push((event.action.clone(), event.origin.clone()));
            Ok(())
        })
    }
}

impl Recorder {
    fn after(&self) -> Vec<(String, Option<Origin>)> {
        self.0.after.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn actions(&self) -> Vec<String> {
        self.after().into_iter().map(|(action, _)| action).collect()
    }
}

fn email(text: &str) -> Res<Email> {
    Ok(Email::new(text)?)
}

fn ann() -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(1)?, email("ann@example.com")?))
}

fn key(text: &str) -> Res<Option<IdempotencyKey>> {
    Ok(Some(IdempotencyKey::new(text)?))
}

async fn books(pool: &PgPool) -> Res<(Openbooks<TestHost>, Recorder, OrgId)> {
    openbooks_db::migrate(pool).await?;
    let recorder = Recorder::default();
    let books = Openbooks::new(TestHost::new(pool.clone()))
        .hooks(recorder.clone())
        .build();
    books.host().sign_in(ann()?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, "none", None)
        .await?;
    Ok((books, recorder, org))
}

async fn counts(pool: &PgPool) -> Res<(i64, i64, i64)> {
    let sql = "SELECT (SELECT count(*) FROM ledger.member), (SELECT count(*) FROM ledger.audit), \
               (SELECT count(*) FROM ledger.event e JOIN ledger.audit a USING (org_id, audit_id) \
                WHERE e.action = a.action AND e.detail = a.detail)";
    Ok(query_as(sql).fetch_one(pool).await?)
}

#[sqlx::test(migrations = false)]
async fn a_refusal_before_commit_rolls_the_change_back_and_leaves_no_row(pool: PgPool) -> Res<()> {
    let (books, recorder, org) = books(&pool).await?;
    *recorder.0.refuse.lock().unwrap_or_else(PoisonError::into_inner) = Some("member.add");
    let added = books
        .add_member(org, email("bob@example.com")?, Role::Staff, None)
        .await;
    assert!(
        matches!(&added, Err(Error::HookRefused(why)) if why == "no member.add"),
        "{added:?}"
    );
    assert_eq!(counts(&pool).await?, (1, 1, 1));
    assert!(
        books
            .members(org, None)
            .await?
            .items
            .iter()
            .all(|m| m.email.as_str() != "bob@example.com")
    );
    *recorder.0.refuse.lock().unwrap_or_else(PoisonError::into_inner) = Some("company.create");
    let opened = books
        .create_company("Other", Currency::new("XTS", 2)?, "none", None)
        .await;
    assert!(matches!(opened, Err(Error::HookRefused(_))), "{opened:?}");
    let orgs: i64 = query_scalar("SELECT count(*) FROM ledger.org").fetch_one(&pool).await?;
    assert_eq!((orgs, counts(&pool).await?), (1, (1, 1, 1)));
    let before = recorder.0.before.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let lines = [
        "company.create after 1 audit rows",
        "member.add after 2 audit rows",
        "company.create after 1 audit rows",
    ];
    assert_eq!(before, lines.map(str::to_owned));
    assert_eq!(recorder.actions(), ["company.create"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_create_sent_twice_with_its_key_writes_once_and_another_body_is_refused(pool: PgPool) -> Res<()> {
    let (books, recorder, org) = books(&pool).await?;
    let bob = email("bob@example.com")?;
    for _ in 0..2 {
        books.add_member(org, bob.clone(), Role::Staff, key("add-bob")?).await?;
    }
    assert_eq!(counts(&pool).await?, (2, 2, 2));
    let other = books.add_member(org, bob.clone(), Role::Viewer, key("add-bob")?).await;
    assert!(matches!(other, Err(Error::KeyReused)), "{other:?}");
    let unkeyed = books.add_member(org, bob, Role::Staff, None).await;
    assert!(matches!(unkeyed, Err(Error::AlreadyMember(_))), "{unkeyed:?}");
    let base = Currency::new("XTS", 2)?;
    let first = books.create_company("Beta", base, "none", key("open-beta")?).await?;
    assert_eq!(
        books.create_company("Beta", base, "none", key("open-beta")?).await?,
        first
    );
    let renamed = books.create_company("Gamma", base, "none", key("open-beta")?).await;
    assert!(matches!(renamed, Err(Error::KeyReused)), "{renamed:?}");
    let orgs: i64 = query_scalar("SELECT count(*) FROM ledger.org").fetch_one(&pool).await?;
    assert_eq!((orgs, counts(&pool).await?), (2, (3, 3, 3)));
    let actions = ["company.create", "member.add", "company.create"];
    assert_eq!(recorder.actions(), actions.map(str::to_owned));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn after_commit_gets_each_event_once_in_order_and_a_failed_one_is_retried(pool: PgPool) -> Res<()> {
    let (books, recorder, org) = books(&pool).await?;
    recorder.0.down.store(true, Ordering::SeqCst);
    books
        .add_member(org, email("bob@example.com")?, Role::Staff, None)
        .await?;
    books.change_role(org, email("bob@example.com")?, Role::Viewer).await?;
    assert!(matches!(books.deliver(org).await, Err(Error::Host(_))));
    assert_eq!(recorder.actions(), ["company.create"]);
    recorder.0.down.store(false, Ordering::SeqCst);
    books.remove_member(org, email("bob@example.com")?).await?;
    books.deliver(org).await?;
    let actions = ["company.create", "member.add", "member.role", "member.remove"];
    assert_eq!(recorder.actions(), actions.map(str::to_owned));
    assert_eq!(counts(&pool).await?, (2, 4, 4));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_change_the_host_makes_carries_its_origin_to_the_event(pool: PgPool) -> Res<()> {
    let (books, recorder, org) = books(&pool).await?;
    let origin = Origin::new("billing:invoice-42")?;
    books.host().sign_in(SignedIn {
        via: Via::Host(origin.clone()),
        ..ann()?
    });
    books
        .add_member(org, email("bob@example.com")?, Role::Staff, None)
        .await?;
    let stored: Vec<(Option<String>, String)> = query_as(
        "SELECT e.origin, a.via FROM ledger.event e JOIN ledger.audit a USING (org_id, audit_id) ORDER BY e.event_id",
    )
    .fetch_all(&pool)
    .await?;
    let host = (Some("billing:invoice-42".to_owned()), "host".to_owned());
    assert_eq!(stored, [(None, "person".to_owned()), host]);
    assert_eq!(recorder.after().last(), Some(&("member.add".to_owned(), Some(origin))));
    assert!(Origin::new("").is_err() && Origin::new("two words").is_err() && IdempotencyKey::new("").is_err());
    Ok(())
}
