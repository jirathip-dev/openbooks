#!/bin/sh
set -eu
root=$(git rev-parse --show-toplevel)
cd "$root"
out=${CARGO_TARGET_DIR:-$root/target}
work=$out/surface
fail() {
    echo "surface: $*" >&2
    exit 1
}
field() { sed -n "/^\[$1\]/,/^\[/s/^$2 = \"\(.*\)\"/\1/p" "$3"; }

version=$(field workspace.package version Cargo.toml)
for m in crates/*/Cargo.toml packs/*/Cargo.toml; do
    [ ! -f "$m" ] || grep -qx 'version.workspace = true' "$m" || fail "$m: needs version.workspace = true, one version for every crate"
done
for p in web/package.json packs/*/web/package.json; do
    [ ! -f "$p" ] || [ "$(jq -r .version "$p")" = "$version" ] || fail "$p: version is not the workspace's $version"
done

if git remote get-url origin >/dev/null 2>&1; then
    depth=
    [ "$(git rev-parse --is-shallow-repository)" = false ] || depth=--depth=1
    git fetch --quiet $depth origin '+refs/tags/v*:refs/tags/v*' || fail "cannot fetch the release tags from origin"
fi
tag=$(git tag -l 'v*' --sort=-v:refname | head -n 1)
if [ -z "$tag" ]; then
    echo "surface: no release yet, nothing to compare"
    exit 0
fi

surface() (
    cd "$1"
    for m in crates/*/Cargo.toml packs/*/Cargo.toml; do
        [ -f "${m%Cargo.toml}src/lib.rs" ] || continue
        name=$(field package name "$m")
        RUSTC_BOOTSTRAP=1 CARGO_TARGET_DIR=$out cargo rustdoc --quiet --locked -p "$name" --lib -- -Z unstable-options --output-format json
        jq -r -f "$root/.gatekeeper/surface.jq" "$out/doc/$(echo "$name" | tr - _).json"
    done
    for p in web packs/*/web; do
        [ -f "$p/tsconfig.json" ] || continue
        [ -x "$root/node_modules/.bin/tsc" ] || (cd "$root" && npm ci --no-audit --no-fund >&2)
        rm -rf "${work:?}/dts"
        "$root/node_modules/.bin/tsc" -p "$p" --noEmit false --declaration --emitDeclarationOnly --rootDir "$p/src" --outDir "$work/dts"
        (cd "$work/dts" && find . -name '*.d.ts' | sort | while read -r f; do sed "s|^|ts $p/${f#./}: |" "$f"; done)
    done
    [ ! -d surface ] || find surface -name '*.json' | sort | while read -r f; do
        jq -r --arg f "$f" 'paths(scalars) as $p | select($p[-1] | IN("description", "summary") | not) | "json \($f) \($p | map(tostring) | join(".")) = \(getpath($p) | tojson)"' "$f"
    done
)

git worktree remove --force "$work/base" 2>/dev/null || true
rm -rf "${work:?}"
git worktree prune
mkdir -p "$work"
git worktree add --quiet --detach "$work/base" "$tag"
trap 'git worktree remove --force "$work/base"' EXIT
surface "$work/base" >"$work/base.raw"
surface "$root" >"$work/head.raw"
sort -u "$work/base.raw" >"$work/base.txt"
sort -u "$work/head.raw" >"$work/head.txt"
removed=$(comm -23 "$work/base.txt" "$work/head.txt")
if [ -z "$removed" ]; then
    echo "surface: every public item of $tag is still there"
    exit 0
fi
old=${tag#v}
major=${old%%.*}
rest=${old#*.}
minor=${rest%%.*}
if [ "$major" = 0 ]; then next=0.$((minor + 1)).0; else next=$((major + 1)).0.0; fi
if [ "$(printf '%s\n%s\n' "$next" "$version" | sort -V | head -n 1)" = "$next" ]; then
    echo "surface: breaking change against $tag, and the version $version is at least $next"
    exit 0
fi
printf '%s\n' "$removed" >&2
fail "the lines above were in $tag and are gone or changed: put them back, or raise the version $version to $next"
