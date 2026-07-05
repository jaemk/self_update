# Contributing

## Verifying changes

Run the full CI pipeline locally before opening a PR:

```
make ci
```

This runs `cargo fmt`, `cargo clippy`, and `cargo test` on both http clients (reqwest and
ureq), checks the README is in sync, and builds every backend example. `make help` lists the
individual targets.

## README

`README.md` is generated from the crate docs in `src/lib.rs` using
[cargo-readme](https://crates.io/crates/cargo-readme). Never edit it directly; edit the doc
comment in `src/lib.rs` and regenerate:

```
cargo install cargo-readme
./readme.sh          # regenerate README.md
./readme.sh check    # verify it is in sync
```

## Changelog

Add a summary of your change to the `[unreleased]` section of `CHANGELOG.md`.

## Agent skills

Agent-agnostic skills live in `.agents/skills/` (see [AGENTS.md](AGENTS.md) for the full
list). In particular, the `pr-review` skill runs a read-only review of your branch or PR
locally, so you can review your own changes before pushing.

## Releases

Publishing is done by the CI pipeline: every push to master runs the full check suite and then
`cargo publish`, which only releases a new version when the version in `Cargo.toml` has been
bumped. So if your change should go out immediately, use the `release` skill (or make the
equivalent `CHANGELOG.md` updates) and bump the version in `Cargo.toml` as part of your PR.
Otherwise the change sits in `[unreleased]` until a separate version-bump change triggers the
next release.
