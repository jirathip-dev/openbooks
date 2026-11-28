use std::fs;
use std::path::Path;

use openbooks::{BOOKS, Mount, Never, Operation, Page};
use openbooks_core::{Email, Word};
use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const WORDS: [&str; 5] = ["read", "reports", "draft", "approve", "own"];

fn is_word(value: &Value) -> bool {
    value.as_str().is_some_and(|w| {
        w.starts_with(|c: char| c.is_ascii_lowercase()) && w.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    })
}

fn pointed<'a>(node: &'a Value, pointer: &str) -> &'a Value {
    node.pointer(pointer).unwrap_or(&Value::Null)
}

fn described(node: &Value) -> bool {
    node.get("description")
        .and_then(Value::as_str)
        .is_some_and(|d| !d.trim().is_empty())
}

fn walk(schema: &Value, at: &str, page_items: bool, answer: bool, out: &mut Vec<String>) {
    let kind = schema.get("type").and_then(Value::as_str);
    match kind {
        None => out.push(format!("typed: {at} has no type")),
        Some("object") if schema.get("additionalProperties") != Some(&Value::Bool(false)) => {
            out.push(format!("typed: {at} takes any property"));
        }
        Some("array") if !page_items => out.push(format!("paged: {at} is a list outside a page")),
        _ => {}
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.iter().all(is_word)
    {
        out.push(format!("words: {at} has an enum value that is not a word"));
    }
    if answer && schema.get("x-openbooks-tier").and_then(Value::as_str) == Some("never") {
        out.push(format!("never: {at} is a never-tier field in a tool's answer"));
    }
    if let Some(items) = schema.get("items") {
        walk(items, &format!("{at}[]"), false, answer, out);
    }
    let paged = schema.get("x-openbooks-page") == Some(&Value::Bool(true));
    for (name, property) in schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if !described(property) {
            out.push(format!("description: {at}.{name} has none"));
        }
        walk(property, &format!("{at}.{name}"), paged && name == "items", answer, out);
    }
}

fn check_operation(at: &str, op: &Value, out: &mut Vec<String>) {
    if !described(op) {
        out.push(format!("description: {at} has none"));
    }
    if !op
        .get("x-openbooks-word")
        .and_then(Value::as_str)
        .is_some_and(|w| WORDS.contains(&w))
    {
        out.push(format!("word: {at} is not declared with a permission word"));
    }
    let tool = op.get("x-openbooks-tool") == Some(&Value::Bool(true));
    for (n, parameter) in pointed(op, "/parameters").as_array().into_iter().flatten().enumerate() {
        if !described(parameter) {
            out.push(format!("description: {at} parameter {n} has none"));
        }
        walk(
            pointed(parameter, "/schema"),
            &format!("{at} parameter {n}"),
            false,
            false,
            out,
        );
    }
    if let Some(body) = op.get("requestBody") {
        if !described(body) {
            out.push(format!("description: {at} body has none"));
        }
        walk(
            pointed(body, "/content/application~1json/schema"),
            &format!("{at} body"),
            false,
            false,
            out,
        );
    }
    for (code, response) in pointed(op, "/responses").as_object().into_iter().flatten() {
        if !described(response) {
            out.push(format!("description: {at} {code} has none"));
        }
        if let Some(schema) = response.pointer("/content/application~1json/schema") {
            walk(schema, &format!("{at} {code}"), false, tool, out);
        }
    }
}

fn check(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (path, methods) in pointed(doc, "/paths").as_object().into_iter().flatten() {
        for (method, op) in methods.as_object().into_iter().flatten() {
            check_operation(&format!("{method} {path}"), op, &mut out);
        }
    }
    out
}

fn rules(doc: &Value) -> Vec<String> {
    let mut rules: Vec<String> = check(doc)
        .iter()
        .filter_map(|v| v.split_once(':').map(|(rule, _)| rule.to_owned()))
        .collect();
    rules.dedup();
    rules
}

fn books() -> Res<Value> {
    Ok(serde_json::from_str(&BOOKS.openapi())?)
}

fn plant(doc: &mut Value, pointer: &str, value: Value) -> Res<()> {
    let (parent, key) = pointer.rsplit_once('/').ok_or("no key")?;
    let parent = doc.pointer_mut(parent).and_then(Value::as_object_mut);
    parent.ok_or(pointer.to_owned())?.insert(key.to_owned(), value);
    Ok(())
}

const MEMBERS: &str = "/paths/~1books~1{org}~1members/get";
const MEMBER: &str =
    "/paths/~1books~1{org}~1members/get/responses/200/content/application~1json/schema/properties/items/items";

#[test]
fn the_books_document_passes_the_shape_check_and_is_the_one_under_surface() -> Res<()> {
    let doc = books()?;
    assert_eq!(check(&doc), Vec::<String>::new());
    let ops = pointed(&doc, "/paths");
    let declared: Vec<(&str, &str, bool)> = ops
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, m)| m.as_object().into_iter().flatten())
        .map(|(_, op)| {
            let field = |k: &str| op.get(k).and_then(Value::as_str).unwrap_or_default();
            (
                field("operationId"),
                field("x-openbooks-word"),
                pointed(op, "/x-openbooks-tool") == true,
            )
        })
        .collect();
    assert_eq!(
        declared,
        [
            ("member.add", "own", false),
            ("member.remove", "own", false),
            ("member.role", "own", false),
            ("members", "read", true)
        ]
    );
    let surface = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../surface/books.openapi.json");
    let written = fs::read_to_string(&surface).unwrap_or_default();
    if written != BOOKS.openapi() {
        fs::write(&surface, BOOKS.openapi())?;
    }
    assert!(
        written == BOOKS.openapi(),
        "surface/books.openapi.json was stale: rewritten, commit it"
    );
    Ok(())
}

#[test]
fn the_check_fails_a_planted_violation_of_each_rule() -> Res<()> {
    let member = books()?.pointer(MEMBER).cloned().ok_or("no member")?;
    let plants = [
        (
            "description",
            format!("{MEMBER}/properties/email/description"),
            json!(""),
        ),
        ("description", format!("{MEMBERS}/description"), json!(" ")),
        (
            "paged",
            format!("{MEMBERS}/responses/200/content/application~1json/schema"),
            json!({"type": "array", "items": member}),
        ),
        ("words", format!("{MEMBER}/properties/role/enum"), json!([0, 1, 2, 3])),
        (
            "words",
            format!("{MEMBER}/properties/role/enum"),
            json!(["Owner", "staff"]),
        ),
        (
            "typed",
            format!("{MEMBER}/properties/role"),
            json!({"description": "Their role."}),
        ),
        ("typed", format!("{MEMBER}/additionalProperties"), json!(true)),
        (
            "never",
            format!("{MEMBER}/properties/email/x-openbooks-tier"),
            json!("never"),
        ),
        ("word", format!("{MEMBERS}/x-openbooks-word"), json!("admin")),
    ];
    for (rule, pointer, value) in plants {
        let mut doc = books()?;
        plant(&mut doc, &pointer, value)?;
        assert_eq!(rules(&doc), [rule], "{pointer}: {:?}", check(&doc));
    }
    Ok(())
}

const LEAKS: Operation = openbooks::read!("leak", Word::Read, "Answers a secret.", {} -> Never<String>);
const SHOWS: Operation = openbooks::closed!("show", Word::Own, "Shows a secret to a person.", {} -> Never<String>);
const QUIET: Operation = openbooks::write!("quiet", Word::Draft, "", { email: Email = "" } -> Page<String>);

#[test]
fn a_declared_tool_answering_a_never_field_or_lacking_descriptions_fails_and_a_closed_one_may_answer_it() -> Res<()> {
    let doc = |operations| serde_json::from_str::<Value>(&Mount { key: "t", operations }.openapi());
    assert_eq!(rules(&doc(&[LEAKS])?), ["never"]);
    assert_eq!(check(&doc(&[SHOWS])?), Vec::<String>::new());
    assert_eq!(rules(&doc(&[QUIET])?), ["description"]);
    assert_eq!(check(&doc(&[QUIET])?).len(), 2, "{:?}", check(&doc(&[QUIET])?));
    Ok(())
}
