use openbooks_core::{BookKind, IdempotencyKey, OrgId, Word};
use openbooks_db::audit::Record;
use openbooks_db::books::{self, Book, NumberSeries};
use sqlx::PgConnection;

use crate::members::{PAGE, paged};
use crate::openapi::{Json, map, text};
use crate::{Ask, Error, Field, Host, Openbooks, Operation, Pack, Page, Reply, Schema, Shape};

pub(crate) const BOOKS_LIST: Operation =
    crate::read!("books", Word::Read, "Lists the company's five books in its order.", {} -> Page<Book>);

pub(crate) const SAVE_BOOK: Operation = crate::closed!("book.save", Word::Approve, "Gives a book its code, place and name; the numbers it gave keep theirs.", {
    kind: BookKind = "Which of the five books.",
    code: String = "Its code, letters and digits, unique among the books.",
    order: u16 = "Its place in the list, from 1.",
    name: String = "Its name.",
} -> ());

pub(crate) const SERIES: Operation = crate::read!("series", Word::Read, "Lists the company's number series by code, a page at a time.", {
    after: Option<String> = "The cursor from the last page's next; absent for the first page.",
} -> Page<NumberSeries>);

pub(crate) const ADD_SERIES: Operation = crate::closed!("series.add", Word::Approve, "Gives a document kind its own number series, in a book if the kind posts.", {
    code: String = "The series' code, letters and digits, unique among the series.",
    document: String = "The document kind it numbers.",
    book: Option<BookKind> = "The book the kind posts in; absent for a kind that posts nothing.",
    year_offset: u16 = "The years its numbers count ahead of the calendar's.",
} -> ())
.creates();

pub(crate) const SERIES_OFFSET: Operation = crate::closed!("series.offset", Word::Approve, "Sets the years a series' numbers count ahead, until it gives its first number.", {
    code: String = "The series' code.",
    year_offset: u16 = "The years its numbers count ahead of the calendar's.",
} -> ());

fn field(name: &'static str, about: &'static str, schema: Schema) -> Field {
    Field { name, about, schema }
}

impl Shape for BookKind {
    fn schema() -> Schema {
        Schema::Words(BookKind::ALL.map(BookKind::as_str).to_vec())
    }
}

impl Shape for Book {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("kind", "Which of the five books.", BookKind::schema()),
            field("code", "Its code.", String::schema()),
            field("order", "Its place in the list.", u16::schema()),
            field("name", "Its name.", String::schema()),
        ])
    }
}

impl Shape for NumberSeries {
    fn schema() -> Schema {
        Schema::Object(vec![
            field("code", "The series' code, the start of each number.", String::schema()),
            field(
                "book",
                "The book it numbers for; absent for a kind that posts nothing.",
                Option::<BookKind>::schema(),
            ),
            field(
                "document",
                "The document kind it is kept for; absent for its book's own.",
                Option::<String>::schema(),
            ),
            field(
                "year_offset",
                "The years its numbers count ahead of the calendar's.",
                u16::schema(),
            ),
        ])
    }
}

fn code(code: &str) -> Result<(), Error> {
    if !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(Error::BadCode(code.to_owned()))
    }
}

pub(crate) async fn serve(ask: Ask<'_>) -> Result<Reply, Error> {
    if ask.tool == BOOKS_LIST.name() {
        let items = books::books(ask.db, ask.org).await?;
        return Ok(Reply::One(Json::List(items.iter().map(book).collect())));
    }
    let (n, after) = (i64::from(ask.rows()), ask.after()?);
    let items = books::series(ask.db, ask.org, after.as_deref(), n).await?;
    Ok(Reply::Rows(items.iter().map(|s| (series(s), s.code.clone())).collect()))
}

fn book(b: &Book) -> Json {
    map([
        ("kind", text(b.kind.as_str())),
        ("code", text(&b.code)),
        ("order", Json::Num(i64::from(b.order))),
        ("name", text(&b.name)),
    ])
}

fn series(s: &NumberSeries) -> Json {
    let mut json = map([
        ("code", text(&s.code)),
        ("year_offset", Json::Num(i64::from(s.year_offset))),
    ]);
    if let Some(book) = s.book {
        json = json.with("book", text(book.as_str()));
    }
    if let Some(document) = &s.document {
        json = json.with("document", text(document));
    }
    json
}

/// Makes a new company's five books and their series from its pack's defaults.
pub(crate) async fn open(db: &mut PgConnection, org: OrgId, pack: &dyn Pack) -> Result<(), Error> {
    let names = pack.books();
    let made: Vec<Book> = (1..)
        .zip(BookKind::ALL)
        .map(|(order, kind)| Book {
            kind,
            code: names.of(kind).code.to_owned(),
            order,
            name: names.of(kind).name.to_owned(),
        })
        .collect();
    let series: Vec<NumberSeries> = pack
        .series()
        .iter()
        .map(|s| NumberSeries {
            code: s.code.to_owned(),
            book: s.book,
            document: Some(s.document.to_owned()),
            year_offset: 0,
        })
        .collect();
    Ok(books::open(db, org, &made, &series).await?)
}

impl<H: Host + 'static> Openbooks<H> {
    /// The company's five books in its order.
    pub async fn books(&self, org: OrgId) -> Result<Page<Book>, Error> {
        self.read(org, BOOKS_LIST, move |db| {
            Box::pin(async move {
                let items = books::books(db, org).await?;
                Ok(Page { items, next: None })
            })
        })
        .await
    }

    /// Gives a book its code, place and name; numbers given already keep theirs.
    pub async fn save_book(&self, org: OrgId, book: Book) -> Result<(), Error> {
        code(&book.code)?;
        let detail = format!("{} {} {} {}", book.kind.as_str(), book.code, book.order, book.name);
        self.write(org, SAVE_BOOK, Record::Book(book.kind), detail, None, move |db| {
            Box::pin(async move {
                match books::book_with_code(db, org, &book.code).await? {
                    Some(kind) if kind != book.kind => Err(Error::CodeTaken(book.code)),
                    _ => Ok(books::save_book(db, org, &book).await?),
                }
            })
        })
        .await
    }

    /// One page of the number series by code, from just after the cursor `after`.
    pub async fn series(&self, org: OrgId, after: Option<String>) -> Result<Page<NumberSeries>, Error> {
        self.read(org, SERIES, move |db| {
            Box::pin(async move {
                let items = books::series(db, org, after.as_deref(), i64::from(PAGE) + 1).await?;
                Ok(paged(items, |s| s.code.clone()))
            })
        })
        .await
    }

    /// Gives a document kind its own series, in the book it posts in; sent again with its key, it adds one.
    pub async fn add_series(
        &self,
        org: OrgId,
        code: String,
        document: String,
        book: Option<BookKind>,
        year_offset: u16,
        key: Option<IdempotencyKey>,
    ) -> Result<(), Error> {
        self::code(&code)?;
        let detail = format!(
            "{code} {document} {} +{year_offset}",
            book.map_or("no book", BookKind::as_str)
        );
        let record = Record::Series(code.clone());
        self.write(org, ADD_SERIES, record, detail, key, move |db| {
            Box::pin(async move {
                if books::series_by_code(db, org, &code).await?.is_some() {
                    return Err(Error::CodeTaken(code));
                }
                if books::series_for(db, org, &document).await?.is_some() {
                    return Err(Error::DocumentHasSeries(document));
                }
                let series = NumberSeries {
                    code,
                    book,
                    document: Some(document),
                    year_offset,
                };
                Ok(books::add_series(db, org, &series).await?)
            })
        })
        .await
    }

    /// Sets the years a series' numbers count ahead; once it has given a number, they stay.
    pub async fn set_year_offset(&self, org: OrgId, code: String, year_offset: u16) -> Result<(), Error> {
        let detail = format!("{code} +{year_offset}");
        let record = Record::Series(code.clone());
        self.write(org, SERIES_OFFSET, record, detail, None, move |db| {
            Box::pin(async move {
                if books::series_by_code(db, org, &code).await?.is_none() {
                    return Err(Error::NoSuchSeries(code));
                }
                if books::numbered(db, org, &code).await? {
                    return Err(Error::SeriesNumbered(code));
                }
                Ok(books::set_year_offset(db, org, &code, year_offset).await?)
            })
        })
        .await
    }
}
