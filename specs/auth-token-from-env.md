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

AUTH-1-1 (revised 2026-08-27). `auth_token_from_env()` is added to the backend
`UpdateBuilder` and `ReleaseListBuilder` types that take an `auth_token`, emitted by
`impl_auth_token_from_env!` (`src/macros.rs:484-555`). It reads the backend's conventional
env vars in order and uses the first that is present and non-empty after trimming
surrounding whitespace:

- github: `GH_TOKEN`, then `GITHUB_TOKEN` (`src/backends/github.rs:191` ReleaseListBuilder,
  `:379` UpdateBuilder). This order was **flipped from the original `GITHUB_TOKEN` then
  `GH_TOKEN`**: `gh help environment` documents "GH_TOKEN, GITHUB_TOKEN (in order of
  precedence)" (`github.rs:188-190`, `:376-378`), so the original order was the reverse of the
  CLI it claimed to match. Inside GitHub Actions `GITHUB_TOKEN` is auto-populated, so a
  deliberately-exported `GH_TOKEN` should win over it, not be silently shadowed by it.
- gitlab: `GITLAB_TOKEN` only (`src/backends/gitlab.rs:203` ReleaseListBuilder, `:388`
  UpdateBuilder). `CI_JOB_TOKEN` was **removed** from the list (it was originally the second
  fallback): it is exported in every GitLab CI job, but this crate's backend pins
  `Authorization: Bearer`, which is not GitLab's job-token mechanism (the `JOB-TOKEN` header
  or a `job_token` request parameter), and job tokens are project-scoped. Keeping it in the
  list meant the call was never the advertised no-op inside CI (AUTH-1-2) -- it silently
  turned a working anonymous fetch against a public project into a 401/403 sent with a token
  the backend cannot actually use correctly. See `gitlab.rs:209`, `:394` for the removal note.
- gitea: `GITEA_TOKEN` (`src/backends/gitea.rs:183`, `:378`).
- gitee: `GITEE_TOKEN` (`src/backends/gitee.rs:213`, `:403`).

AUTH-1-2. No variable set (or all empty) leaves `auth_token` unset: the request
goes out unauthenticated exactly as today, no error. This makes the call safe to
place unconditionally in an application that also runs outside CI or a corporate
network.

AUTH-1-3 (superseded 2026-08-27; see revision note below). Reading env is opt-in, never
automatic. A library that harvests credentials from the environment without being asked is
surprising, and the configured API base can be a self-hosted host, so an implicit read would
decide on its own to send a user's token somewhere. The explicit call keeps the decision with
the embedding application.

The rule for how `auth_token(..)` and `auth_token_from_env()` interact was originally
"last-setter-wins when the environment supplies a token; a lookup that finds nothing leaves
the existing value alone". A review of that reading replaced it with the rule below, which is
now the normative one:

**An explicit `auth_token(..)` call always wins, whatever the call order.** The environment is
purely a *fallback* that fills the token slot only when it is still empty:
`auth_token(t).auth_token_from_env()` and `auth_token_from_env().auth_token(t)` both end up
with `t`. A lookup that finds nothing leaves the slot exactly as it was, so the call can never
clear a token (unchanged from the original reading). Enforced by
`crate::backends::common::fill_env_token_if_unset(slot: &mut Option<String>, resolved:
Option<String>) -> bool` (`src/backends/common.rs:255-266`; renamed from the earlier
`apply_env_token`), called from the generated `auth_token_from_env()` setter
(`macros.rs:533-542`). The explicit `auth_token(..)` setter unconditionally overwrites the slot
and clears the env-sourced flag (AUTH-1-8) so a later `auth_token_from_env()` call cannot
re-fill it (`macros.rs:594-600`, per-backend equivalents e.g. `github.rs:177-183`).

Why the order-sensitive last-wins reading was replaced: it was surprising (two setters on the
same builder disagreeing about which one "wins" depending on call order is not how any other
pair of setters on these builders behaves), it let an ambient developer PAT silently displace
the credential the application itself provisioned (`auth_token(app_token).auth_token_from_env()`
would previously drop `app_token` for whatever the caller's shell happened to export), and on a
shared or CI host a hostile or merely misconfigured environment variable could displace a
token the application deliberately supplied -- a credential-confusion risk, not just an
ergonomics one.

AUTH-1-4. The env read happens in the setter (not at request time), so the
resolved value is visible in the builder's `Debug` output (redacted as
`<token>`; see AUTH-1-9) and the behavior does not depend on env changes made
later in the process.

AUTH-1-5. Tests: the env-var precedence is exercised through a pure helper
taking the candidate `(name, value)` pairs, so no test mutates process env
(which is racy under the parallel test harness). Cover first-wins, empty-skip,
whitespace-trim, and none-set.

AUTH-1-6 (added 2026-08-27). `has_auth_token() -> bool` is added alongside `auth_token_from_env()`
on every builder that has it (`macros.rs:544-553`, part of `impl_auth_token_from_env!`). It
reports whether a token is currently set on the builder, from either `auth_token(..)` or a
successful `auth_token_from_env()`, without exposing the value -- so an application can decide
"am I about to run authenticated?" (e.g. to pick a polling interval, or warn that a private repo
will be unreachable) without reimplementing the env-var list itself.

AUTH-1-7 (added 2026-08-27). Each builder carries a crate-internal `AUTH_TOKEN_ENV_VARS: &'static
[&'static str]` constant (`macros.rs:494`) holding the exact list from AUTH-1-1, in precedence
order. It is the literal list `auth_token_from_env()` reads (not a copy that could drift), and is
also the value backend tests assert against so a test failure means the real behavior changed, not
a documentation-only mismatch (e.g. `github.rs:784-789`, `gitlab.rs:781-784`, `gitea.rs:767-770`,
`gitee.rs:853-856`).

AUTH-1-8 (added 2026-08-27). Because the env var list is tied to the backend *type* (`GITHUB_TOKEN`
for github, etc.) while the token is sent to whatever host the application configured via
`api_base_url` / `host`, an application that exposes its update URL as configuration and runs in
CI could hand a `GITHUB_TOKEN` to an attacker-chosen host with no signal at all -- the request-time
host gate (`ref-common-config.md` "Auth scheme") cannot catch this, because the configured host
*is* `auth_base_host`. `warn_if_env_token_off_canonical_host(env_sourced: bool, auth_base_host:
Option<&str>, canonical_host: Option<&str>) -> bool` (`common.rs:308-329`) closes that gap: when
the current token is env-sourced (the `auth_token_from_env` flag, cleared by any explicit
`auth_token(..)`) and the resolved `auth_base_host` does not case-insensitively match the backend's
canonical host, it logs a `log::warn!` naming both hosts. Github's canonical host is
`api.github.com`, gitlab's `gitlab.com`, gitee's `gitee.com` (each a `CANONICAL_AUTH_HOST` const,
e.g. `github.rs:18`); called from `build()` on both builders (e.g. `github.rs:221-225` ReleaseList,
`:422-426` Update). **Gitea has no canonical host** (it is always self-hosted) and passes `None`,
so the call is made for symmetry but never warns (`gitea.rs:207-213`, `:418-424`). An explicitly-set
token is the application's own decision and is never warned about.

AUTH-1-9 (added 2026-08-27). Both config structs that can hold a raw token now carry a
hand-written `Debug` impl that redacts it to `"<token>"` (`None` still renders `None`):
`RequestConfig::fmt` (`common.rs:416-440`, unchanged in spirit) and, new, `CommonBuilderConfig::fmt`
(`common.rs:679-723`) -- `CommonBuilderConfig` holds its own separate `auth_token: Option<String>`
field (`common.rs:657`, distinct from `RequestConfig::auth_token`), so a `#[derive(Debug)]` on it
(the pre-fix state) would have printed a live credential from a plain `log::debug!("{builder:?}")`,
including one the application author never typed themselves (an ambient CI token picked up by
`auth_token_from_env()`). `CommonBuilderConfig::fmt` also renders the `auth_token_from_env` flag
(AUTH-1-3) verbatim (it is not sensitive), so a debug dump answers "is a token set, and did it come
from the environment?" without leaking the value.

## AUTH-2: distinguishable rate-limit error

AUTH-2-1. New variant `Error::RateLimited { status, url, reset_at, retry_after }`
(`Error` is `#[non_exhaustive]`, `src/errors.rs:21`, so this is a minor-version
addition; the variant itself is at `errors.rs:111-132`). `reset_at` is the parsed reset instant
(`Option<SystemTime>`) when the response carries one, `retry_after` the `Retry-After` delay
(`Option<Duration>`; only the delta-seconds form is parsed, the HTTP-date form
yields `None` rather than adding a date-parsing dependency). Both are capped at 24h; see
AUTH-2-7.

AUTH-2-2 (broadened 2026-08-27). `classify_status(status, url, RateLimitSignals)`
(`errors.rs:853-875`) classifies a response as `RateLimited` instead of falling through to
`status_to_error`:

- **429 is always `RateLimited`**, with or without any quota headers -- RFC 6585 defines the
  status as rate limiting, and it is what proxies, CDNs, and self-hosted gitea return with no
  quota headers at all. (The original reading required a zero remaining-quota header even on
  429, so a bare 429 misclassified as `HttpStatus`.)
- **403 is `RateLimited` when EITHER**: the remaining-quota header parses as `0`
  (`x-ratelimit-remaining: 0` on github/gitea/gitee, or gitlab's `RateLimit-Remaining: 0`), **or**
  a `Retry-After` header is present and parses (AUTH-2-7's clamp still applies). The second
  condition is new: GitHub's *secondary* rate limit (abuse-detection / high-frequency-request
  throttling, as opposed to the primary per-hour quota) answers with 403 + `Retry-After` while
  `x-ratelimit-remaining` is still nonzero, which the original remaining-only rule misclassified as
  `Unauthorized` -- a genuine credential failure and "back off, you're going too fast" produced the
  same variant.
- A bare 403 with **neither** signal stays `Unauthorized` (a genuine credential failure); this
  carve-out is unchanged.
- Every other status is untouched by the quota headers and falls through to `status_to_error`.

The remaining-quota and reset headers are read from `x-ratelimit-remaining` / `x-ratelimit-reset`
falling back to `ratelimit-remaining` / `ratelimit-reset` (`errors.rs:912-913`); `HeaderMap`
lookups are case-insensitive, so *that* fallback is only needed to bridge github/gitea/gitee's
`x-ratelimit-*` spelling and gitlab's differently-named `RateLimit-*` header, not to cover casing
within either spelling.

AUTH-2-3. `Error::http_status()` returns the status for `RateLimited` as it does
for the other HTTP variants, and `Error::url()` returns its URL
(`src/errors.rs:362-370`, `:374-382`).

AUTH-2-4 (exact string corrected 2026-08-27). The `Display` string (`errors.rs:563-579`) names
rate limiting, the reset time when known, and the token remedy, rather than reading as an auth
failure. Exact form: `"RateLimitedError: request to {url} was rate limited (HTTP {status})"`, then
when `rate_limit_delay()` (AUTH-2-8) resolves to `Some(wait)`, `"; quota resets in {n}s"`, then
always `"; set an auth token to raise the limit, or check less often"`. The wait clause and the
`http_status()`/`url()`/`Display` precedence all route through the same `rate_limit_delay()`
accessor rather than re-deriving the `Retry-After`-then-`reset_at` choice inline, so they cannot
disagree.

AUTH-2-6 (added 2026-08-27; gap closed 2026-08-27, see revision note below). The same
classification is reachable from a custom `HttpClient` via the public
`Error::http_status_error_with_headers(status, url, &HeaderMap)` (`errors.rs:493-499`); the
header-less `Error::http_status_error` (`errors.rs:475-477`) keeps its behavior and never
produces `RateLimited`, having no headers to read.

The ureq *injected-agent* path was originally the one built-in exception: an injected
`ureq::Agent` keeps ureq's own default `http_status_as_error(true)`, so a non-2xx response fired
`ureq::Error::StatusCode(code)` from `call()?`, which carries no headers, so that path fell back to
`Unauthorized` even for a spent-quota 403. **This gap is now closed.** `UreqClient::get`
(`http_client/ureq.rs:115-178`) applies a **per-request** override on an injected agent's request
builder: `req.config().http_status_as_error(false).build()` (`ureq.rs:142-149`), which does not
touch the injected agent's own persistent timeout/TLS/proxy config, only this one request's status
handling. With that override, an injected agent's non-2xx response reaches the same header-aware
`status_to_error_with_headers` check at the bottom of `get` (`ureq.rs:168-174`) as the
default (per-call) agent, which is built with the same option at agent-construction time
(`build_call_agent`, `ureq.rs:75`). All three client lanes (default ureq agent, injected ureq
agent, reqwest) now classify a given status + headers identically.

The `Err(ureq::Error::StatusCode(code)) if is_injected` arm (`ureq.rs:157-164`) is retained only as
a **defensive fallback** for a future ureq release that stops honoring the per-request override; it
maps through the header-less `status_to_error`, where a 429 is still `RateLimited` (carrying no
wait) but a 403 stays `Unauthorized`, since only a header distinguishes a spent quota from a
credential failure -- and it is not expected to fire in normal operation anyway.

AUTH-2-5 (expanded 2026-08-27). Tests, all in `errors.rs` `mod tests` unless noted: classification
from synthetic response headers, covering both the original and the broadened rule --
`classify_status_maps_a_spent_quota_403_to_rate_limited`,
`classify_status_keeps_a_plain_403_unauthorized`,
`classify_status_keeps_403_unauthorized_when_quota_remains`,
`classify_status_maps_a_spent_quota_429_to_rate_limited`,
`classify_status_maps_a_bare_429_to_rate_limited`,
`classify_status_maps_a_429_with_only_retry_after_to_rate_limited`,
`classify_status_maps_a_403_with_retry_after_to_rate_limited`,
`classify_status_maps_a_403_with_spent_quota_and_no_retry_after_to_rate_limited`,
`classify_status_ignores_quota_headers_on_a_404`,
`classify_status_ignores_quota_headers_on_other_statuses`,
`classify_status_tolerates_unparseable_reset_and_retry_after`,
`classify_status_redacts_the_rate_limited_url`,
`status_to_error_with_headers_reads_both_header_spellings`; accessor coverage
`http_status_and_url_helpers_cover_rate_limited`; Display coverage
`rate_limited_display_names_the_limit_and_the_remedy`,
`rate_limited_display_omits_the_wait_when_unknown_or_elapsed`,
`rate_limited_display_separates_its_clauses_consistently`; the ureq injected-agent gap-closure is
covered separately in `http_client/ureq.rs` (`injected_agent_sees_rate_limit_headers`,
`injected_agent_with_status_error_disabled_sees_rate_limit_headers`,
`injected_agent_no_status_error_falls_through_to_is_success_check`,
`default_agent_path_maps_statuses_identically_to_injected`). See AUTH-2-7/2-8 for the clamp and
`rate_limit_delay()` test lists.

AUTH-2-7 (added 2026-08-27). Both server-supplied wait values are clamped to a 24h ceiling:
`MAX_RATE_LIMIT_WAIT` (`errors.rs:840`) is `Duration::from_secs(24 * 60 * 60)`. `parse_reset_epoch`
(`errors.rs:880-887`, feeding `reset_at`) discards a parsed instant more than 24h in the future,
yielding `None` instead; an instant already in the past is kept as-is (it renders no wait via
AUTH-2-8). `parse_retry_after` (`errors.rs:892-895`, feeding `retry_after`) discards a delta-seconds
value above 24h, also yielding `None`. Both values are attacker-controlled -- anything able to shape
the response chooses them -- and the documented use is "sleep this long before retrying", so an
unbounded value is a way to park a caller (and with it its update/security-patch channel)
indefinitely. A value past the ceiling resolves to `None` rather than being clamped down to the
ceiling, so a caller falls back to its own policy instead of trusting an implausible number; 24h is
comfortably above any real forge window (GitHub's is one hour) while staying well short of
"indefinitely". Tests: `parse_retry_after_keeps_a_normal_delay`,
`parse_retry_after_clamps_at_twenty_four_hours`, `parse_retry_after_rejects_the_u64_max_delay`,
`classify_status_ignores_an_over_ceiling_retry_after_on_a_403` (`errors.rs`).

AUTH-2-8 (added 2026-08-27). `Error::rate_limit_delay(&self) -> Option<std::time::Duration>`
(`errors.rs:406-417`) is a public accessor giving "how long to wait before retrying", `None` for
every non-`RateLimited` variant. It prefers `retry_after` and falls back to `reset_at` minus the
current time (`None` when that subtraction would be negative, i.e. the window already elapsed):
neither field alone is the right answer, since GitHub's *primary* rate limit sends only
`x-ratelimit-reset` (so reading `retry_after` alone would sleep zero and burn more quota) while
`reset_at.unwrap().duration_since(now).unwrap()` would panic once the window has passed. `Display`
(AUTH-2-4) calls this same accessor for its optional wait clause, so the precedence exists in
exactly one place. Tests: `rate_limit_delay_prefers_retry_after_over_reset_at`,
`rate_limit_delay_uses_retry_after_alone`, `rate_limit_delay_derives_a_wait_from_a_future_reset_at`,
`rate_limit_delay_is_none_for_an_elapsed_reset_at`, `rate_limit_delay_is_none_when_nothing_is_known`,
`rate_limit_delay_is_none_for_a_non_rate_limited_variant` (`errors.rs`).

AUTH-2-9 (added 2026-08-27). The "no automatic wait-and-retry on `RateLimited`" policy (see
Non-goals) is now structurally enforced, not just a documented intention: `is_rate_limited(err:
&Error) -> bool` (`backends/mod.rs:364-366`) matches `Error::RateLimited { .. }`, and both `retry`
(`backends/mod.rs:376-396`) and its async sibling `retry_async` (`backends/mod.rs:401-431`) check it
alongside the existing `attempts >= retries` budget check and return the error immediately when
either is true -- so a `RateLimited` response is returned on the *first* attempt and never consumes
retry budget, regardless of how `retries` is configured. Every other error still consumes the
budget as before. Tests: `retry_does_not_retry_a_rate_limited_error`,
`retry_still_consumes_the_budget_for_a_non_rate_limited_error`,
`retry_async_does_not_retry_a_rate_limited_error`,
`retry_async_still_consumes_the_budget_for_a_non_rate_limited_error` (`backends/mod.rs`).

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
  `UpdateCheckGuard` (`ref-check-interval.md`) is the throttle the crate offers. This is now
  structurally enforced by `retry`/`retry_async`, not just documented intent; see AUTH-2-9.

## Related

- `ref-github-backend.md`, `ref-gitlab-backend.md`, `ref-gitea-backend.md`, `ref-gitee-backend.md`,
  `ref-common-config.md` (auth token threading, the host gate, and each backend's env-var list)
- `ref-errors.md` (variant inventory, classification, the 24h clamp)
- `ref-http-client.md` (the ureq injected-agent gap closure, the retry short-circuit on
  `RateLimited`)
- `ref-check-interval.md` (reducing check frequency)

## Implemented

Landed 2026-08-27; review-driven fixes landed the same day (superseding some of the original
choices, per the revision notes on AUTH-1-1/1-3/2-2/2-6 above):

- `auth_token_from_env()` on the `Update` and `ReleaseList` builders of all four git backends,
  emitted by `impl_auth_token_from_env!` (`src/macros.rs:484-555`) and by the
  `impl_common_builder_setters!(auth_env: [..])` form. Resolution is
  `backends::common::token_from_env` -> `first_env_token`; the write is `fill_env_token_if_unset`
  (renamed from `apply_env_token`). Env var lists: github `["GH_TOKEN", "GITHUB_TOKEN"]` (flipped
  order), gitlab `["GITLAB_TOKEN"]` (`CI_JOB_TOKEN` removed), gitea `["GITEA_TOKEN"]`, gitee
  `["GITEE_TOKEN"]`, each also a crate-internal `AUTH_TOKEN_ENV_VARS` const (AUTH-1-7). An
  explicit `auth_token(..)` always wins over the environment regardless of call order (AUTH-1-3).
  `has_auth_token()` (AUTH-1-6) queries whether a token is set from either source.
- `warn_if_env_token_off_canonical_host` (AUTH-1-8) flags an env-sourced token bound to a
  non-canonical host; a hand-written redacting `Debug` on `CommonBuilderConfig` (AUTH-1-9)
  complements the existing one on `RequestConfig`.
- `Error::RateLimited` plus `classify_status` / `status_to_error_with_headers` in `src/errors.rs`,
  called by both built-in clients (sync + async reqwest, ureq), and the public
  `Error::http_status_error_with_headers` constructor. Classification broadened (AUTH-2-2): 429
  always classifies, and a 403 also classifies on a usable `Retry-After` (GitHub's secondary rate
  limit), not just a spent quota. Both `reset_at` and `retry_after` are capped at 24h
  (`MAX_RATE_LIMIT_WAIT`, AUTH-2-7); `Error::rate_limit_delay()` (AUTH-2-8) is the one place the
  `Retry-After`-then-`reset_at` precedence is computed, reused by `Display`. The ureq
  injected-agent classification gap is closed via a per-request `http_status_as_error(false)`
  override (AUTH-2-6). `retry` / `retry_async` (`backends/mod.rs`) short-circuit on
  `Error::RateLimited`, never spending retry budget on it (AUTH-2-9).
- Docs: the crate-level "GitHub rate limits" section now covers the per-source-IP mechanism, the
  `auth_token_from_env()` remedy, and `Error::RateLimited` (`src/lib.rs`, mirrored into `README.md`).
