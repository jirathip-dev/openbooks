#!/bin/sh
set -eu
root=$(git rev-parse --show-toplevel)
tree=${CARGO_TARGET_DIR:-$root/target}/surface-fixture
rm -rf "${tree:?}"
mkdir -p "$tree/.gatekeeper" "$tree/crates/demo/src"
cp "$root/.gatekeeper/surface.sh" "$root/.gatekeeper/surface.jq" "$tree/.gatekeeper/"
cp "$root/rust-toolchain.toml" "$tree/"
cd "$tree"
release() {
    printf '[workspace]\nmembers = ["crates/*"]\nresolver = "3"\n\n[workspace.package]\nversion = "%s"\nedition = "2024"\n' "$1" >Cargo.toml
    printf 'version = 4\n\n[[package]]\nname = "demo"\nversion = "%s"\n' "$1" >Cargo.lock
}
commit() { git add -A && git -c user.name=fixture -c user.email=fixture@localhost commit -qm "$1"; }
expect() {
    code=0
    sh .gatekeeper/surface.sh >log 2>&1 || code=$?
    if [ "$1" = pass ]; then [ "$code" = 0 ]; else [ "$code" != 0 ]; fi && grep -qF -- "$2" log || {
        cat log >&2
        echo "surface fixture: wanted $1 with: $2" >&2
        exit 1
    }
}
printf 'target/\nlog\n' >.gitignore
release 0.1.0
printf '[package]\nname = "demo"\nversion.workspace = true\nedition.workspace = true\n' >crates/demo/Cargo.toml
printf 'pub fn kept() {}\npub fn gone() {}\n' >crates/demo/src/lib.rs
git init -q
commit release
git tag v0.1.0
expect pass "every public item of v0.1.0 is still there"
printf 'pub fn kept() {}\npub fn added() {}\n' >crates/demo/src/lib.rs
commit "remove gone"
expect fail "function demo::gone"
expect fail "raise the version 0.1.0 to 0.2.0"
! grep -q 'demo::kept' log || { cat log >&2; echo "surface fixture: an unchanged item was reported" >&2; exit 1; }
release 0.2.0
commit 0.2.0
expect pass "the version 0.2.0 is at least 0.2.0"
echo "surface fixture: a removed pub fn fails at 0.1.0 and passes at 0.2.0"
