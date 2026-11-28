use std::{error::Error, fs, path::Path};

fn unscoped(queries: &[String]) -> Vec<&String> {
    let on_ledger = |q: &&String| q.to_lowercase().contains("ledger.") && !q.contains("org_id");
    queries.iter().filter(on_ledger).collect()
}

#[test]
fn every_cached_statement_on_a_ledger_table_names_org_id_and_a_planted_one_fails() -> Result<(), Box<dyn Error>> {
    let mut queries = Vec::new();
    for entry in fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".sqlx"))? {
        let data: serde_json::Value = serde_json::from_str(&fs::read_to_string(entry?.path())?)?;
        queries.extend(data["query"].as_str().map(str::to_owned));
    }
    assert!(!queries.is_empty(), "the query cache is empty");
    assert_eq!(unscoped(&queries), Vec::<&String>::new());
    let planted = String::from("SELECT amount_minor FROM ledger.line WHERE entry_id = $1");
    queries.push(planted.clone());
    assert_eq!(unscoped(&queries), [&planted]);
    Ok(())
}
