# openbooks

Open double-entry accounting: books, AR/AP, projects, MCP-ready.

## Intent

- One open core for the books: a ledger in double entry, with receivables, payables and projects over it.
- The books always balance: each sub-ledger equals its control account after every posting.
- Nothing is lost: there is no delete; a wrong entry is reversed by a new one, and every write leaves one audit row.
- Every route is declared with the permission word it needs.
- Any country: the core names none; a country's words, taxes and forms live in its pack under `packs/`, and the
  suite passes with no pack installed.
- One build: nothing is behind a feature switch.

## Non-goals

- No country in the core: no tax rule, form or currency rule outside a pack.
- No delete, no hard purge of a posted entry.
- No feature switches and no per-customer builds.
- No payroll, no banking or payments rail of its own.

## Layout

| Path | What |
|---|---|
| `crates/openbooks-core` | the domain types and rules |
| `crates/openbooks-db` | storage, over the core |
| `crates/openbooks` | the library a host embeds |
| `crates/openbooks-server` | the standalone server |
| `packs/th` | the Thai pack, `openbooks-th`, and its web package `packs/th/web` |
| `web` | the web package; its theme in `web/theme` |
| `migrations` | the database migrations |
| `templates` | the print and message templates |

## Checks

`just check` runs `./.gatekeeper/run check` (packs core, rust and ts; every gate on but the line cap) and then
`cargo test --workspace`; `just gen` refreshes the lockfiles. The layers and bans live in `.drover/arch.toml`: no
self-hosted runner, no delete, no country word outside `packs/`, no feature switch, no crate over a pack. The theme
test runs Blocks' scheme and style gates over `web/theme`. CI is `.github/workflows/ci.yml`: the gates, and the
tests beside a Postgres service, on GitHub-hosted runners with read-only permissions; a fork's pull request gets no
secret.

## Releases

The five crates and both npm packages share one version, the workspace's. When `ci` goes green on `main` at a
version with no `v*` tag yet, `.github/workflows/release.yml` builds the server for Linux (x86_64, arm64), macOS
(arm64) and Windows (x86_64), publishes to crates.io and npm (`CARGO_REGISTRY_TOKEN`, `NPM_TOKEN`), and tags the
release with notes from the PR titles. The `local/surface` gate (`.gatekeeper/surface.sh`) diffs the public
surface against the last release: each crate's public items, each npm package's declarations (`mount()`'s props)
and each JSON document under `surface/` (the OpenAPI documents, template data shapes, event payloads, import and
export columns). A line that is gone or changed fails unless the version is raised to the next breaking one.

## Branch protection

On `main` and `staging`:

- Require a pull request before merging; no direct pushes, no force pushes, no deletions.
- Require status checks to pass, with `gates / done` and `test` as the required checks, and the branch up to date.
- Apply the rules to administrators too.

## License

Apache-2.0
