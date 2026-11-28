use openbooks_core::{Role, Word};

use crate::api::{Mount, Operation, Schema};
use crate::openapi::{Json, map, object, render, text};
use crate::{Error, SignedIn};

pub(crate) const ROWS: u16 = 50;
const SIZE: usize = 8 * 1024;
const TEXT: usize = 200;

/// The tools built at start-up from the mounts' declarations, each with its switch; company-wide reports start off.
#[derive(Debug, Clone)]
pub struct Tools(Vec<(Operation, bool)>);

impl Tools {
    pub fn new(mounts: &[Mount]) -> Self {
        let ops = mounts.iter().flat_map(|m| m.operations);
        let tools = ops.filter(|op| op.is_tool() && !reaches_never(&op.output()));
        Self(tools.map(|op| (*op, op.word() != Word::Reports)).collect())
    }

    pub fn switch(mut self, name: &str, on: bool) -> Result<Self, Error> {
        let tool = self.0.iter_mut().find(|(op, _)| op.name() == name);
        let (_, switch) = tool.ok_or_else(|| Error::NoSuchTool(name.to_owned()))?;
        *switch = on;
        Ok(self)
    }

    /// The tools switched on whose word the role has and the host's narrowing keeps.
    pub fn offered(&self, role: Role, signed: &SignedIn) -> Vec<Operation> {
        let offered = |op: &Operation| role.allows(op.word()) && signed.keeps(op.word());
        self.0
            .iter()
            .filter(|(op, on)| *on && offered(op))
            .map(|(op, _)| *op)
            .collect()
    }
}

fn reaches_never(schema: &Schema) -> bool {
    match schema {
        Schema::Never(_) => true,
        Schema::Maybe(inner) | Schema::Page(inner) => reaches_never(inner),
        Schema::Object(fields) => fields.iter().any(|f| reaches_never(&f.schema)),
        Schema::Nothing | Schema::Text | Schema::Email | Schema::Id | Schema::Instant | Schema::Words(_) => false,
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

pub(crate) fn agent_page<T>(mut items: Vec<T>, json: impl Fn(&T) -> Json, cursor: impl Fn(&T) -> String) -> String {
    let more = items.len() > usize::from(ROWS);
    items.truncate(usize::from(ROWS));
    let mut size = 0;
    let fits = items.iter().take_while(|item| {
        size += render(&scrub(json(item))).len();
        size <= SIZE
    });
    let kept = fits.count().max(1).min(items.len());
    let more = more || kept < items.len();
    items.truncate(kept);
    let mut page = map([("items", Json::List(items.iter().map(|i| scrub(json(i))).collect()))]);
    if let Some(last) = items.last().filter(|_| more) {
        page = page.with("next", text(&cursor(last)));
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
