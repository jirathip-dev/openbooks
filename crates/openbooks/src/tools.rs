use openbooks_core::{OrgId, Refusal, Role, Word};
use sqlx::PgConnection;

use crate::api::{Mount, Operation, Schema};
use crate::openapi::{Json, map, object, render, text};
use crate::{Error, SignedIn, Work};

const ROWS: u16 = 50;
const SIZE: usize = 8 * 1024;
const TEXT: usize = 200;

/// Answers a mount's tool by its name, in the caller's transaction once their role is admitted.
pub type Serve = for<'a> fn(Ask<'a>) -> Work<'a, Reply>;

/// One tool call: the tool's name and arguments, the company and its database.
pub struct Ask<'a> {
    pub db: &'a mut PgConnection,
    pub org: OrgId,
    pub tool: &'static str,
    pub arguments: &'a [(String, String)],
}

impl Ask<'_> {
    pub fn argument(&self, name: &str) -> Option<&str> {
        self.arguments.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The cursor `after` from the last page's next, read back from its hex; absent for the first page.
    pub fn after(&self) -> Result<Option<String>, Error> {
        self.argument("after").map(unhex).transpose()
    }

    /// The rows a page may fetch: the agent view's 50, and one more to tell there are more.
    pub fn rows(&self) -> u16 {
        ROWS + 1
    }
}

/// A tool's answer: one value, or rows each with the cursor that pages on after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    One(Json),
    Rows(Vec<(Json, String)>),
}

/// The tools built at start-up from the mounts' declarations, each with its switch; company-wide reports start off.
#[derive(Debug, Clone)]
pub struct Tools(Vec<(Operation, Serve, bool)>);

impl Tools {
    pub fn new(mounts: &[Mount]) -> Self {
        let ops = mounts.iter().flat_map(|m| m.operations.iter().map(|op| (*op, m.serve)));
        let tools = ops.filter(|(op, _)| op.is_tool() && !reaches_never(&op.output()));
        Self(
            tools
                .map(|(op, serve)| (op, serve, op.word() != Word::Reports))
                .collect(),
        )
    }

    pub fn switch(mut self, name: &str, on: bool) -> Result<Self, Error> {
        let tool = self.0.iter_mut().find(|(op, _, _)| op.name() == name);
        let (_, _, switch) = tool.ok_or_else(|| Error::NoSuchTool(name.to_owned()))?;
        *switch = on;
        Ok(self)
    }

    pub(crate) fn serve(&self, op: &Operation) -> Option<Serve> {
        self.0
            .iter()
            .find(|(o, _, on)| *on && o.name() == op.name())
            .map(|(_, serve, _)| *serve)
    }

    /// The tools switched on whose word the role has and the host's narrowing keeps.
    pub fn offered(&self, role: Role, signed: &SignedIn) -> Vec<Operation> {
        let offered = |op: &Operation| role.allows(op.word()) && signed.keeps(op.word());
        self.0
            .iter()
            .filter(|(op, _, on)| *on && offered(op))
            .map(|(op, _, _)| *op)
            .collect()
    }
}

fn reaches_never(schema: &Schema) -> bool {
    match schema {
        Schema::Never(_) => true,
        Schema::Maybe(inner) | Schema::Page(inner) => reaches_never(inner),
        Schema::Object(fields) => fields.iter().any(|f| reaches_never(&f.schema)),
        Schema::Nothing
        | Schema::Text
        | Schema::Flag
        | Schema::Whole
        | Schema::Quantity
        | Schema::Day
        | Schema::Email
        | Schema::Id
        | Schema::Instant
        | Schema::Words(_) => false,
    }
}

pub(crate) fn listing(tools: &[Operation]) -> String {
    let tool = |op: &Operation| {
        map([
            ("name", text(op.name())),
            ("description", text(op.about())),
            ("inputSchema", object(&op.input())),
        ])
    };
    render(&map([("tools", Json::List(tools.iter().map(tool).collect()))]))
}

pub(crate) fn answer(reply: Reply) -> String {
    match reply {
        Reply::One(json) => render(&scrub(json)),
        Reply::Rows(rows) => agent_page(rows),
    }
}

fn agent_page(mut items: Vec<(Json, String)>) -> String {
    let more = items.len() > usize::from(ROWS);
    items.truncate(usize::from(ROWS));
    let mut size = 0;
    let fits = items.iter().take_while(|item| {
        size += render(&scrub(item.0.clone())).len();
        size <= SIZE
    });
    let kept = fits.count().max(1).min(items.len());
    let more = more || kept < items.len();
    items.truncate(kept);
    let next = items.last().filter(|_| more).map(|(_, cursor)| text(&hex(cursor)));
    let mut page = map([("items", Json::List(items.into_iter().map(|(i, _)| scrub(i)).collect()))]);
    if let Some(next) = next {
        page = page.with("next", next);
    }
    render(&page)
}

fn scrub(json: Json) -> Json {
    match json {
        Json::Str(value) => {
            let words = value.split(char::is_control).flat_map(str::split_whitespace);
            Json::Str(words.collect::<Vec<_>>().join(" ").chars().take(TEXT).collect())
        }
        Json::List(items) => Json::List(items.into_iter().map(scrub).collect()),
        Json::Map(pairs) => Json::Map(pairs.into_iter().map(|(k, v)| (k, scrub(v))).collect()),
        other @ (Json::Bool(_) | Json::Num(_)) => other,
    }
}

fn hex(cursor: &str) -> String {
    let digit = |n: u8| char::from_digit(u32::from(n), 16).unwrap_or('0');
    cursor.bytes().flat_map(|b| [digit(b >> 4), digit(b & 15)]).collect()
}

fn unhex(cursor: &str) -> Result<String, Error> {
    let bad = || Error::from(Refusal::BadId);
    let bytes = cursor.as_bytes().chunks(2).map(|pair| {
        let pair = std::str::from_utf8(pair).map_err(|_| bad())?;
        (pair.len() == 2)
            .then(|| u8::from_str_radix(pair, 16).ok())
            .flatten()
            .ok_or_else(bad)
    });
    String::from_utf8(bytes.collect::<Result<_, _>>()?).map_err(|_| bad())
}
