# Auth token from env, and rate-limit errors

Status: done (decided 2026-07-26; implemented 2026-08-27)

## Problem

An update check behind a shared egress IP (a NAT'd corporate network) fails with
HTTP 403 once the unauthenticated GitHub REST budget, 60 requests/hour counted
per source IP, is exhausted by everyone sharing that IP. The fix is to send a
token, but the crate makes each consumer plumb one in itself, and the resulting
403 is indistinguishable from a real credential failure.

Current behavior:

- `auth_token(impl Into<String>)` on every backend builder is the only way to
  supply a token (`src/backends/github.rs:147`). There is no env-var path, so
  every consumer writes the same `std::env::var("GITHUB_TOKEN")` plumbing,
  including the skip-when-empty case.
- The token is already forwarded safely: `apply_auth` attaches it only to a URL
  whose host matches the configured API base or an `allow_auth_host` entry, and
  only over https (`src/backends/common.rs:322-356`), with a per-backend scheme
  (github/gitea `Token`, gitlab `Bearer`, `common.rs:196`). Nothing about the
  env source changes that gate.
- A rate-limited response surfaces as `Error::Unauthorized { status: 403, url }`
  (`src/errors.rs:75`), the same variant as a bad token, so a caller cannot tell
  "wait for the window to reset, or set a token" from "these credentials are
  wrong". README:360 documents the limits and tells the reader to recognize the
  rate-limit case by its symptom.

## AUTH-1: token from the environment

AUTH-1-1. `auth_token_from_env()` is added to the backend `UpdateBuilder` and
`ReleaseListBuilder` types that take an `auth_token`. It reads the backend's
conventional env vars in order and uses the first that is present and non-empty
after trimming surrounding whitespace:

- github: `GITHUB_TOKEN`, then `GH_TOKEN` (matching the `gh` CLI).
- gitlab: `GITLAB_TOKEN`, then `CI_JOB_TOKEN`.
- gitea: `GITEA_TOKEN`.
- gitee: `GITEE_TOKEN`.

AUTH-1-2. No variable set (or all empty) leaves `auth_token` unset: the request
goes out unauthenticated exactly as today, no error. This makes the call safe to
place unconditionally in an application that also runs outside CI or a corporate
network.

AUTH-1-3. Reading env is opt-in, never automatic. A library that harvests
credentials from the environment without being asked is surprising, and the
configured API base can be a self-hosted host, so an implicit read would decide
on its own to send a user's token somewhere. The explicit call keeps the
decision with the embedding application. `auth_token(..)` and
`auth_token_from_env()` are last-setter-wins **when the environment supplies a
token**; a lookup that finds nothing leaves the existing value alone rather than
clearing it (implementation note, 2026-08-27: the strict last-wins reading would
let `auth_token(t).auth_token_from_env()` silently drop `t` on a machine with no
variable set, turning a convenience call into a surprise 403 against a private
repo -- the additive rule is the safe half of the ambiguity in AUTH-1-2's
"leaves `auth_token` unset"). Enforced by `apply_env_token`
(`backends/common.rs`).

AUTH-1-4. The env read happens in the setter (not at request time), so the
resolved value is visible in the builder's `Debug` output (redacted as
`<token>`, `common.rs:273`) and the behavior does not depend on env changes made
later in the process.

AUTH-1-5. Tests: the env-var precedence is exercised through a pure helper
taking the candidate `(name, value)` pairs, so no test mutates process env
(which is racy under the parallel test harness). Cover first-wins, empty-skip,
whitespace-trim, and none-set.

## AUTH-2: distinguishable rate-limit error

AUTH-2-1. New variant `Error::RateLimited { status, url, reset_at, retry_after }`
(`Error` is `#[non_exhaustive]`, `src/errors.rs:21`, so this is a minor-version
addition). `reset_at` is the parsed reset instant (`Option<SystemTime>`) when the
response carries one, `retry_after` the `Retry-After` delay
(`Option<Duration>`; only the delta-seconds form is parsed, the HTTP-date form
yields `None` rather than adding a date-parsing dependency).

AUTH-2-2. A 403 (or 429) response is classified as `RateLimited` instead of
`Unauthorized` when it carries a zero remaining-quota header:
`x-ratelimit-remaining: 0` (github, gitea, gitee) or `RateLimit-Remaining: 0`
(gitlab). Absent that header the classification is unchanged. (Implementation
note, 2026-08-27: the remaining-quota header alone triggers the classification --
a companion reset header is picked up when present but is not required, since the
two spellings otherwise carry different rules for no benefit. `HeaderMap` lookups
are case-insensitive, so both spellings are read through one pair of names.)

AUTH-2-6 (added 2026-08-27). The same classification is reachable from a custom
`HttpClient` via the public `Error::http_status_error_with_headers(status, url,
&HeaderMap)`; the header-less `Error::http_status_error` keeps its behavior and
never produces `RateLimited`. Without this a custom transport could not report a
rate limit the built-in clients do report. The ureq *injected-agent* path is the
one built-in exception: `ureq::Error::StatusCode(code)` carries no headers, so it
falls back to `Unauthorized` (an injected agent built with
`http_status_as_error(false)` reaches the header-aware check).

AUTH-2-3. `Error::http_status()` returns the status for `RateLimited` as it does
for the other HTTP variants, and `Error::url()` returns its URL
(`src/errors.rs:267`, `:279`).

AUTH-2-4. The `Display` string names rate limiting, the reset time when known,
and the token remedy, rather than reading as an auth failure.

AUTH-2-5. Tests: classification from synthetic response headers (403 with
remaining 0 -> `RateLimited`; 403 without the headers -> `Unauthorized`; 429
with headers -> `RateLimited`), plus `http_status()` / `url()` accessor
coverage.

## AUTH-3: docs

AUTH-3-1. The README / lib.rs rate-limit section gains the shared-IP mechanism:
the 60/hour budget is per source IP, so on a NAT'd network it is pooled across
everyone behind that IP and can be exhausted by other people entirely, which is
why a lightly-used application still sees 403s there.

AUTH-3-2. It also documents `auth_token_from_env()` as the one-line remedy, and
`Error::RateLimited` as the variant to match for backoff.

## Non-goals

- No automatic env fallback (AUTH-1-3).
- No credential-helper, keychain, netrc, or `gh auth token` shell-out lookups.
- No automatic wait-and-retry on `RateLimited`. Retrying a rate-limited request
  only consumes more quota; backing off is the caller's policy decision, and
  `UpdateCheckGuard` (`ref-check-interval.md`) is the throttle the crate offers.

## Related

- `ref-github-backend.md`, `ref-common-config.md` (auth token threading and the
  host gate)
- `ref-errors.md` (variant inventory)
- `ref-check-interval.md` (reducing check frequency)

## Implemented

Landed 2026-08-27:

- `auth_token_from_env()` on the `Update` and `ReleaseList` builders of all four git backends,
  emitted by `impl_auth_token_from_env!` (`src/macros.rs`) and by the new
  `impl_common_builder_setters!(auth_env: [..])` form. Resolution is
  `backends::common::token_from_env` -> `first_env_token`; the write is `apply_env_token`.
- `Error::RateLimited` plus `classify_status` / `status_to_error_with_headers` in `src/errors.rs`,
  called by both built-in clients (sync + async reqwest, ureq), and the public
  `Error::http_status_error_with_headers` constructor.
- Docs: the crate-level "GitHub rate limits" section now covers the per-source-IP mechanism, the
  `auth_token_from_env()` remedy, and `Error::RateLimited` (`src/lib.rs`, mirrored into `README.md`).
