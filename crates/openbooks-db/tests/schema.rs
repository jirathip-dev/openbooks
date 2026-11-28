use openbooks_db::{SchemaError, check, migrate, migrations};
use sqlx::{PgPool, query_scalar, raw_sql};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

async fn run(pool: &PgPool, sql: &str) -> Result<(), String> {
    let done = raw_sql(sql).execute(pool).await.map(drop);
    done.map_err(|e| e.as_database_error().map_or(e.to_string(), |d| d.message().to_owned()))
}

async fn dump(pool: &PgPool) -> Res<String> {
    Ok(query_scalar(include_str!("sql/dump.sql")).fetch_one(pool).await?)
}

#[sqlx::test(migrations = false)]
async fn a_fresh_database_replays_to_the_same_dump_twice(pool: PgPool) -> Res<()> {
    assert!(matches!(check(&pool).await, Err(SchemaError::Missing(v)) if v == [1, 2]));
    migrate(&pool).await?;
    migrate(&pool).await?;
    check(&pool).await?;
    let name: String = query_scalar("SELECT 'host_' || md5(current_database())")
        .fetch_one(&pool)
        .await?;
    run(&pool, &format!("CREATE DATABASE {name}")).await?;
    let host = PgPool::connect_with(pool.connect_options().as_ref().clone().database(&name)).await?;
    for migration in migrations() {
        run(&host, migration.sql).await?;
    }
    check(&host).await?;
    assert_eq!(dump(&pool).await?, dump(&host).await?);
    let unscoped: Vec<String> = query_scalar(include_str!("sql/unscoped.sql")).fetch_all(&host).await?;
    assert_eq!(
        (unscoped.len(), dump(&host).await?.contains("posted_line_is_final")),
        (0, true)
    );
    run(&host, "INSERT INTO openbooks.schema_version VALUES (99)").await?;
    assert!(matches!(check(&host).await, Err(SchemaError::Unknown(v)) if v == [99]));
    host.close().await;
    Ok(run(&pool, &format!("DROP DATABASE {name}")).await?)
}

#[sqlx::test(migrations = false)]
async fn the_guards_refuse_an_unbalanced_entry_and_any_change_to_a_posted_one(pool: PgPool) -> Res<()> {
    migrate(&pool).await?;
    let line = "entry 1 is posted: its lines never change";
    let entry = "entry 1 is posted and never changes";
    let post = "UPDATE ledger.entry SET posted_at = now()";
    for (sql, outcome) in [
        (include_str!("sql/books.sql"), Ok(())),
        (post, Err("entry 1 does not balance in base currency")),
        ("UPDATE ledger.line SET base_minor = 500 WHERE line_no = 1", Ok(())),
        (post, Ok(())),
        ("UPDATE ledger.line SET amount_minor = 600", Err(line)),
        ("INSERT INTO ledger.line VALUES (1, 1, 3, 1, 'USD', 0, 0)", Err(line)),
        ("UPDATE ledger.entry SET memo = 'x'", Err(entry)),
    ] {
        assert_eq!(run(&pool, sql).await, outcome.map_err(str::to_owned), "{sql}");
    }
    Ok(())
}
