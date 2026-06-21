# Transactional multi-file install (G6)

Status: implemented

## Problem

`update()` extracts exactly one binary and replaces the current exe. A tool that
ships more than a single binary (a binary plus sidecar libraries or resources), or
that updates a file other than `current_exe()`, had to drop to manual
`Download` + `Extract` + `Move` with no atomicity across files.

## Decision

A `MoveAll` primitive applies a set of `(source -> dest)` renames all-or-nothing:
either every move succeeds, or on the first failure each applied move is rolled back
via atomic rename-only restore (same-filesystem constraint, like
`Move::replace_using_temp`). A documented cookbook covers the pattern: `extract_into`
the whole archive, then `MoveAll`. `MoveAll` is `#[non_exhaustive]`.

See `MoveAll` in `src/lib.rs`, the crate-doc cookbook, and the CHANGELOG `[1.0.0]`
Added entry.
