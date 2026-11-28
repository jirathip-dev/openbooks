use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type Res<T> = Result<T, Box<dyn Error>>;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write(tree: &Path, path: &str, text: &str) -> Res<()> {
    let dest = tree.join(path);
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(dest, text)?;
    Ok(())
}

fn workspace(version: &str) -> String {
    format!(
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n\n[workspace.package]\nversion = \"{version}\"\nedition = \"2024\"\n"
    )
}

fn lock(version: &str) -> String {
    format!("version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"{version}\"\n")
}

fn run(tree: &Path, program: &str, args: &[&str]) -> Res<(bool, String)> {
    let out = Command::new(program)
        .args(args)
        .current_dir(tree)
        .env_remove("CARGO_TARGET_DIR")
        .output()?;
    let log = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Ok((out.status.success(), log))
}

fn commit(tree: &Path, message: &str) -> Res<()> {
    for args in [
        &["add", "-A"][..],
        &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", message],
    ] {
        let (ok, log) = run(tree, "git", args)?;
        assert!(ok, "git {args:?}: {log}");
    }
    Ok(())
}

fn gate(tree: &Path) -> Res<(bool, String)> {
    run(tree, "sh", &[".gatekeeper/surface.sh"])
}

#[test]
fn a_removed_public_item_fails_until_the_version_moves_to_the_next_breaking_one() -> Res<()> {
    let tree = Path::new(env!("CARGO_TARGET_TMPDIR")).join("surface");
    if tree.exists() {
        fs::remove_dir_all(&tree)?;
    }
    for file in [
        ".gatekeeper/surface.sh",
        ".gatekeeper/surface.jq",
        "rust-toolchain.toml",
    ] {
        write(&tree, file, &fs::read_to_string(root().join(file))?)?;
    }
    write(&tree, ".gitignore", "target/\n")?;
    write(&tree, "Cargo.toml", &workspace("0.1.0"))?;
    write(&tree, "Cargo.lock", &lock("0.1.0"))?;
    write(
        &tree,
        "crates/demo/Cargo.toml",
        "[package]\nname = \"demo\"\nversion.workspace = true\nedition.workspace = true\n",
    )?;
    write(&tree, "crates/demo/src/lib.rs", "pub fn kept() {}\npub fn gone() {}\n")?;
    assert!(run(&tree, "git", &["init", "-q"])?.0);
    commit(&tree, "release")?;
    assert!(run(&tree, "git", &["tag", "v0.1.0"])?.0);

    let (ok, log) = gate(&tree)?;
    assert!(
        ok && log.contains("every public item of v0.1.0 is still there"),
        "{log}"
    );

    write(&tree, "crates/demo/src/lib.rs", "pub fn kept() {}\npub fn added() {}\n")?;
    commit(&tree, "remove gone")?;
    let (ok, log) = gate(&tree)?;
    assert!(
        !ok && log.contains("function demo::gone") && log.contains("raise the version 0.1.0 to 0.2.0"),
        "{log}"
    );
    assert!(!log.contains("demo::kept"), "{log}");

    write(&tree, "Cargo.toml", &workspace("0.2.0"))?;
    write(&tree, "Cargo.lock", &lock("0.2.0"))?;
    commit(&tree, "0.2.0")?;
    let (ok, log) = gate(&tree)?;
    assert!(ok && log.contains("the version 0.2.0 is at least 0.2.0"), "{log}");
    Ok(())
}
