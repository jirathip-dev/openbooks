use openbooks_db::events::Event;
use sqlx::PgConnection;

use crate::Work;

/// What the host runs on each change: before it commits, where an error rolls it back, and after, in order.
pub trait Hooks: Send + Sync {
    fn before_commit<'a>(&'a self, _db: &'a mut PgConnection, _event: &'a Event) -> Work<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn after_commit<'a>(&'a self, _event: &'a Event) -> Work<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl Hooks for () {}
