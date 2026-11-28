use openbooks::testing::{TestHost, TestPack};
use openbooks::{Account, Dimension, DimensionValue, Error, Openbooks, RoleAccount, SignedIn, Terms};
use openbooks_core::{AccountType, Currency, Date, Email, IdempotencyKey, OrgId, Role, UserId, Word};
use sqlx::{PgPool, query, query_scalar};

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

fn account(code: &str, account_type: AccountType, parent: Option<&str>) -> Account {
    Account {
        code: code.to_owned(),
        name: format!("Account {code}"),
        account_type,
        parent: parent.map(str::to_owned),
        active: true,
        currency: None,
    }
}

async fn audits(pool: &PgPool, org: OrgId) -> Res<i64> {
    let sql = "SELECT count(*) FROM ledger.audit WHERE org_id = $1";
    Ok(query_scalar(sql).bind(org.get()).fetch_one(pool).await?)
}

type Posting<'a> = (&'a str, i64, &'a [(i16, &'a str)]);

async fn post(pool: &PgPool, org: OrgId, lines: &[Posting<'_>], posted: bool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let entry: i64 = query_scalar(
        "INSERT INTO ledger.entry (org_id, book, entry_date) VALUES ($1, 'general', '2026-11-01') RETURNING entry_id",
    )
    .bind(org.get())
    .fetch_one(&mut *tx)
    .await?;
    for (n, (code, minor, values)) in (1_i32..).zip(lines) {
        query(
            "INSERT INTO ledger.line (org_id, entry_id, line_no, account_id, currency, amount_minor, base_minor) \
             SELECT $1, $2, $3, account_id, 'XTS', $4, $4 FROM ledger.account WHERE org_id = $1 AND code = $5",
        )
        .bind(org.get())
        .bind(entry)
        .bind(n)
        .bind(minor)
        .bind(code)
        .execute(&mut *tx)
        .await?;
        for (slot, value) in *values {
            query(
                "INSERT INTO ledger.line_dimension (org_id, entry_id, line_no, kind_id, value_id) \
                 SELECT v.org_id, $2, $3, v.kind_id, v.value_id FROM ledger.dimension_value v \
                 JOIN ledger.dimension k USING (org_id, kind_id) \
                 WHERE v.org_id = $1 AND k.slot = $4 AND NOT k.removed AND v.code = $5",
            )
            .bind(org.get())
            .bind(entry)
            .bind(n)
            .bind(slot)
            .bind(value)
            .execute(&mut *tx)
            .await?;
        }
    }
    if posted {
        query("UPDATE ledger.entry SET posted_at = now() WHERE org_id = $1 AND entry_id = $2")
            .bind(org.get())
            .bind(entry)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

fn said(outcome: Result<(), sqlx::Error>) -> Option<String> {
    outcome
        .err()
        .map(|e| e.as_database_error().map_or(e.to_string(), |d| d.message().to_owned()))
}

#[sqlx::test(migrations = false)]
async fn an_account_with_postings_keeps_its_type_and_none_sits_under_itself(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    for saved in [
        account("1000", AccountType::Asset, None),
        account("1100", AccountType::Asset, Some("1000")),
        account("4000", AccountType::Income, None),
    ] {
        books.save_account(org, saved).await?;
    }
    post(&pool, org, &[("1100", 500, &[]), ("4000", -500, &[])], true).await?;
    let before = audits(&pool, org).await?;
    let retyped = books
        .save_account(org, account("1100", AccountType::Liability, Some("1000")))
        .await;
    assert!(
        matches!(&retyped, Err(Error::AccountPosted(code)) if code == "1100"),
        "{retyped:?}"
    );
    let looped = books
        .save_account(org, account("1000", AccountType::Asset, Some("1100")))
        .await;
    assert!(
        matches!(&looped, Err(Error::ParentLoop(code)) if code == "1000"),
        "{looped:?}"
    );
    let itself = books
        .save_account(org, account("1000", AccountType::Asset, Some("1000")))
        .await;
    assert!(matches!(itself, Err(Error::ParentLoop(_))), "{itself:?}");
    let orphan = books
        .save_account(org, account("1200", AccountType::Asset, Some("9999")))
        .await;
    assert!(
        matches!(&orphan, Err(Error::NoSuchAccount(code)) if code == "9999"),
        "{orphan:?}"
    );
    let currency = Account {
        currency: Some("usd".to_owned()),
        ..account("1300", AccountType::Asset, None)
    };
    assert!(books.save_account(org, currency).await.is_err());
    assert_eq!(audits(&pool, org).await?, before);
    let renamed = Account {
        name: "Bank".to_owned(),
        active: false,
        currency: Some("XTS".to_owned()),
        ..account("1100", AccountType::Asset, None)
    };
    books.save_account(org, renamed.clone()).await?;
    books
        .save_account(org, account("1000", AccountType::Expense, None))
        .await?;
    assert_eq!(audits(&pool, org).await?, before + 2);
    let chart = books.accounts(org, None).await?;
    let expense = account("1000", AccountType::Expense, None);
    let income = account("4000", AccountType::Income, None);
    assert_eq!((chart.items, chart.next), (vec![expense, renamed, income], None));
    Ok(())
}

async fn give(books: &Openbooks<TestHost>, org: OrgId, role: &str, day: (u16, u8, u8), code: &str) -> Res<()> {
    let from = Date::new(day.0, day.1, day.2)?;
    Ok(books.give_role(org, role.to_owned(), from, code.to_owned()).await?)
}

#[sqlx::test(migrations = false)]
async fn a_role_goes_to_an_account_from_a_day_only_when_the_core_or_the_companys_pack_declares_it(
    pool: PgPool,
) -> Res<()> {
    let (books, taxed) = books(&pool, "test").await?;
    let plain = books
        .create_company("Plain", Currency::new("XTS", 2)?, "none", None)
        .await?;
    for org in [taxed, plain] {
        books
            .save_account(org, account("1100", AccountType::Asset, None))
            .await?;
        books
            .save_account(org, account("1200", AccountType::Asset, None))
            .await?;
    }
    give(&books, taxed, "receivables", (2026, 1, 1), "1100").await?;
    give(&books, taxed, "receivables", (2026, 7, 1), "1200").await?;
    give(&books, taxed, "added_tax", (2026, 1, 1), "1200").await?;
    let before = audits(&pool, plain).await?;
    for (role, code) in [("added_tax", "1200"), ("made_up", "1200")] {
        let refused = give(&books, plain, role, (2026, 1, 1), code)
            .await
            .map_err(|e| e.to_string());
        let why = format!("neither the core nor the company's pack declares the role {role}");
        assert_eq!(refused, Err(why));
    }
    let missing = books
        .give_role(plain, "payables".to_owned(), Date::new(2026, 1, 1)?, "9999".to_owned())
        .await;
    assert!(
        matches!(&missing, Err(Error::NoSuchAccount(code)) if code == "9999"),
        "{missing:?}"
    );
    assert_eq!(audits(&pool, plain).await?, before);
    let on = |month| -> Res<Vec<RoleAccount>> {
        let role = |role: &str, from_month, account: &str| -> Res<RoleAccount> {
            let (role, account) = (role.to_owned(), account.to_owned());
            Ok(RoleAccount {
                role,
                from: Date::new(2026, from_month, 1)?,
                account,
            })
        };
        let receivables = if month < 7 { (1, "1100") } else { (7, "1200") };
        Ok(vec![
            role("added_tax", 1, "1200")?,
            role("receivables", receivables.0, receivables.1)?,
        ])
    };
    for month in [3, 8] {
        let seen = books.account_roles(taxed, Date::new(2026, month, 15)?).await?;
        assert_eq!(seen.items, on(month)?);
    }
    assert_eq!(books.account_roles(taxed, Date::new(2025, 12, 31)?).await?.items, []);
    Ok(())
}

fn kinds(names: &[&str]) -> Vec<Dimension> {
    (1..)
        .zip(names)
        .map(|(slot, name)| Dimension {
            slot,
            name: (*name).to_owned(),
        })
        .collect()
}

#[sqlx::test(migrations = false)]
async fn a_company_has_project_and_up_to_two_kinds_of_its_own_and_a_posted_kind_is_only_renamed(
    pool: PgPool,
) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    assert_eq!(books.dimensions(org).await?.items, kinds(&["Project"]));
    let key = IdempotencyKey::new("cc-1")?;
    books
        .add_dimension(org, "Cost centre".to_owned(), Some(key.clone()))
        .await?;
    books.add_dimension(org, "Cost centre".to_owned(), Some(key)).await?;
    books.add_dimension(org, "Region".to_owned(), None).await?;
    let before = audits(&pool, org).await?;
    let fourth = books.add_dimension(org, "Department".to_owned(), None).await;
    assert!(matches!(fourth, Err(Error::DimensionsFull)), "{fourth:?}");
    let project = books.rename_dimension(org, 1, "Job".to_owned()).await;
    assert!(matches!(project, Err(Error::ProjectDimension)), "{project:?}");
    assert!(matches!(
        books.remove_dimension(org, 1).await,
        Err(Error::ProjectDimension)
    ));
    assert!(matches!(
        books.remove_dimension(org, 4).await,
        Err(Error::NoSuchDimension(4))
    ));
    books
        .save_account(org, account("1000", AccountType::Asset, None))
        .await?;
    books
        .save_account(org, account("4000", AccountType::Income, None))
        .await?;
    books
        .save_value(org, 2, "CC1".to_owned(), "Kitchen".to_owned(), true)
        .await?;
    books
        .save_value(org, 3, "R1".to_owned(), "North".to_owned(), true)
        .await?;
    post(&pool, org, &[("1000", 500, &[]), ("4000", -500, &[(2, "CC1")])], true).await?;
    let after_setup = audits(&pool, org).await?;
    let posted = books.remove_dimension(org, 2).await;
    assert!(matches!(posted, Err(Error::DimensionPosted(2))), "{posted:?}");
    assert_eq!(audits(&pool, org).await?, after_setup);
    books.rename_dimension(org, 2, "Cost center".to_owned()).await?;
    books.remove_dimension(org, 3).await?;
    assert_eq!(books.dimensions(org).await?.items, kinds(&["Project", "Cost center"]));
    books.add_dimension(org, "Department".to_owned(), None).await?;
    assert_eq!(books.dimension_values(org, 3, None).await?.items, []);
    assert_eq!(
        books.dimensions(org).await?.items,
        kinds(&["Project", "Cost center", "Department"])
    );
    assert_eq!(audits(&pool, org).await?, after_setup + 3);
    assert_eq!(before + 4, after_setup);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_line_carries_one_value_per_kind_and_each_kind_its_account_requires(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    books.add_dimension(org, "Department".to_owned(), None).await?;
    books
        .save_account(org, account("1000", AccountType::Asset, None))
        .await?;
    books
        .save_account(org, account("5000", AccountType::Expense, None))
        .await?;
    for (slot, code) in [(1, "P1"), (2, "D1"), (2, "D2")] {
        books
            .save_value(org, slot, code.to_owned(), code.to_owned(), true)
            .await?;
    }
    let twice = post(&pool, org, &[("5000", 0, &[(2, "D1"), (2, "D2")])], false).await;
    assert!(said(twice).is_some_and(|why| why.contains("line_dimension_pkey")));
    books.require_dimension(org, "5000".to_owned(), 2, true).await?;
    let missing = post(&pool, org, &[("1000", 500, &[]), ("5000", -500, &[(1, "P1")])], true).await;
    let why = "entry 2 has a line without a dimension its account requires".to_owned();
    assert_eq!(said(missing), Some(why));
    post(
        &pool,
        org,
        &[("1000", 500, &[]), ("5000", -500, &[(1, "P1"), (2, "D1")])],
        true,
    )
    .await?;
    books.require_dimension(org, "5000".to_owned(), 2, false).await?;
    post(&pool, org, &[("1000", 500, &[]), ("5000", -500, &[])], true).await?;
    let required = books.require_dimension(org, "5000".to_owned(), 3, true).await;
    assert!(matches!(required, Err(Error::NoSuchDimension(3))), "{required:?}");
    let snapshot: Option<String> = query_scalar(
        "SELECT after::text FROM ledger.audit WHERE org_id = $1 AND action = 'account.require' ORDER BY seq LIMIT 1",
    )
    .bind(org.get())
    .fetch_one(&pool)
    .await?;
    assert!(snapshot.is_some_and(|s| s.contains(r#""requires": [2]"#)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_value_the_host_syncs_by_its_reference_is_read_only_by_hand(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    let value = |code: &str, name: &str, host_ref: Option<&str>| DimensionValue {
        code: code.to_owned(),
        name: name.to_owned(),
        active: true,
        host_ref: host_ref.map(str::to_owned),
    };
    books
        .sync_value(org, 1, "job-7".to_owned(), value("P7", "Bridge", None))
        .await?;
    books
        .sync_value(org, 1, "job-7".to_owned(), value("P7A", "Bridge works", None))
        .await?;
    books
        .save_value(org, 1, "P8".to_owned(), "Tunnel".to_owned(), true)
        .await?;
    books
        .save_value(org, 1, "P8".to_owned(), "Tunnel works".to_owned(), true)
        .await?;
    let before = audits(&pool, org).await?;
    let by_hand = books
        .save_value(org, 1, "P7A".to_owned(), "Mine now".to_owned(), true)
        .await;
    assert!(
        matches!(&by_hand, Err(Error::Synced(code)) if code == "P7A"),
        "{by_hand:?}"
    );
    assert_eq!(audits(&pool, org).await?, before);
    let listed = books.dimension_values(org, 1, None).await?.items;
    let synced = value("P7A", "Bridge works", Some("job-7"));
    assert_eq!(listed, [synced, value("P8", "Tunnel works", None)]);
    assert!(matches!(books.dimension_values(org, 2, None).await, Ok(page) if page.items.is_empty()));
    assert!(matches!(
        books.save_value(org, 0, "X".to_owned(), "X".to_owned(), true).await,
        Err(Error::NoSuchDimension(0))
    ));
    assert_eq!(before, 5);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn terms_are_saved_by_code_and_the_setup_needs_approve_while_reading_needs_read(pool: PgPool) -> Res<()> {
    let (books, org) = books(&pool, "none").await?;
    let terms = |days| Terms {
        code: "NET".to_owned(),
        name: "Net".to_owned(),
        days,
        active: true,
    };
    books.save_terms(org, terms(30)).await?;
    books.save_terms(org, terms(45)).await?;
    assert_eq!(books.terms(org, None).await?.items, [terms(45)]);
    for (n, role) in (2..).zip([Role::Staff, Role::Viewer, Role::Accountant]) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, Email::new(&address)?, role, None).await?;
        books.host().sign_in(person(n, &address)?);
        assert_eq!(books.accounts(org, None).await?.items, []);
        let saved = books.save_terms(org, terms(60)).await;
        let added = books.add_dimension(org, format!("Kind {n}"), None).await;
        for outcome in [saved, added] {
            match outcome {
                Ok(()) => assert!(role.allows(Word::Approve), "{role:?} set up the books"),
                Err(Error::Forbidden { word }) => assert!(!role.allows(word) && word == Word::Approve),
                Err(other) => return Err(other.into()),
            }
        }
    }
    assert_eq!(books.terms(org, None).await?.items, [terms(60)]);
    Ok(())
}
