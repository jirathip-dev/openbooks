default:
    @just --list

check:
    ./.gatekeeper/run check
    cargo test --workspace --locked

gen:
    cargo update --workspace
    npm install --package-lock-only --no-audit --no-fund
