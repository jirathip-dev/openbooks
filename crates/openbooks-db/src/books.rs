use openbooks_core::{BookKind, Date, OrgId};
use sqlx::{PgConnection, query, query_as, query_scalar};

use crate::refused;

/// One of the company's five books: its kind, and the code, place and name the company gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub kind: BookKind,
    pub code: String,
    pub order: u16,
    pub name: String,
}

/// A number series: its code, the book it numbers for, the document kind it is kept for, and the years its numbers count ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumberSeries {
    pub code: String,
    pub book: Option<BookKind>,
    pub document: Option<String>,
    pub year_offset: u16,
}

fn small(n: u16) -> i16 {
    i16::try_from(n).unwrap_or(i16::MAX)
}

fn whole(n: i16) -> u16 {
    u16::try_from(n).unwrap_or_default()
}

fn kind(text: Option<String>) -> Result<Option<BookKind>, sqlx::Error> {
    text.map(|k| BookKind::parse(&k)).transpose().map_err(refused)
}

/// Makes a new company's books, a series for each under its code, then the other series.
pub async fn open(
    db: &mut PgConnection,
    org: OrgId,
    books: &[Book],
    series: &[NumberSeries],
) -> Result<(), sqlx::Error> {
    for book in books {
        save_book(db, org, book).await?;
        let own = NumberSeries {
            code: book.code.clone(),
            book: Some(book.kind),
            document: None,
            year_offset: 0,
        };
        add_series(db, org, &own).await?;
    }
    for added in series {
        add_series(db, org, added).await?;
    }
    Ok(())
}

/// The company's books, in its order.
pub async fn books(db: &mut PgConnection, org: OrgId) -> Result<Vec<Book>, sqlx::Error> {
    let rows: Vec<(String, String, i16, String)> =
        query_as("SELECT kind, code, position, name FROM ledger.book WHERE org_id = $1 ORDER BY position, kind")
            .bind(org.get())
            .fetch_all(db)
            .await?;
    rows.into_iter()
        .map(|(k, code, order, name)| {
            Ok(Book {
                kind: BookKind::parse(&k).map_err(refused)?,
                code,
                order: whole(order),
                name,
            })
        })
        .collect()
}

/// Makes the book of its kind, or gives it the code, place and name.
pub async fn save_book(db: &mut PgConnection, org: OrgId, book: &Book) -> Result<(), sqlx::Error> {
    query(
        "INSERT INTO ledger.book (org_id, kind, code, position, name) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (org_id, kind) DO UPDATE SET code = excluded.code, position = excluded.position, name = excluded.name",
    )
    .bind(org.get())
    .bind(book.kind.as_str())
    .bind(&book.code)
    .bind(small(book.order))
    .bind(&book.name)
    .execute(db)
    .await
    .map(drop)
}

/// The kind of the book with the code, if one has it.
pub async fn book_with_code(db: &mut PgConnection, org: OrgId, code: &str) -> Result<Option<BookKind>, sqlx::Error> {
    let found = query_scalar("SELECT kind FROM ledger.book WHERE org_id = $1 AND code = $2")
        .bind(org.get())
        .bind(code)
        .fetch_optional(db)
        .await?;
    kind(found)
}

const SERIES: &str = "SELECT code, book, document, year_offset FROM ledger.series";

type SeriesRow = (String, Option<String>, Option<String>, i16);

fn series_of((code, book, document, offset): SeriesRow) -> Result<NumberSeries, sqlx::Error> {
    Ok(NumberSeries {
        code,
        book: kind(book)?,
        document,
        year_offset: whole(offset),
    })
}

/// The series with the code, if there is one.
pub async fn series_by_code(
    db: &mut PgConnection,
    org: OrgId,
    code: &str,
) -> Result<Option<NumberSeries>, sqlx::Error> {
    let row = query_as(&format!("{SERIES} WHERE org_id = $1 AND code = $2"))
        .bind(org.get())
        .bind(code)
        .fetch_optional(db)
        .await?;
    row.map(series_of).transpose()
}

/// The series kept for the document kind, if there is one.
pub async fn series_for(
    db: &mut PgConnection,
    org: OrgId,
    document: &str,
) -> Result<Option<NumberSeries>, sqlx::Error> {
    let row = query_as(&format!("{SERIES} WHERE org_id = $1 AND document = $2"))
        .bind(org.get())
        .bind(document)
        .fetch_optional(db)
        .await?;
    row.map(series_of).transpose()
}

/// Up to `limit` series by code, from just after `after`.
pub async fn series(
    db: &mut PgConnection,
    org: OrgId,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<NumberSeries>, sqlx::Error> {
    let sql = format!("{SERIES} WHERE org_id = $1 AND ($2::text IS NULL OR code > $2) ORDER BY code LIMIT $3");
    let rows: Vec<SeriesRow> = query_as(&sql)
        .bind(org.get())
        .bind(after)
        .bind(limit)
        .fetch_all(db)
        .await?;
    rows.into_iter().map(series_of).collect()
}

pub async fn add_series(db: &mut PgConnection, org: OrgId, series: &NumberSeries) -> Result<(), sqlx::Error> {
    query("INSERT INTO ledger.series (org_id, code, book, document, year_offset) VALUES ($1, $2, $3, $4, $5)")
        .bind(org.get())
        .bind(&series.code)
        .bind(series.book.map(BookKind::as_str))
        .bind(series.document.as_deref())
        .bind(small(series.year_offset))
        .execute(db)
        .await
        .map(drop)
}

pub async fn set_year_offset(db: &mut PgConnection, org: OrgId, code: &str, offset: u16) -> Result<(), sqlx::Error> {
    query("UPDATE ledger.series SET year_offset = $3 WHERE org_id = $1 AND code = $2")
        .bind(org.get())
        .bind(code)
        .bind(small(offset))
        .execute(db)
        .await
        .map(drop)
}

/// Whether the series has given a number.
pub async fn numbered(db: &mut PgConnection, org: OrgId, code: &str) -> Result<bool, sqlx::Error> {
    query_scalar("SELECT EXISTS (SELECT 1 FROM ledger.number WHERE org_id = $1 AND code = $2)")
        .bind(org.get())
        .bind(code)
        .fetch_one(db)
        .await
}

/// Gives a document dated `on` the next number of its kind's series, else its book's, locked to the caller's transaction end; none if neither.
pub async fn take_number(
    db: &mut PgConnection,
    org: OrgId,
    document: &str,
    book: Option<BookKind>,
    on: Date,
) -> Result<Option<String>, sqlx::Error> {
    let series: Option<(String, i16)> = query_as(
        "SELECT code, year_offset FROM ledger.series WHERE org_id = $1 \
         AND (document = $2 OR (document IS NULL AND book = $3)) ORDER BY document IS NULL LIMIT 1 FOR UPDATE",
    )
    .bind(org.get())
    .bind(document)
    .bind(book.map(BookKind::as_str))
    .fetch_optional(&mut *db)
    .await?;
    let Some((code, offset)) = series else {
        return Ok(None);
    };
    let (year, month) = (on.month().year(), on.month().month());
    let first = format!("{year:04}-{month:02}-01");
    let seq: i32 = query_scalar(
        "SELECT coalesce(max(seq), 0) + 1 FROM ledger.number WHERE org_id = $1 AND code = $2 AND month = $3::date",
    )
    .bind(org.get())
    .bind(&code)
    .bind(&first)
    .fetch_one(&mut *db)
    .await?;
    let yy = (u32::from(year) + u32::from(whole(offset))) % 100;
    let number = format!("{code}{yy:02}{month:02}-{seq:04}");
    query("INSERT INTO ledger.number (org_id, code, month, seq, number) VALUES ($1, $2, $3::date, $4, $5)")
        .bind(org.get())
        .bind(&code)
        .bind(&first)
        .bind(seq)
        .bind(&number)
        .execute(db)
        .await?;
    Ok(Some(number))
}
