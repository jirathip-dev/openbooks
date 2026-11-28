use std::future::{Future, poll_fn};
use std::pin::pin;
use std::task::Poll;

use openbooks::testing::{TestHost, TestPack};
use openbooks::{Book, Error, NumberSeries, Openbooks, SignedIn};
use openbooks_core::{BookKind, Currency, Date, Email, IdempotencyKey, OrgId, Role, UserId, Word};
use openbooks_db::books::take_number;
use sqlx::{PgConnection, PgPool, query_scalar};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, Email::new(address)?))
}

async fn books(pool: &PgPool, pack: &str) -> Res<(Openbooks<TestHost>, OrgId)> {
    let books = Openbooks::new(TestHost::new(pool.clone()))
        .pack(TestPack::default())
        .build();
    books.migrate(pool).await?;
    books.host().sign_in(person(1, "ann@example.com")?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, pack, None)
        .await?;
    Ok((books, org))
}

async fn audits(pool: &PgPool, org: OrgId) -> Res<i64> {
    let sql = "SELECT count(*) FROM ledger.audit WHERE org_id = $1";
    Ok(query_scalar(sql).bind(org.get()).fetch_one(pool).await?)
}

async fn given(pool: &PgPool, org: OrgId) -> Res<Vec<String>> {
    let sql = "SELECT number FROM ledger.number WHERE org_id = $1 ORDER BY number";
    Ok(query_scalar(sql).bind(org.get()).fetch_all(pool).await?)
}

fn november() -> Res<Date> {
    Ok(Date::new(2026, 11, 24)?)
}

async fn take(db: &mut PgConnection, org: OrgId, document: &str, book: Option<BookKind>) -> Res<String> {
    let number = take_number(db, org, document, book, november()?).await?;
    Ok(number.ok_or("no series")?)
}

async fn approve(pool: &PgPool, org: OrgId) -> Res<String> {
    let mut tx = pool.begin().await?;
    let number = take(&mut tx, org, "journal", Some(BookKind::General)).await?;
    tx.commit().await?;
    Ok(number)
}

async fn waits_on_a_lock(pool: &PgPool) -> Res<bool> {
    let sql = "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock')";
    Ok(query_scalar(sql).fetch_one(pool).await?)
}

async fn both<A: Future, B: Future>(a: A, b: B) -> Option<(A::Output, B::Output)> {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut left, mut right) = (None, None);
    poll_fn(|cx| {
        if left.is_none()
            && let Poll::Ready(out) = a.as_mut().poll(cx)
        {
            left = Some(out);
        }
        if right.is_none()
            && let Poll::Ready(out) = b.as_mut().poll(cx)
        {
            right = Some(out);
        }
        if left.is_some() && right.is_some() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    left.zip(right)
}

#[sqlx::test(migrations = false)]
async fn two_concurrent_approvals_take_consecutive_numbers_and_a_rolled_back_one_burns_none(pool: PgPool) -> Res<()> {
    let (_books, org) = books(&pool, "none").await?;
    let mut first = pool.begin().await?;
    assert_eq!(
        take(&mut first, org, "journal", Some(BookKind::General)).await?,
        "JV2611-0001"
    );
    let finish_first = async {
        for _ in 0..10_000 {
            if waits_on_a_lock(&pool).await? {
                first.commit().await?;
                return Ok(true);
            }
        }
        first.commit().await?;
        Ok::<_, Box<dyn std::error::Error>>(false)
    };
    let (blocked, second) = both(finish_first, approve(&pool, org)).await.ok_or("unfinished")?;
    assert_eq!((blocked?, second?), (true, "JV2611-0002".to_owned()));
    let mut undone = pool.begin().await?;
    assert_eq!(
        take(&mut undone, org, "journal", Some(BookKind::General)).await?,
        "JV2611-0003"
    );
    undone.rollback().await?;
    assert_eq!(approve(&pool, org).await?, "JV2611-0003");
    let mut december = pool.begin().await?;
    let next = take_number(
        &mut december,
        org,
        "journal",
        Some(BookKind::General),
        Date::new(2026, 12, 1)?,
    )
    .await?;
    december.commit().await?;
    assert_eq!(next.as_deref(), Some("JV2612-0001"));
    assert_eq!(
        given(&pool, org).await?,
        ["JV2611-0001", "JV2611-0002", "JV2611-0003", "JV2612-0001"]
    );
    Ok(())
}

fn book(kind: BookKind, code: &str, order: u16, name: &str) -> Book {
    Book {
        kind,
        code: code.to_owned(),
        order,
        name: name.to_owned(),
    }
}

#[sqlx::test(migrations = false)]
async fn a_company_starts_with_its_packs_five_books_and_renaming_one_changes_no_number_given(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "test").await?;
    let started = books.books(org).await?.items;
    assert_eq!(
        started,
        [
            book(BookKind::General, "JV", 1, "General journal"),
            book(BookKind::Receipts, "RV", 2, "Receipts"),
            book(BookKind::Payments, "PV", 3, "Payments"),
            book(BookKind::Purchases, "AP", 4, "Purchases"),
            book(BookKind::Sales, "AR", 5, "Sales"),
        ]
    );
    let mut tx = pool.begin().await?;
    let numbers = [
        take(&mut tx, org, "invoice", Some(BookKind::Sales)).await?,
        take(&mut tx, org, "credit_note", Some(BookKind::Sales)).await?,
        take(&mut tx, org, "quotation", None).await?,
    ];
    assert_eq!(take_number(&mut tx, org, "order", None, november()?).await?, None);
    tx.commit().await?;
    assert_eq!(numbers, ["TI2611-0001", "AR2611-0001", "QT2611-0001"]);
    let before = audits(&pool, org).await?;
    books
        .save_book(org, book(BookKind::Sales, "SL", 1, "Sales book"))
        .await?;
    let taken = books.save_book(org, book(BookKind::General, "SL", 2, "General")).await;
    assert!(matches!(&taken, Err(Error::CodeTaken(c)) if c == "SL"), "{taken:?}");
    let bad = books.save_book(org, book(BookKind::General, "J-V", 2, "General")).await;
    assert!(matches!(bad, Err(Error::BadCode(_))), "{bad:?}");
    assert_eq!(audits(&pool, org).await?, before + 1);
    let listed = books.books(org).await?.items;
    assert_eq!(listed.first(), Some(&book(BookKind::Sales, "SL", 1, "Sales book")));
    assert_eq!(given(&pool, org).await?, ["AR2611-0001", "QT2611-0001", "TI2611-0001"]);
    let mut tx = pool.begin().await?;
    let next = take(&mut tx, org, "credit_note", Some(BookKind::Sales)).await?;
    tx.commit().await?;
    assert_eq!(next, "AR2611-0002");
    let after: String = query_scalar(
        "SELECT after::text FROM ledger.audit WHERE org_id = $1 AND action = 'book.save' ORDER BY seq DESC LIMIT 1",
    )
    .bind(org.get())
    .fetch_one(&pool)
    .await?;
    assert!(after.contains("\"code\": \"SL\""), "{after}");
    Ok(())
}

fn series(code: &str, book: Option<BookKind>, document: Option<&str>, year_offset: u16) -> NumberSeries {
    NumberSeries {
        code: code.to_owned(),
        book,
        document: document.map(str::to_owned),
        year_offset,
    }
}

#[sqlx::test(migrations = false)]
async fn two_series_cannot_share_a_code_and_a_year_offset_stays_once_numbers_are_given(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "test").await?;
    let before = audits(&pool, org).await?;
    let receipt = Some(BookKind::Receipts);
    let key = IdempotencyKey::new("series-1")?;
    for _ in 0..2 {
        books
            .add_series(org, "OR".into(), "receipt".into(), receipt, 0, Some(key.clone()))
            .await?;
    }
    let shared = books
        .add_series(org, "OR".into(), "deposit".into(), receipt, 0, None)
        .await;
    assert!(matches!(&shared, Err(Error::CodeTaken(c)) if c == "OR"), "{shared:?}");
    let book_code = books
        .add_series(org, "TI".into(), "deposit".into(), receipt, 0, None)
        .await;
    assert!(matches!(book_code, Err(Error::CodeTaken(_))), "{book_code:?}");
    let twice = books
        .add_series(org, "OR2".into(), "invoice".into(), None, 0, None)
        .await;
    assert!(
        matches!(&twice, Err(Error::DocumentHasSeries(d)) if d == "invoice"),
        "{twice:?}"
    );
    books.set_year_offset(org, "OR".into(), 543).await?;
    assert_eq!(audits(&pool, org).await?, before + 2);
    assert_eq!(
        books.series(org, None).await?.items,
        [
            series("AP", Some(BookKind::Purchases), None, 0),
            series("AR", Some(BookKind::Sales), None, 0),
            series("JV", Some(BookKind::General), None, 0),
            series("OR", receipt, Some("receipt"), 543),
            series("PV", Some(BookKind::Payments), None, 0),
            series("QT", None, Some("quotation"), 0),
            series("RV", receipt, None, 0),
            series("TI", Some(BookKind::Sales), Some("invoice"), 0),
        ]
    );
    let mut tx = pool.begin().await?;
    let number = take(&mut tx, org, "receipt", receipt).await?;
    tx.commit().await?;
    assert_eq!(number, "OR6911-0001");
    let stays = books.set_year_offset(org, "OR".into(), 0).await;
    assert!(
        matches!(&stays, Err(Error::SeriesNumbered(c)) if c == "OR"),
        "{stays:?}"
    );
    let missing = books.set_year_offset(org, "ZZ".into(), 1).await;
    assert!(matches!(missing, Err(Error::NoSuchSeries(_))), "{missing:?}");
    assert_eq!(audits(&pool, org).await?, before + 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn books_and_series_are_set_with_approve_and_read_with_read(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    let staff = Email::new("sam@example.com")?;
    books.add_member(org, staff, Role::Staff, None).await?;
    books.host().sign_in(person(2, "sam@example.com")?);
    assert_eq!(books.books(org).await?.items.len(), 5);
    assert_eq!(books.series(org, None).await?.items.len(), 5);
    let listed: serde_json::Value = serde_json::from_str(&books.call_tool(org, "books", &[]).await?)?;
    assert_eq!(
        listed.pointer("/0"),
        Some(&serde_json::json!({"kind": "general", "code": "JV", "order": 1, "name": "General journal"}))
    );
    let paged: serde_json::Value = serde_json::from_str(&books.call_tool(org, "series", &[]).await?)?;
    assert_eq!(
        paged.pointer("/items/0"),
        Some(&serde_json::json!({"code": "AP", "book": "purchases", "year_offset": 0}))
    );
    let saved = books.save_book(org, book(BookKind::General, "GJ", 1, "General")).await;
    assert!(
        matches!(saved, Err(Error::Forbidden { word: Word::Approve })),
        "{saved:?}"
    );
    let added = books
        .add_series(org, "QT".into(), "quotation".into(), None, 0, None)
        .await;
    assert!(
        matches!(added, Err(Error::Forbidden { word: Word::Approve })),
        "{added:?}"
    );
    let offset = books.set_year_offset(org, "JV".into(), 1).await;
    assert!(
        matches!(offset, Err(Error::Forbidden { word: Word::Approve })),
        "{offset:?}"
    );
    Ok(())
}
