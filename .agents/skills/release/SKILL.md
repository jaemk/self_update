---
name: release
description: Prepare a release (bump the crate version, update CHANGELOG.md with a migration guide for breaking changes, regenerate README, commit), or run a pre-release review. The `review` option kicks off a consistency review and an API-examination review to surface lingering inconsistencies and a categorized list of breaking and non-breaking improvements before you cut the release — advisory only, it changes nothing. Use when asked to "cut a release", "bump the version", "prepare a release", "release X.Y.Z", or "do a release review" / "review for release". Takes a version string (e.g. `/release 1.2.0`) or `review` as the argument.
---

# Release

Prepare a new release by bumping the version and updating the changelog — or, with the
`review` option, run a pre-release audit that surfaces inconsistencies and improvement
ideas without changing anything.

`self_update` is a single crate; the version lives only in the top-level `Cargo.toml`.

## Input

The argument selects the mode:

- **`review`** (or "release review", "review for release", "pre-release review") — run the
  **Release review** below. This is advisory only: it bumps nothing, edits no files, and
  makes no commit. Run it *before* cutting a release.
- **A version string** (e.g. `1.2.0`) — run the **Version bump** flow (the numbered steps).
  If no argument is given and the intent is clearly to cut a release, ask which version.

If the user asks for a release review and then to proceed, run the review first and the
version bump second.

## Release review

A pre-release audit. Run it **before** cutting a release — especially a major version,
since a major bump is the only window in which breaking API changes are acceptable. It
**does not** bump the version, edit the changelog/README, or commit anything; it produces
a report. It kicks off two complementary reviews, then synthesizes them.

### A. Consistency review

Audit the public surface for lingering inconsistencies across the **whole crate**, not
just the latest diff. Scope it to everything accumulated since the last released version.

Establish the baseline with plain, side-effect-free commands — run each separately rather
than nesting a `$(…)` command substitution, so each invocation can be statically verified
and auto-approved (a `$(git …)` substitution defeats static analysis and forces a prompt):

```bash
git describe --tags --abbrev=0    # last released tag
git tag --sort=-creatordate       # full tag list, newest first
git log --oneline -20             # recent commits — find the last "release" commit
```

Read the last-released **version** off the result, then diff against it explicitly, e.g.:

```bash
git diff v0.44.0..HEAD --stat     # substitute the real last-released ref
```

Cross-check the tag against the `[…]` version headings in `CHANGELOG.md` and the most
recent `release` commit in the log; if they disagree, use the last *actually released*
commit (the one matching the newest released CHANGELOG section) as the diff baseline.

Check for:
- **Naming parity** — do sibling backends (`github` / `gitlab` / `gitea` / `s3`) share
  vocabulary on their builders? As of 1.0 the shared setters (`url`, `target`, `identifier`,
  `auth_token`, `access_key`, …) are unified via `impl_common_builder_setters!` and
  `CommonConfig`; flag any backend that re-introduces an odd-one-out name or diverges from the
  shared macro.
- **Builder / entry symmetry** — does every backend expose the same entry (`configure()`),
  the same fallible `build()`, and the same shared setters where they make sense?
- **Trait-method parity** — does the shared release-update trait in `src/update.rs` (and
  any helper traits) expose a consistent method set with consistent signatures
  (`&self` vs `&mut self`)?
- **Feature-gate symmetry** — is each public item gated behind exactly the features it
  needs? Are the `reqwest`/`ureq` and `default-tls`/`rustls` selections handled
  consistently, and is the mutual exclusion surfaced clearly?
- **Doc / CHANGELOG alignment** — do the `src/lib.rs` docs, the `[unreleased]` CHANGELOG
  section, and any migration notes accurately describe the shipped API (named types,
  signatures, builder/setter names, feature gates)?

Delegate the correctness/idiom sweep to a `pr-code-reviewer` sub-agent fed the full
release diff, and reason through the cross-backend parity items yourself — they need a
whole-surface view the diff alone does not give.

### B. API examination review

Run the `consumer-experience-review` skill. It builds a throwaway external consumer that
depends on the crate the way a real user would and surfaces API gaps, naming
inconsistencies, trait-import friction, and feature-flag dead-ends with
**compiler-verified evidence**. This is the authoritative "what would a new user trip
over" pass.

### C. Synthesize the report

Merge both reviews into a single report. **Apply no change** — this mode is advisory.
Produce:

1. **Lingering inconsistencies** — a flat list of every inconsistency found, each with a
   `file:line` (or API path) and a one-line description.
2. **Recommended changes, split by impact:**
   - **Breaking** — changes that alter the public API (renames, signature changes, removed
     items, new required trait methods). For each: what to change, why it improves the
     library, and the migration cost. Mark these "land now or wait for the next major".
   - **Non-breaking** — additive or internal changes (new constructors, doc fixes, new
     re-exports, deprecations that keep the old path working). For each: what to change and
     why.
3. **Recommendation** — is the library consistent enough to release as-is, or are there
   high-impact items that should land in this version first?

After presenting the report, ask whether to address findings first or proceed to the
version bump below.

## Version bump

These numbered steps are the default mode — run them when the argument is a version
string, or after a release review once the user opts to proceed.

### 1. Update `Cargo.toml` version

In the top-level `Cargo.toml`, set `[package] version` to the new version. Use precise
string replacement — do not touch dependency versions.

### 2. Update `CHANGELOG.md`

- Replace `## [unreleased]` with `## [X.Y.Z]`.
- Add a fresh `## [unreleased]` section (with empty `### Added` / `### Changed` / `### Removed`) above the new version heading — the changelog must always have an `[unreleased]` section at the top.
- Ensure the new version's `### Added` / `### Changed` / `### Removed` bullets accurately describe `git diff <last-release>..HEAD` — named types, builder/setter names, and feature gates must match the code exactly.

### 3. Migration guide (breaking releases)

This repo keeps migration guidance **inside the CHANGELOG entry** rather than a separate
`docs/migrations/` tree (see the `[1.0.0]` entry for the established format). For a release
with breaking changes, the version's CHANGELOG entry must include a **Migration guide**
section that is terse, mechanical, and grep-friendly:

- one line per breaking change with what to search for and the exact code transformation
- the `Cargo.toml` change if a feature/flag default changed
- For a purely additive release, no migration section is needed.

### 4. Regenerate README

`README.md` is generated from the `src/lib.rs` crate doc — never edit it directly.

```bash
cargo readme --no-indent-headings > README.md
```

(equivalently, `./readme.sh`).

### 5. Verify

Build the full feature set on both http clients and run the tests:

```bash
cargo test --features "archive-tar archive-zip compression-flate2 compression-zip-deflate compression-zip-bzip2 signatures s3-auth"
cargo build --no-default-features --features "ureq default-tls archive-tar archive-zip compression-flate2 compression-zip-deflate compression-zip-bzip2 signatures s3-auth"
```

Fix any compilation errors before proceeding.

### 6. Commit or amend

If there is already a single release-prep commit ahead of master on this branch, amend it:
```bash
git add Cargo.toml CHANGELOG.md README.md
git commit --amend --no-edit
```

Otherwise create a new commit:
```bash
git commit -m "release: bump version to X.Y.Z"
```

### 7. Report

Tell the user:
- The new version
- That README was regenerated
- Whether a migration guide was added (and why, or why not)
- The resulting commit SHA
