use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use blocks::theme::{Scheme, Style, gate};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn misses(style: &Path, schemes: &[PathBuf]) -> Result<Vec<String>, Box<dyn Error>> {
    let style: Style = serde_json::from_str(&fs::read_to_string(style)?)?;
    let mut out = Vec::new();
    for path in schemes {
        let scheme: Scheme = serde_json::from_str(&fs::read_to_string(path)?)?;
        out.extend(
            gate(&scheme, style.fills)
                .into_iter()
                .map(|m| format!("{}: {} {:.2} < {}", path.display(), m.what, m.got, m.min)),
        );
    }
    Ok(out)
}

#[test]
fn the_web_theme_passes_the_scheme_and_style_gates() -> Result<(), Box<dyn Error>> {
    let theme = root().join("web/theme");
    let got = misses(
        &theme.join("style.json"),
        &[theme.join("light.json"), theme.join("dark.json")],
    )?;
    assert_eq!(got, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_low_contrast_scheme_fails_the_scheme_gate() -> Result<(), Box<dyn Error>> {
    let bad = root().join("crates/openbooks-server/tests/fixtures/low-contrast.json");
    let got = misses(&root().join("web/theme/style.json"), &[bad])?;
    assert!(got.iter().any(|m| m.contains("--mute on --bg")), "{got:?}");
    Ok(())
}
