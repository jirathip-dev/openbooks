use std::collections::BTreeSet;

use openbooks::testing::TestHost;
use openbooks::{BOOKS, Error, Member, Openbooks, Operation, Scope, SignedIn, Via};
use openbooks_core::{Currency, Email, OrgId, ProjectId, Role, UserId, Word};
use sqlx::{PgPool, query_scalar};

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
    let org = books.create_company("Acme", Currency::new("XTS", 2)?).await?;
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
    books.create_company("Acme", Currency::new("XTS", 2)?).await?;
    for table in ["org", "member", "audit"] {
        assert_eq!(rows(&pool, table).await?, 0, "{table}");
    }
    books.host().roll_back(false);
    let org = books.create_company("Acme", Currency::new("XTS", 2)?).await?;
    books.host().roll_back(true);
    books.add_member(org, email("bob@example.com")?, Role::Staff).await?;
    assert_eq!(
        (rows(&pool, "member").await?, audits(&pool).await?),
        (1, vec!["company.create".to_owned()])
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn each_operation_is_declared_with_its_word_and_each_role_gets_what_its_words_allow(pool: PgPool) -> Res<()> {
    let words = [Word::Read, Word::Own, Word::Own, Word::Own];
    assert_eq!(BOOKS.operations.iter().map(Operation::word).collect::<Vec<_>>(), words);
    let (books, org) = books(pool).await?;
    for (n, role) in (2..).zip(Role::ALL) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, email(&address)?, role).await?;
        books.host().sign_in(person(n, &address)?);
        let read = books.members(org, None).await.map(drop);
        let own = books
            .add_member(org, email(&format!("new{n}@example.com"))?, Role::Viewer)
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
        let added = books.add_member(org, email("bob@example.com")?, Role::Staff).await;
        assert!(matches!(added, Err(Error::Forbidden { word: Word::Own })), "{added:?}");
    }
    assert!(projects.covers(ProjectId::new(7)?) && !projects.covers(ProjectId::new(8)?));
    books.host().sign_in(ann);
    books.add_member(org, email("vic@example.com")?, Role::Viewer).await?;
    books.host().sign_in(SignedIn {
        via: Via::Host,
        ..person(3, "vic@example.com")?
    });
    let added = books.add_member(org, email("eve@example.com")?, Role::Owner).await;
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
    books.add_member(org, email("Bob@Example.com")?, Role::Staff).await?;
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
    books.add_member(org, bob.clone(), Role::Owner).await?;
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
            .add_member(org, email(&format!("m{n:03}@example.com"))?, Role::Viewer)
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
