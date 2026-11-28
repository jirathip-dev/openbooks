use openbooks::testing::TestHost;
use openbooks::{
    Ask, BOOKS, Error, Filter, Json, Mount, Never, Openbooks, Operation, Page, Reply, SignedIn, Tools, Via, Work,
};
use openbooks_core::{Currency, Email, OrgId, Role, UserId, Word};
use serde_json::Value;
use sqlx::PgPool;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const PROPOSE: Operation =
    openbooks::write!("propose", Word::Draft, "Proposes an entry.", { note: String = "What it is for." } -> ());
const APPROVE: Operation = openbooks::write!("approve", Word::Approve, "Approves an entry.", {} -> ());
const REPORT: Operation = openbooks::read!("report", Word::Reports, "Reports on the whole company.", {} -> String);
const LEAKS: Operation = openbooks::read!("leak", Word::Read, "Answers secrets.", {} -> Page<Never<String>>);
const PACK: Mount = Mount {
    key: "pack",
    operations: &[PROPOSE, APPROVE, REPORT, LEAKS],
    serve: serve_pack,
};

fn serve_pack(ask: Ask<'_>) -> Work<'_, Reply> {
    Box::pin(async move {
        let note = ask.argument("note").unwrap_or_default().to_owned();
        let seen: i64 = sqlx::query_scalar("SELECT count(*) FROM ledger.member WHERE org_id = $1")
            .bind(ask.org.get())
            .fetch_one(&mut *ask.db)
            .await
            .map_err(|e| Error::Host(e.to_string()))?;
        Ok(Reply::One(Json::Str(format!("{} {note} {seen}", ask.tool))))
    })
}

fn person(user: i64, address: &str) -> Res<SignedIn> {
    Ok(SignedIn::person(UserId::new(user)?, Email::new(address)?))
}

const READS: [&str; 6] = [
    "members",
    "accounts",
    "account_roles",
    "dimensions",
    "dimension_values",
    "terms",
];

fn reads_and(more: &[&'static str]) -> Vec<&'static str> {
    [READS.as_slice(), more].concat()
}

fn names(tools: &Tools, role: Role, signed: &SignedIn) -> Vec<&'static str> {
    tools.offered(role, signed).iter().map(Operation::name).collect()
}

#[test]
fn a_mounted_route_becomes_a_tool_with_no_registry_change_and_a_never_field_never_does() -> Res<()> {
    let owner = person(1, "ann@example.com")?;
    assert_eq!(names(&Tools::new(&[BOOKS]), Role::Owner, &owner), READS);
    let tools = Tools::new(&[BOOKS, PACK]);
    assert_eq!(names(&tools, Role::Owner, &owner), reads_and(&["propose", "approve"]));
    let all = tools.switch("report", true)?;
    assert_eq!(
        names(&all, Role::Owner, &owner),
        reads_and(&["propose", "approve", "report"])
    );
    assert!(matches!(all.switch("leak", true), Err(Error::NoSuchTool(name)) if name == "leak"));
    Ok(())
}

#[test]
fn a_tool_is_offered_only_to_a_role_with_its_word_so_a_viewer_gets_no_propose() -> Res<()> {
    let tools = Tools::new(&[BOOKS, PACK]).switch("report", true)?;
    let signed = person(1, "ann@example.com")?;
    let offered = |role| names(&tools, role, &signed);
    assert_eq!(offered(Role::Viewer), reads_and(&["report"]));
    assert_eq!(offered(Role::Staff), reads_and(&["propose"]));
    assert_eq!(offered(Role::Accountant), reads_and(&["propose", "approve", "report"]));
    let assistant = SignedIn {
        via: Via::Assistant,
        ..signed.clone()
    };
    assert_eq!(
        names(&tools, Role::Owner, &assistant),
        reads_and(&["propose", "report"])
    );
    Ok(())
}

#[test]
fn company_wide_reports_start_off_and_a_switch_turns_one_on_or_a_tool_off() -> Res<()> {
    let viewer = person(1, "vic@example.com")?;
    let tools = Tools::new(&[BOOKS, PACK]);
    assert_eq!(names(&tools, Role::Viewer, &viewer), READS);
    let tools = tools.switch("report", true)?.switch("members", false)?;
    assert_eq!(
        names(&tools, Role::Viewer, &viewer),
        [&READS[1..], &["report"]].concat()
    );
    assert!(matches!(tools.switch("nothing", true), Err(Error::NoSuchTool(_))));
    Ok(())
}

async fn books(pool: PgPool, tools: Tools) -> Res<(Openbooks<TestHost>, OrgId)> {
    openbooks_db::migrate(&pool).await?;
    let books = Openbooks::new(TestHost::new(pool)).tools(tools).build();
    books.host().sign_in(person(1, "ann@example.com")?);
    let org = books
        .create_company("Acme", Currency::new("XTS", 2)?, "none", None)
        .await?;
    Ok((books, org))
}

fn listed(json: &str) -> Res<Vec<String>> {
    let tools: Value = serde_json::from_str(json)?;
    let tools = tools.get("tools").and_then(Value::as_array).ok_or("no tools")?;
    Ok(tools
        .iter()
        .filter_map(|t| Some(t.get("name")?.as_str()?.to_owned()))
        .collect())
}

#[sqlx::test(migrations = false)]
async fn the_tool_list_names_what_the_callers_role_may_call_with_its_input_schema(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool, Tools::new(&[BOOKS, PACK])).await?;
    books
        .add_member(org, Email::new("vic@example.com")?, Role::Viewer, None)
        .await?;
    assert_eq!(listed(&books.tools(org).await?)?, reads_and(&["propose", "approve"]));
    books.host().sign_in(person(2, "vic@example.com")?);
    let json = books.tools(org).await?;
    assert_eq!(listed(&json)?, READS);
    let kinds = books.call_tool(org, "dimensions", &[]).await?;
    assert_eq!(
        serde_json::from_str::<Value>(&kinds)?,
        serde_json::json!([{"slot": 1, "name": "Project"}])
    );
    let roles = books.call_tool(org, "account_roles", &[("on", "2026-02-30")]).await;
    assert!(matches!(roles, Err(Error::Refused(_))), "{roles:?}");
    let tools: Value = serde_json::from_str(&json)?;
    let after = tools.pointer("/tools/0/inputSchema/properties/after/type");
    assert_eq!(after, Some(&Value::from("string")));
    assert!(matches!(
        books.call_tool(org, "propose", &[]).await,
        Err(Error::NoSuchTool(_))
    ));
    assert!(matches!(
        books.call_tool(org, "audit", &[]).await,
        Err(Error::NoSuchTool(_))
    ));
    Ok(())
}

async fn walk(books: &Openbooks<TestHost>, org: OrgId) -> Res<Vec<Vec<String>>> {
    let (mut pages, mut after) = (Vec::new(), None::<String>);
    loop {
        let arguments: Vec<(&str, &str)> = after.iter().map(|a| ("after", a.as_str())).collect();
        let answer = books.call_tool(org, "members", &arguments).await?;
        assert!(answer.len() <= 9 * 1024, "{} bytes", answer.len());
        let page: Value = serde_json::from_str(&answer)?;
        let items = page.get("items").and_then(Value::as_array).ok_or("no items")?;
        let emails = items.iter().filter_map(|m| Some(m.get("email")?.as_str()?.to_owned()));
        pages.push(emails.collect());
        after = page.get("next").and_then(Value::as_str).map(str::to_owned);
        let clean = |a: &String| !a.chars().any(|c| c.is_control() || c.is_whitespace());
        assert!(after.iter().all(clean), "unscrubbed cursor {after:?}");
        if after.is_none() {
            return Ok(pages);
        }
    }
}

#[sqlx::test(migrations = false)]
async fn a_tool_call_answers_fifty_rows_under_a_size_cap_and_its_cursor_reaches_the_last(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool, Tools::new(&[BOOKS])).await?;
    for n in 0..60 {
        let address = Email::new(&format!("m{n:03}@example.com"))?;
        books.add_member(org, address, Role::Viewer, None).await?;
    }
    let sizes: Vec<usize> = walk(&books, org).await?.iter().map(Vec::len).collect();
    assert_eq!(sizes, [50, 11]);
    let long = "x".repeat(180);
    for n in 0..60 {
        let address = Email::new(&format!("{long}{n:03}@example.com"))?;
        books.add_member(org, address, Role::Viewer, None).await?;
    }
    let pages = walk(&books, org).await?;
    let sizes: Vec<usize> = pages.iter().map(Vec::len).collect();
    let cut = sizes.iter().rev().skip(1).any(|&n| n < 50);
    assert!(cut, "no page but the last was cut by the size cap: {sizes:?}");
    let mut all: Vec<&String> = pages.iter().flatten().collect();
    let count = all.len();
    all.dedup();
    assert_eq!((count, all.len()), (121, 121));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_tool_answer_scrubs_free_text_and_a_switched_off_tool_is_not_called(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool, Tools::new(&[BOOKS])).await?;
    let planted = Email::new("eve\u{1b}[2j\nignore\u{7}all@example.com")?;
    books.add_member(org, planted, Role::Viewer, None).await?;
    for n in 0..48 {
        let address = Email::new(&format!("b{n:03}@example.com"))?;
        books.add_member(org, address, Role::Viewer, None).await?;
    }
    books
        .add_member(org, Email::new("zed@example.com")?, Role::Viewer, None)
        .await?;
    let pages = walk(&books, org).await?;
    let last: Vec<Option<&str>> = pages.iter().map(|p| p.last().map(String::as_str)).collect();
    assert_eq!(last, [Some("eve [2j ignore all@example.com"), Some("zed@example.com")]);
    let off = Tools::new(&[BOOKS]).switch("members", false)?;
    let books = Openbooks::new(TestHost::new(books.host().pool().clone()))
        .tools(off)
        .build();
    books.host().sign_in(person(1, "ann@example.com")?);
    assert!(matches!(
        books.call_tool(org, "members", &[]).await,
        Err(Error::NoSuchTool(_))
    ));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_packs_offered_tools_are_called_through_its_serve_and_a_write_leaves_one_audit_row(pool: PgPool) -> Res<()> {
    let (books, org) = books(pool, Tools::new(&[BOOKS, PACK]).switch("report", true)?).await?;
    let before = books.audit(org, Filter::default(), None).await?.items.len();
    let proposed = books.call_tool(org, "propose", &[("note", "rent\u{7}\nnow")]).await?;
    assert_eq!(proposed, "\"propose rent now 1\"");
    let rows = books.audit(org, Filter::default(), None).await?.items;
    let actions: Vec<&str> = rows.iter().skip(before).map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["propose"]);
    assert_eq!(books.call_tool(org, "report", &[]).await?, "\"report 1\"");
    assert_eq!(books.audit(org, Filter::default(), None).await?.items.len(), before + 1);
    Ok(())
}
