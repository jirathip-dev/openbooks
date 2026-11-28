use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use openbooks_core::{
    BasisPoints, BookKind, Document, Due, NoteReason, Party, Refusal, TaxCode, TaxLine, TaxPack, UserId,
};
use sqlx::{PgConnection, PgPool, query};

use crate::{AccountRole, ChartAccount, Error, Event, Files, Hooks, Host, Migration, Pack, Series, SignedIn, Work};

/// A host for tests: sign-in held in memory, a fixed clock, counted ids, files in memory, and a switch to roll back.
pub struct TestHost {
    pool: PgPool,
    who: Mutex<Option<SignedIn>>,
    names: Mutex<BTreeMap<UserId, String>>,
    rolls_back: AtomicBool,
    ids: AtomicU64,
    files: MemoryFiles,
}

impl TestHost {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            who: Mutex::new(None),
            names: Mutex::new(BTreeMap::new()),
            rolls_back: AtomicBool::new(false),
            ids: AtomicU64::new(0),
            files: MemoryFiles::default(),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn sign_in(&self, signed: SignedIn) {
        *self.who.lock().unwrap_or_else(PoisonError::into_inner) = Some(signed);
    }

    pub fn sign_out(&self) {
        *self.who.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub fn name(&self, user: UserId, name: &str) {
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        names.insert(user, name.to_owned());
    }

    /// Ends every later transaction in a roll back, whatever the work returned.
    pub fn roll_back(&self, rolls_back: bool) {
        self.rolls_back.store(rolls_back, Ordering::SeqCst);
    }
}

impl Host for TestHost {
    fn who(&self) -> Option<SignedIn> {
        self.who.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    async fn transact<T, F>(&self, work: F) -> Result<T, Error>
    where
        T: Send,
        F: for<'c> FnOnce(&'c mut PgConnection) -> Work<'c, T> + Send,
    {
        let mut tx = self.pool.begin().await?;
        let out = work(&mut tx).await;
        if out.is_ok() && !self.rolls_back.load(Ordering::SeqCst) {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        out
    }

    fn new_id(&self) -> String {
        format!("test-{}", self.ids.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_hours(500_000)
    }

    fn files(&self) -> &dyn Files {
        &self.files
    }

    fn names(&self, users: &[UserId]) -> BTreeMap<UserId, String> {
        let names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        users.iter().filter_map(|u| Some((*u, names.get(u)?.clone()))).collect()
    }
}

#[derive(Default)]
struct MemoryFiles(Mutex<Vec<(String, Vec<u8>)>>);

impl Files for MemoryFiles {
    fn put<'a>(&'a self, name: &'a str, bytes: Vec<u8>) -> Work<'a, String> {
        Box::pin(async move {
            let mut files = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let key = format!("{}-{name}", files.len() + 1);
            files.push((key.clone(), bytes));
            Ok(key)
        })
    }

    fn get<'a>(&'a self, key: &'a str) -> Work<'a, Vec<u8>> {
        Box::pin(async move {
            let files = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let found = files.iter().find(|(k, _)| k == key).map(|(_, bytes)| bytes.clone());
            found.ok_or_else(|| Error::Host(format!("no file {key}")))
        })
    }
}

/// The added tax of the test pack: seven percent, due when the document is settled.
pub const ADDED: TaxCode = TaxCode {
    code: "T7",
    due: Due::Settlement,
};

/// The withheld tax of the test pack: three percent of what a settlement pays.
pub const WITHHELD: TaxCode = TaxCode {
    code: "W3",
    due: Due::Settlement,
};

/// A pack for tests, `test`: one added and one withheld tax, a table in its schema each change is noted in, and the changes it was handed after commit.
#[derive(Clone, Default)]
pub struct TestPack(Arc<Mutex<Vec<String>>>);

impl TestPack {
    pub fn delivered(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

struct TestTax;

impl TaxPack for TestTax {
    fn added(&self) -> &[TaxCode] {
        &[ADDED]
    }

    fn withheld(&self) -> &[TaxCode] {
        &[WITHHELD]
    }

    fn document_tax(&self, document: &Document) -> Result<Vec<TaxLine>, Refusal> {
        document.only(self.added())?;
        if !document.lines.iter().any(|l| l.codes.iter().any(|c| c == ADDED.code)) {
            return Ok(Vec::new());
        }
        let taxable = document.taxable(Some(ADDED.code))?;
        let tax = taxable.share(BasisPoints(700))?;
        Ok(vec![TaxLine {
            code: ADDED.code,
            taxable,
            tax,
        }])
    }

    fn settlement_tax(&self, settled: &Document, withheld: &[&str]) -> Result<Vec<TaxLine>, Refusal> {
        let mut lines = self.document_tax(settled)?;
        for code in withheld {
            if *code != WITHHELD.code {
                return Err(Refusal::UnknownTaxCode);
            }
            let taxable = settled.taxable(None)?;
            let tax = taxable.share(BasisPoints(300))?;
            lines.push(TaxLine {
                code: WITHHELD.code,
                taxable,
                tax,
            });
        }
        Ok(lines)
    }

    fn check_party(&self, party: &Party) -> Result<(), Refusal> {
        match &party.tax_id {
            Some(id) if !id.trim().is_empty() => Ok(()),
            _ => Err(Refusal::Party("the test pack needs the party's tax id")),
        }
    }

    fn note_reasons(&self) -> &[NoteReason] {
        &[NoteReason {
            code: "price",
            label: "The price was wrong",
        }]
    }
}

impl Hooks for TestPack {
    fn before_commit<'a>(&'a self, db: &'a mut PgConnection, event: &'a Event) -> Work<'a, ()> {
        Box::pin(async move {
            query("INSERT INTO pack_test.seen (org_id, event_id, action) VALUES ($1, $2, $3)")
                .bind(event.org.get())
                .bind(event.id)
                .bind(&event.action)
                .execute(db)
                .await?;
            Ok(())
        })
    }

    fn after_commit<'a>(&'a self, event: &'a Event) -> Work<'a, ()> {
        Box::pin(async move {
            let mut delivered = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            delivered.push(event.action.clone());
            Ok(())
        })
    }
}

impl Pack for TestPack {
    fn name(&self) -> &'static str {
        "test"
    }

    fn tax(&self) -> &dyn TaxPack {
        &TestTax
    }

    fn account_roles(&self) -> &[AccountRole] {
        &[
            AccountRole {
                key: "added_tax_undue",
                about: "Added tax on documents not yet settled.",
            },
            AccountRole {
                key: "added_tax",
                about: "Added tax due on what was settled.",
            },
            AccountRole {
                key: "withheld_tax",
                about: "Tax a customer withheld from a settlement.",
            },
        ]
    }

    fn chart(&self) -> &[ChartAccount] {
        &[
            ChartAccount {
                code: "2101",
                name: "Added tax undue",
                role: Some("added_tax_undue"),
            },
            ChartAccount {
                code: "2102",
                name: "Added tax",
                role: Some("added_tax"),
            },
            ChartAccount {
                code: "1150",
                name: "Tax withheld from us",
                role: Some("withheld_tax"),
            },
        ]
    }

    fn series(&self) -> &[Series] {
        &[
            Series {
                document: "invoice",
                code: "TI",
                book: Some(BookKind::Sales),
            },
            Series {
                document: "quotation",
                code: "QT",
                book: None,
            },
        ]
    }

    fn migrations(&self) -> &[Migration] {
        &[Migration {
            version: 1,
            name: "seen",
            sql: "CREATE TABLE seen (org_id bigint NOT NULL REFERENCES ledger.org (org_id), event_id bigint NOT NULL, \
                  action text NOT NULL, PRIMARY KEY (org_id, event_id))",
        }]
    }
}
