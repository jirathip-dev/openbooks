use openbooks::testing::{ADDED, TestHost, TestPack, WITHHELD};
use openbooks::{Error, Migration, NoPack, Openbooks, Pack, SignedIn};
use openbooks_core::{Currency, Date, Document, Due, Email, Line, Money, Party, Rate, Refusal, Role, TaxLine, UserId};
use openbooks_db::SchemaError;
use sqlx::{PgPool, query_as, query_scalar, raw_sql};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const UNSCOPED: &str = include_str!("../../openbooks-db/tests/sql/unscoped.sql");

fn xts() -> Res<Currency> {
    Ok(Currency::new("XTS", 2)?)
}

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, Email::new(address)?))
}

fn document(lines: &[(i64, &[&str])]) -> Res<Document> {
    let lines = lines.iter().map(|(minor, codes)| {
        let codes = codes.iter().map(|c| (*c).to_owned()).collect();
        Ok(Line {
            amount: Money::new(*minor, xts()?),
            codes,
        })
    });
    Ok(Document {
        lines: lines.collect::<Res<_>>()?,
        party: Party {
            name: "Nolla".to_owned(),
            tax_id: Some("0105".to_owned()),
        },
        date: Date::new(2026, 11, 26)?,
        currency: xts()?,
        rate: Rate::ONE,
    })
}

fn line(code: &'static str, taxable: i64, tax: i64) -> Res<TaxLine> {
    Ok(TaxLine {
        code,
        taxable: Money::new(taxable, xts()?),
        tax: Money::new(tax, xts()?),
    })
}

#[test]
fn the_test_pack_adds_a_tax_due_on_settlement_and_withholds_one_from_what_a_settlement_pays() -> Res<()> {
    let pack = TestPack::default();
    let tax = pack.tax();
    assert_eq!((tax.added(), tax.withheld()), (&[ADDED][..], &[WITHHELD][..]));
    assert_eq!((ADDED.due, WITHHELD.due), (Due::Settlement, Due::Settlement));
    let invoice = document(&[(10_000, &["T7"]), (5_000, &[])])?;
    assert_eq!(tax.document_tax(&invoice)?, [line("T7", 10_000, 700)?]);
    let half = document(&[(5_000, &["T7"]), (2_500, &[])])?;
    let settled = [line("T7", 5_000, 350)?, line("W3", 7_500, 225)?];
    assert_eq!(tax.settlement_tax(&half, &["W3"])?, settled);
    assert_eq!(tax.document_tax(&document(&[(100, &[])])?)?, []);
    assert_eq!(
        tax.document_tax(&document(&[(100, &["W3"])])?),
        Err(Refusal::UnknownTaxCode)
    );
    assert_eq!(tax.settlement_tax(&half, &["T7"]), Err(Refusal::UnknownTaxCode));
    assert_eq!(tax.check_party(&invoice.party), Ok(()));
    let anonymous = Party {
        tax_id: None,
        ..invoice.party
    };
    assert!(matches!(tax.check_party(&anonymous), Err(Refusal::Party(_))));
    assert_eq!(tax.note_reasons().len(), 1);
    let roles: Vec<_> = pack.account_roles().iter().map(|r| Some(r.key)).collect();
    assert!(pack.chart().iter().all(|a| roles.contains(&a.role)));
    assert_eq!(
        (NoPack.name(), NoPack.tax().added(), NoPack.migrations()),
        ("none", &[][..], &[][..])
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_packs_migration_lands_in_its_own_schema_once_and_the_org_scope_check_covers_it(pool: PgPool) -> Res<()> {
    let books = Openbooks::new(TestHost::new(pool.clone()))
        .pack(TestPack::default())
        .build();
    books.migrate(&pool).await?;
    books.migrate(&pool).await?;
    let schema: String = query_scalar("SELECT relnamespace::regnamespace::text FROM pg_class WHERE relname = 'seen'")
        .fetch_one(&pool)
        .await?;
    let versions: Vec<(String, i64)> = query_as("SELECT pack, version FROM openbooks.pack_version")
        .fetch_all(&pool)
        .await?;
    assert_eq!((schema.as_str(), versions), ("pack_test", vec![("test".to_owned(), 1)]));
    let unscoped: Vec<String> = query_scalar(UNSCOPED).fetch_all(&pool).await?;
    assert_eq!(unscoped, Vec::<String>::new());
    raw_sql("CREATE TABLE pack_test.loose (n integer)")
        .execute(&pool)
        .await?;
    let unscoped: Vec<String> = query_scalar(UNSCOPED).fetch_all(&pool).await?;
    assert_eq!(unscoped, ["loose"]);
    let stray = [Migration {
        version: 1,
        name: "stray",
        sql: "CREATE TABLE ledger.stray (org_id bigint)",
    }];
    let refused = openbooks_db::migrate_pack(&pool, "stray", &stray).await;
    assert!(
        matches!(&refused, Err(SchemaError::Pack(why)) if why.contains("outside pack_stray")),
        "{refused:?}"
    );
    let kept: Option<String> = query_scalar("SELECT to_regclass('ledger.stray')::text")
        .fetch_one(&pool)
        .await?;
    assert_eq!(kept, None);
    let named = openbooks_db::migrate_pack(&pool, "x; drop", &[]).await;
    assert!(matches!(named, Err(SchemaError::Pack(_))), "{named:?}");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn each_company_names_one_offered_pack_and_only_its_pack_hears_its_changes(pool: PgPool) -> Res<()> {
    let pack = TestPack::default();
    let books = Openbooks::new(TestHost::new(pool.clone())).pack(pack.clone()).build();
    books.migrate(&pool).await?;
    books.host().sign_in(person(1, "ann@example.com")?);
    let taxed = books.create_company("Acme", xts()?, "test", None).await?;
    books
        .add_member(taxed, Email::new("bob@example.com")?, Role::Staff, None)
        .await?;
    let plain = books.create_company("Plain", xts()?, "none", None).await?;
    books
        .add_member(plain, Email::new("cy@example.com")?, Role::Staff, None)
        .await?;
    let unknown = books.create_company("Nope", xts()?, "nope", None).await;
    assert!(
        matches!(&unknown, Err(Error::NoSuchPack(name)) if name == "nope"),
        "{unknown:?}"
    );
    let seen: Vec<(i64, String)> = query_as("SELECT org_id, action FROM pack_test.seen ORDER BY event_id")
        .fetch_all(&pool)
        .await?;
    let heard = ["company.create", "member.add"].map(str::to_owned);
    assert_eq!(seen, heard.clone().map(|action| (taxed.get(), action)));
    assert_eq!(pack.delivered(), heard);
    raw_sql("UPDATE ledger.org SET pack = 'gone' WHERE name = 'Plain'")
        .execute(&pool)
        .await?;
    let orphan = books
        .add_member(plain, Email::new("dee@example.com")?, Role::Staff, None)
        .await;
    assert!(
        matches!(&orphan, Err(Error::NoSuchPack(name)) if name == "gone"),
        "{orphan:?}"
    );
    let orgs: Vec<(String, String, i64)> = query_as(
        "SELECT o.name, o.pack, (SELECT count(*) FROM ledger.audit a WHERE a.org_id = o.org_id) \
         FROM ledger.org o ORDER BY o.org_id",
    )
    .fetch_all(&pool)
    .await?;
    let owned = |name: &str, pack: &str| (name.to_owned(), pack.to_owned(), 2);
    assert_eq!(orgs, [owned("Acme", "test"), owned("Plain", "gone")]);
    Ok(())
}
