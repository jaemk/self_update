# Auth token from env, and rate-limit errors

Status: pending (decided 2026-07-26; not implemented)

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
`auth_token_from_env()` are last-setter-wins.

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
addition). `reset_at` is the parsed reset instant when the response carries one,
`retry_after` the `Retry-After` delay when present; both `Option`.

AUTH-2-2. A 403 (or 429) response is classified as `RateLimited` instead of
`Unauthorized` when it carries a zero remaining-quota header:
`x-ratelimit-remaining: 0` with `x-ratelimit-reset` (github, gitea, gitee), or
`RateLimit-Remaining: 0` (gitlab). Absent those headers the classification is
unchanged.

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
