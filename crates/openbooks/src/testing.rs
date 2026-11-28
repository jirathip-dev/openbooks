use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use openbooks_core::UserId;
use sqlx::{PgConnection, PgPool};

use crate::{Error, Files, Host, SignedIn, Work};

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
