# Release-tag URL encoding

Status: implemented

## Problem

The fetch-by-tag paths interpolated the caller-supplied tag directly into the request URL with no
percent-encoding. `github::get_release_version` built `.../releases/tags/{ver}`, gitea built
`.../releases/tags/{ver}`, and gitlab built `.../releases/{ver}` (no `tags/` segment). A tag
containing URL-special characters (a space, `#`, `?`, `+`) produced a malformed request rather
than a clean lookup.

## Resolution

The tag segment is wrapped in `urlencoding::encode` at every fetch-by-tag site on github, gitlab,
and gitea (both the sync and the async paths). `urlencoding` is already an unconditional
dependency (`Cargo.toml`) and is used the same way for the gitlab `repo_owner` segment, so this
added no dependency. Covered by a test asserting the encoded tag in the request path.
