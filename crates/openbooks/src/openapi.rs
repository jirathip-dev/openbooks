use std::fmt::Write as _;

use openbooks_core::OrgId;

use crate::api::{Field, Mount, Operation, Schema, Shape};

/// A JSON value as the books write it: the `OpenAPI` document or a tool's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    Bool(bool),
    Num(i64),
    Str(String),
    List(Vec<Json>),
    Map(Vec<(String, Json)>),
}

pub(crate) fn text(value: &str) -> Json {
    Json::Str(value.to_owned())
}

pub(crate) fn map<const N: usize>(pairs: [(&str, Json); N]) -> Json {
    Json::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

impl Json {
    pub(crate) fn with(mut self, key: &str, value: Json) -> Self {
        if let Self::Map(pairs) = &mut self {
            pairs.push((key.to_owned(), value));
        }
        self
    }

    fn write(&self, out: &mut String, depth: usize) {
        let (open, close, items): (char, char, Vec<(Option<&str>, &Json)>) = match self {
            Self::Bool(value) => return out.push_str(if *value { "true" } else { "false" }),
            Self::Num(value) => return out.push_str(&value.to_string()),
            Self::Str(value) => return quote(out, value),
            Self::List(items) => ('[', ']', items.iter().map(|v| (None, v)).collect()),
            Self::Map(pairs) => ('{', '}', pairs.iter().map(|(k, v)| (Some(k.as_str()), v)).collect()),
        };
        out.push(open);
        let last = items.len();
        for (n, (key, value)) in items.into_iter().enumerate() {
            out.push('\n');
            out.push_str(&"  ".repeat(depth + 1));
            if let Some(key) = key {
                quote(out, key);
                out.push_str(": ");
            }
            value.write(out, depth + 1);
            if n + 1 < last {
                out.push(',');
            }
        }
        if last > 0 {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
        out.push(close);
    }
}

fn quote(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => write!(out, "\\u{:04x}", u32::from(c)).unwrap_or_default(),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn schema(shape: &Schema) -> Json {
    match shape {
        Schema::Nothing => map([]),
        Schema::Text => map([("type", text("string"))]),
        Schema::Email => map([("type", text("string")), ("format", text("email"))]),
        Schema::Instant => map([("type", text("string")), ("format", text("date-time"))]),
        Schema::Id => map([
            ("type", text("integer")),
            ("format", text("int64")),
            ("minimum", Json::Num(1)),
        ]),
        Schema::Words(words) => map([
            ("type", text("string")),
            ("enum", Json::List(words.iter().map(|w| text(w)).collect())),
        ]),
        Schema::Object(fields) => object(fields),
        Schema::Maybe(inner) => schema(inner),
        Schema::Never(inner) => schema(inner).with("x-openbooks-tier", text("never")),
        Schema::Page(inner) => map([
            ("type", text("object")),
            ("additionalProperties", Json::Bool(false)),
            ("x-openbooks-page", Json::Bool(true)),
            ("required", Json::List(vec![text("items")])),
            (
                "properties",
                map([
                    (
                        "items",
                        map([
                            ("type", text("array")),
                            ("items", schema(inner)),
                            ("description", text("This page's items, in order.")),
                        ]),
                    ),
                    (
                        "next",
                        map([
                            ("type", text("string")),
                            (
                                "description",
                                text("The cursor to pass as after for the next page; absent on the last."),
                            ),
                        ]),
                    ),
                ]),
            ),
        ]),
    }
}

pub(crate) fn object(fields: &[Field]) -> Json {
    let required = fields.iter().filter(|f| !matches!(f.schema, Schema::Maybe(_)));
    map([
        ("type", text("object")),
        ("additionalProperties", Json::Bool(false)),
        ("required", Json::List(required.map(|f| text(f.name)).collect())),
        (
            "properties",
            Json::Map(
                fields
                    .iter()
                    .map(|f| (f.name.to_owned(), schema(&f.schema).with("description", text(f.about))))
                    .collect(),
            ),
        ),
    ])
}

fn parameter(field: &Field, place: &str) -> Json {
    map([
        ("name", text(field.name)),
        ("in", text(place)),
        (
            "required",
            Json::Bool(place == "path" || !matches!(field.schema, Schema::Maybe(_))),
        ),
        ("description", text(field.about)),
        ("schema", schema(&field.schema)),
    ])
}

fn body(about: &str, shape: &Schema) -> Json {
    map([
        ("description", text(about)),
        ("content", map([("application/json", map([("schema", schema(shape))]))])),
    ])
}

fn operation(op: &Operation) -> Json {
    let org = Field {
        name: "org",
        about: "The company the operation is on.",
        schema: OrgId::schema(),
    };
    let input = op.input();
    let mut parameters = vec![parameter(&org, "path")];
    if op.is_read() {
        parameters.extend(input.iter().map(|f| parameter(f, "query")));
    }
    if op.is_create() {
        let key = Field {
            name: "Idempotency-Key",
            about: "The caller's key for this create: the same key and body get the first answer, another body a 409.",
            schema: Schema::Maybe(Box::new(Schema::Text)),
        };
        parameters.push(parameter(&key, "header"));
    }
    let mut answers = vec![match op.output() {
        Schema::Nothing => (
            "204".to_owned(),
            map([("description", text("Done, with nothing to answer."))]),
        ),
        shape => ("200".to_owned(), body("The answer.", &shape)),
    }];
    if op.is_create() {
        let reused = map([("description", text("The key was sent with another body."))]);
        answers.push(("409".to_owned(), reused));
    }
    let mut json = map([
        ("operationId", text(op.name())),
        ("description", text(op.about())),
        ("x-openbooks-word", text(op.word().as_str())),
        ("x-openbooks-tool", Json::Bool(op.is_tool())),
        ("parameters", Json::List(parameters)),
    ]);
    if !op.is_read() && !input.is_empty() {
        json = json.with("requestBody", body("What to write.", &Schema::Object(input)));
    }
    json.with("responses", Json::Map(answers))
}

impl Mount {
    /// The `OpenAPI` document of this mount: each operation under the mount's `key`, with the word it needs.
    pub fn openapi(&self) -> String {
        let paths = self.operations.iter().map(|op| {
            let method = if op.is_read() { "get" } else { "post" };
            let path = format!("/{}/{{org}}/{}", self.key, op.name());
            (path, map([(method, operation(op))]))
        });
        let doc = map([
            ("openapi", text("3.1.0")),
            (
                "info",
                map([
                    ("title", Json::Str(format!("openbooks {}", self.key))),
                    ("version", text("1")),
                    (
                        "description",
                        Json::Str(format!("The operations mounted under {}.", self.key)),
                    ),
                ]),
            ),
            ("paths", Json::Map(paths.collect())),
        ]);
        render(&doc) + "\n"
    }
}

pub(crate) fn render(json: &Json) -> String {
    let mut out = String::new();
    json.write(&mut out, 0);
    out
}
