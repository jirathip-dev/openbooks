use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Res<T> = Result<T, Box<dyn Error>>;

const TAG: &str = "git init -q && git add -A && git -c user.name=f -c user.email=f@f commit -qm v && git tag v0.1.0";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn overlay(from: &Path, to: &Path) -> Res<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&dest)?;
            overlay(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn arch_gate(name: &str, fixture: Option<&Path>) -> Res<(bool, String)> {
    let tree = Path::new(env!("CARGO_TARGET_TMPDIR")).join("arch").join(name);
    if tree.exists() {
        fs::remove_dir_all(&tree)?;
    }
    for config in [".gatekeeper/run", ".gatekeeper/config.toml", ".drover/arch.toml"] {
        let dest = tree.join(config);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::copy(root().join(config), dest)?;
    }
    overlay(&root().join(".gatekeeper"), &tree.join(".gatekeeper"))?;
    let before = fixture.map(|f| f.with_file_name("before")).filter(|b| b.exists());
    if let Some(before) = &before {
        overlay(before, &tree)?;
        Command::new("sh").args(["-c", TAG]).current_dir(&tree).status()?;
    }
    if let Some(fixture) = fixture {
        overlay(fixture, &tree)?;
    }
    let gate = if before.is_some() { "local/surface" } else { "core/arch" };
    let out = Command::new(tree.join(".gatekeeper/run"))
        .args(["check", "--gate", gate])
        .current_dir(&tree)
        .output()?;
    let log = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Ok((out.status.success(), log))
}

#[test]
fn the_arch_config_passes_alone_and_fails_each_fixture_with_its_rule() -> Res<()> {
    let (ok, log) = arch_gate("clean", None)?;
    assert!(ok, "clean: {log}");
    let mut cases = fs::read_dir(root().join(".drover/fixtures"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    cases.sort();
    assert_eq!(cases.len(), 18);
    for case in cases {
        let name = case.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let expect = fs::read_to_string(case.join("expect"))?;
        let (ok, log) = arch_gate(name, Some(&case.join("tree")))?;
        assert!(!ok && log.contains(expect.trim()), "{name}: {log}");
    }
    Ok(())
}
