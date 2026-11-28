use openbooks::testing::{TestHost, TestPack};
use openbooks::{Account, Error, Item, LineDefaults, Openbooks, Side, SignedIn, Unit};
use openbooks_core::{AccountType, Currency, Email, ItemKind, OrgId, Quantity, Refusal, Role, UserId, Word};
use sqlx::{PgPool, query, query_scalar};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, Email::new(address)?))
}

async fn open(pool: &PgPool, pack: &str) -> Res<(Openbooks<TestHost>, OrgId)> {
    let books = Openbooks::new(TestHost::new(pool.clone()))
        .pack(TestPack::default())
        .build();
    books.migrate(pool).await?;
    books.host().sign_in(person(1, "ann@example.com")?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, pack, None)
        .await?;
    for (code, account_type) in [("1000", AccountType::Asset), ("4000", AccountType::Income)] {
        let account = Account {
            code: code.to_owned(),
            name: format!("Account {code}"),
            account_type,
            parent: None,
            active: true,
            currency: None,
        };
        books.save_account(org, account).await?;
    }
    for code in ["EA", "BOX"] {
        let unit = Unit {
            code: code.to_owned(),
            name: format!("Unit {code}"),
        };
        books.save_unit(org, unit).await?;
    }
    Ok((books, org))
}

fn item(code: &str, kind: ItemKind, unit: &str) -> Item {
    Item {
        code: code.to_owned(),
        name: format!("Item {code}"),
        kind,
        unit: unit.to_owned(),
        sales_account: Some("4000".to_owned()),
        purchase_account: None,
        sales_tax: Some("T7".to_owned()),
        purchase_tax: None,
        weight: Some(Quantity(12_500)),
        volume: None,
        host_ref: None,
    }
}

async fn audits(pool: &PgPool, org: OrgId) -> Res<Vec<String>> {
    let sql = "SELECT record FROM ledger.audit WHERE org_id = $1 ORDER BY seq";
    Ok(query_scalar(sql).bind(org.get()).fetch_all(pool).await?)
}

async fn post(pool: &PgPool, org: OrgId, item: &str, posted: bool) -> Res<()> {
    let mut tx = pool.begin().await?;
    let entry: i64 = query_scalar(
        "INSERT INTO ledger.entry (org_id, book, entry_date) VALUES ($1, 'general', '2026-11-01') RETURNING entry_id",
    )
    .bind(org.get())
    .fetch_one(&mut *tx)
    .await?;
    for (n, code, minor, item) in [(1, "1000", 500, None), (2, "4000", -500, Some(item))] {
        query(
            "INSERT INTO ledger.line (org_id, entry_id, line_no, account_id, currency, amount_minor, base_minor, item_id) \
             SELECT $1, $2, $3, a.account_id, 'XTS', $4, $4, \
             (SELECT item_id FROM ledger.item WHERE org_id = $1 AND code = $6) \
             FROM ledger.account a WHERE a.org_id = $1 AND a.code = $5",
        )
        .bind(org.get())
        .bind(entry)
        .bind(n)
        .bind(minor)
        .bind(code)
        .bind(item)
        .execute(&mut *tx)
        .await?;
    }
    if posted {
        query("UPDATE ledger.entry SET posted_at = now() WHERE org_id = $1 AND entry_id = $2")
            .bind(org.get())
            .bind(entry)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let sql = "SELECT count(*) FROM ledger.line l JOIN ledger.entry e USING (org_id, entry_id) \
         WHERE l.org_id = $1 AND e.book <> 'general'";
    let sub_ledger: i64 = query_scalar(sql).bind(org.get()).fetch_one(pool).await?;
    assert_eq!(
        sub_ledger, 0,
        "the posting touches no sub-ledger, so none can stray from its control"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_item_on_a_posted_line_keeps_its_kind_and_unit_and_a_line_takes_its_defaults(pool: PgPool) -> Res<()> {
    let (books, org) = open(&pool, "test").await?;
    books.save_item(org, item("W1", ItemKind::Stock, "EA")).await?;
    books.save_item(org, item("W2", ItemKind::Stock, "EA")).await?;
    post(&pool, org, "W1", true).await?;
    post(&pool, org, "W2", false).await?;
    let before = audits(&pool, org).await?;
    for changed in [item("W1", ItemKind::Service, "EA"), item("W1", ItemKind::Stock, "BOX")] {
        let refused = books.save_item(org, changed).await;
        assert!(
            matches!(&refused, Err(Error::ItemPosted(code)) if code == "W1"),
            "{refused:?}"
        );
    }
    assert_eq!(audits(&pool, org).await?, before);
    let renamed = Item {
        name: "Widget".to_owned(),
        volume: Some(Quantity(3)),
        ..item("W1", ItemKind::Stock, "EA")
    };
    books.save_item(org, renamed.clone()).await?;
    let drafted = item("W2", ItemKind::NonStock, "BOX");
    books.save_item(org, drafted.clone()).await?;
    assert_eq!(books.items(org, None).await?.items, [renamed.clone(), drafted]);
    assert_eq!(
        renamed.defaults(Side::Sale),
        LineDefaults {
            unit: "EA".to_owned(),
            account: Some("4000".to_owned()),
            tax: Some("T7".to_owned()),
        }
    );
    assert_eq!(
        renamed.defaults(Side::Purchase),
        LineDefaults {
            unit: "EA".to_owned(),
            account: None,
            tax: None,
        }
    );
    let after = audits(&pool, org).await?;
    assert_eq!(
        after.get(before.len()..),
        Some(&["item:W1".to_owned(), "item:W2".to_owned()][..])
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_item_names_only_a_unit_account_and_tax_code_the_company_has(pool: PgPool) -> Res<()> {
    let (books, org) = open(&pool, "test").await?;
    let before = audits(&pool, org).await?;
    let unit = books.save_item(org, item("W1", ItemKind::Stock, "KG")).await;
    assert!(
        matches!(&unit, Err(Error::NoSuchUnit(code)) if code == "KG"),
        "{unit:?}"
    );
    let account = Item {
        purchase_account: Some("5000".to_owned()),
        ..item("W1", ItemKind::Stock, "EA")
    };
    let account = books.save_item(org, account).await;
    assert!(
        matches!(&account, Err(Error::NoSuchAccount(code)) if code == "5000"),
        "{account:?}"
    );
    let tax = Item {
        purchase_tax: Some("X9".to_owned()),
        ..item("W1", ItemKind::Stock, "EA")
    };
    let tax = books.save_item(org, tax).await;
    assert!(matches!(tax, Err(Error::Refused(Refusal::UnknownTaxCode))), "{tax:?}");
    assert_eq!(audits(&pool, org).await?, before);
    let withheld = Item {
        purchase_tax: Some("W3".to_owned()),
        ..item("W1", ItemKind::Service, "EA")
    };
    books.save_item(org, withheld).await?;
    let (plain, bare) = open(&pool, "none").await?;
    let untaxed = plain.save_item(bare, item("W1", ItemKind::Stock, "EA")).await;
    assert!(
        matches!(untaxed, Err(Error::Refused(Refusal::UnknownTaxCode))),
        "{untaxed:?}"
    );
    let units = books.units(org, None).await?.items;
    let codes: Vec<&str> = units.iter().map(|u| u.code.as_str()).collect();
    assert_eq!(codes, ["BOX", "EA"]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_synced_item_is_read_only_by_hand_and_keeps_its_kind_once_posted(pool: PgPool) -> Res<()> {
    let (books, org) = open(&pool, "test").await?;
    let sync = |code: &str, kind| books.sync_item(org, "sku-9".to_owned(), item(code, kind, "EA"));
    sync("S9", ItemKind::Stock).await?;
    sync("S9A", ItemKind::Stock).await?;
    let before = audits(&pool, org).await?;
    let by_hand = books.save_item(org, item("S9A", ItemKind::Stock, "EA")).await;
    assert!(
        matches!(&by_hand, Err(Error::Synced(code)) if code == "S9A"),
        "{by_hand:?}"
    );
    assert_eq!(audits(&pool, org).await?, before);
    let synced = Item {
        host_ref: Some("sku-9".to_owned()),
        ..item("S9A", ItemKind::Stock, "EA")
    };
    assert_eq!(books.items(org, None).await?.items, [synced]);
    post(&pool, org, "S9A", true).await?;
    let retyped = sync("S9A", ItemKind::Service).await;
    assert!(
        matches!(&retyped, Err(Error::ItemPosted(code)) if code == "S9A"),
        "{retyped:?}"
    );
    assert_eq!(before.iter().filter(|r| *r == "item:host:sku-9").count(), 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn the_catalogue_needs_approve_to_change_and_read_to_list(pool: PgPool) -> Res<()> {
    let (books, org) = open(&pool, "test").await?;
    books.save_item(org, item("W1", ItemKind::Stock, "EA")).await?;
    for (n, role) in (2..).zip([Role::Staff, Role::Viewer, Role::Accountant]) {
        let address = format!("member{n}@example.com");
        books.host().sign_in(person(1, "ann@example.com")?);
        books.add_member(org, Email::new(&address)?, role, None).await?;
        books.host().sign_in(person(n, &address)?);
        assert_eq!(books.items(org, None).await?.items.len(), 1);
        let unit = Unit {
            code: format!("U{n}"),
            name: "Unit".to_owned(),
        };
        let saved = books.save_item(org, item("W1", ItemKind::NonStock, "EA")).await;
        let unit = books.save_unit(org, unit).await;
        let synced = books
            .sync_item(org, format!("sku-{n}"), item(&format!("S{n}"), ItemKind::Stock, "EA"))
            .await;
        for outcome in [saved, unit, synced] {
            match outcome {
                Ok(()) => assert!(role.allows(Word::Approve), "{role:?} changed the catalogue"),
                Err(Error::Forbidden { word }) => assert!(!role.allows(word) && word == Word::Approve),
                Err(other) => return Err(other.into()),
            }
        }
    }
    Ok(())
}
