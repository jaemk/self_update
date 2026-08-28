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
  supply a token (`src/backends/github.rs:auth_token`). There is no env-var path, so
  every consumer writes the same `std::env::var("GITHUB_TOKEN")` plumbing,
  including the skip-when-empty case.
- The token is already forwarded safely: `apply_auth` attaches it only to a URL
  whose host matches the configured API base or an `allow_auth_host` entry, and
  only over https (`src/backends/common.rs:apply_auth` `apply_auth`,
  `src/backends/common.rs:auth_allowed_for` `auth_allowed_for`), with a per-backend scheme (github/gitea `Token`, gitlab
  `Bearer`; the `AuthScheme` enum is in `common.rs`). Nothing about the
  env source changes that gate.
- A rate-limited response surfaces as `Error::Unauthorized { status: 403, url }`
  (`src/errors.rs:Unauthorized`), the same variant as a bad token, so a caller cannot tell
  "wait for the window to reset, or set a token" from "these credentials are
  wrong". README:360 documents the limits and tells the reader to recognize the
  rate-limit case by its symptom.

## AUTH-1: token from the environment

AUTH-1-1 (revised 2026-08-27). `auth_token_from_env()` is added to the backend
`UpdateBuilder` and `ReleaseListBuilder` types that take an `auth_token`, emitted by
`impl_auth_token_from_env!` (`src/macros.rs:impl_auth_token_from_env`). It reads the backend's conventional
env vars in order and uses the first that is present and non-empty after trimming
surrounding whitespace:

- github: `GH_TOKEN`, then `GITHUB_TOKEN` (`src/backends/github.rs:ReleaseListBuilder` ReleaseListBuilder,
  `src/backends/github.rs:UpdateBuilder` UpdateBuilder). This order was **flipped from the original `GITHUB_TOKEN` then
  `GH_TOKEN`**: `gh help environment` documents "GH_TOKEN, GITHUB_TOKEN (in order of
  precedence)" (`src/backends/github.rs:ReleaseListBuilder`, `src/backends/github.rs:UpdateBuilder`), so the original order was the reverse of the
  CLI it claimed to match. Inside GitHub Actions `GITHUB_TOKEN` is auto-populated, so a
  deliberately-exported `GH_TOKEN` should win over it, not be silently shadowed by it.
- gitlab: `GITLAB_TOKEN` only (`src/backends/gitlab.rs:ReleaseListBuilder` ReleaseListBuilder, `src/backends/gitlab.rs:UpdateBuilder`
  UpdateBuilder). `CI_JOB_TOKEN` was **removed** from the list (it was originally the second
  fallback): it is exported in every GitLab CI job, but this crate's backend pins
  `Authorization: Bearer`, which is not GitLab's job-token mechanism (the `JOB-TOKEN` header
  or a `job_token` request parameter), and job tokens are project-scoped. Keeping it in the
  list meant the call was never the advertised no-op inside CI (AUTH-1-2) -- it silently
  turned a working anonymous fetch against a public project into a 401/403 sent with a token
  the backend cannot actually use correctly. See `src/backends/gitlab.rs:ReleaseListBuilder`, `src/backends/gitlab.rs:UpdateBuilder` for the removal note.
- gitea: `GITEA_TOKEN` (`src/backends/gitea.rs:ReleaseListBuilder`, `src/backends/gitea.rs:UpdateBuilder`).
- gitee: `GITEE_TOKEN` (`src/backends/gitee.rs:ReleaseListBuilder`, `src/backends/gitee.rs:UpdateBuilder`).

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

**An explicit `auth_token(..)` call with a non-blank value always wins, whatever the call order.**
The environment is purely a *fallback* that fills the token slot only when it is still blank:
`auth_token(t).auth_token_from_env()` and `auth_token_from_env().auth_token(t)` both end up
with `t`, for a non-blank `t`. A lookup that finds nothing leaves the slot exactly as it was, so
the call can never clear a token (unchanged from the original reading). **A blank explicit token
is the one documented exception to order-independence** -- see the blank-token rule below for the
asymmetry. Enforced by
`crate::backends::common::fill_env_token_if_unset_with(slot: &mut Option<String>, resolve: impl
FnOnce() -> Option<String>) -> bool` (`src/backends/common.rs:fill_env_token_if_unset_with`), which checks
`is_blank_token(slot)` (`src/backends/common.rs:is_blank_token`) before calling `resolve` at all -- so the
environment is not even read, let alone logged, for a call whose result would be discarded. The
generated `auth_token_from_env()` setter (`src/macros.rs:auth_token_from_env`) calls it with a closure over
`token_from_env`. `fill_env_token_if_unset(slot: &mut Option<String>, resolved: Option<String>)
-> bool` (`src/backends/common.rs:fill_env_token_if_unset`; renamed from the earlier `apply_env_token`) still exists as a thin
wrapper over an already-resolved value, kept only so its original unit tests stand -- it is not
what the generated setter calls. The explicit `auth_token(..)` setter unconditionally overwrites
the slot and clears the env-sourced flag (AUTH-1-8) so a later `auth_token_from_env()` call
cannot re-fill it, via the shared `set_explicit_auth_token(slot: &mut Option<String>,
env_sourced: &mut bool, value: impl Into<String>)` (`src/backends/common.rs:set_explicit_auth_token`), called from the
macro-generated setter (`src/macros.rs:auth_token`) and each hand-written `ReleaseListBuilder::auth_token`
(e.g. `src/backends/github.rs:auth_token`).

**Blank-token rule (added 2026-08-27).** A blank explicit token (`auth_token("")` or
`auth_token("   ")`, e.g. from `auth_token(cfg.token.unwrap_or_default())` applied to a missing
config value) is treated the same as an unset one everywhere a token is consulted:
`is_blank_token` (`src/backends/common.rs:is_blank_token`) backs `fill_env_token_if_unset_with` above (so a blank
explicit token does not block the environment fallback), `RequestConfig::apply_auth`
(`src/backends/common.rs:apply_auth`, see `ref-common-config.md`, so a blank token never produces a literal
`Authorization: token ` header), and `has_auth_token()` (AUTH-1-6). This is distinct from
`first_env_token`'s trimming: an explicit `auth_token(..)` value is never trimmed, so a token
merely *surrounded* by whitespace still surfaces as `Error::InvalidAuthToken` at request time
rather than being silently repaired.

**Documented asymmetry: a blank token is not order-independent.** `set_explicit_auth_token`
overwrites the slot unconditionally (`src/backends/common.rs:set_explicit_auth_token`), regardless of what value it holds, so
call order decides the outcome for a blank explicit token specifically:

- `auth_token("").auth_token_from_env()` picks up the env token: `auth_token("")` leaves the slot
  blank, and `fill_env_token_if_unset_with` fills a blank slot.
- `auth_token_from_env().auth_token("")` discards it: `auth_token_from_env()` fills the slot from
  the environment, then `auth_token("")` unconditionally overwrites that back to blank.

So "an explicit `auth_token(..)` call always wins, whatever the call order" (above) holds only for
a non-blank value; a blank one behaves like last-call-wins against `auth_token_from_env()`. This
was a deliberate tradeoff, not an oversight: fixing it would mean `auth_token(..)` sometimes does
not overwrite the slot, which is a bigger behavior change than the asymmetry itself.

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
on every builder that has it (`src/macros.rs:has_auth_token`, part of `impl_auth_token_from_env!`). It
reports whether a token is currently set on the builder, from either `auth_token(..)` or a
successful `auth_token_from_env()`, without exposing the value -- so an application can decide
"am I about to run authenticated?" (e.g. to pick a polling interval, or warn that a private repo
will be unreachable) without reimplementing the env-var list itself. It answers `false` for a
blank slot too (the blank-token rule, AUTH-1-3), not just an unset one.

AUTH-1-7 (added 2026-08-27). Each builder carries a crate-internal `AUTH_TOKEN_ENV_VARS: &'static
[&'static str]` constant (`src/macros.rs:AUTH_TOKEN_ENV_VARS`) holding the exact list from AUTH-1-1, in precedence
order. It is the literal list `auth_token_from_env()` reads (not a copy that could drift), and is
also the value backend tests assert against so a test failure means the real behavior changed, not
a documentation-only mismatch (`src/backends/github.rs:auth_token_env_vars_are_gh_token_then_github_token`,
`src/backends/gitlab.rs:auth_token_env_vars_are_gitlab_token_only`, `src/backends/gitea.rs:auth_token_env_vars_are_gitea_token_only`,
`src/backends/gitee.rs:auth_token_env_vars_are_gitee_token_only`).

AUTH-1-8 (revised 2026-08-27; gitea's outcome changed, see revision note below). Because the env
var list is tied to the backend *type* (`GITHUB_TOKEN` for github, etc.) while the token is sent
to whatever host the application configured via `api_base_url` / `host`, an application that
exposes its update URL as configuration and runs in CI could hand a `GITHUB_TOKEN` to an
attacker-chosen host with no signal at all -- the request-time host gate (`ref-common-config.md`
"Auth scheme") cannot catch this, because the configured host *is* `auth_base_host`.
`env_token_host_decision(env_sourced: bool, auth_base_host: Option<&str>, auth_hosts: &[String],
canonical_host: Option<&str>) -> EnvTokenDecision` (`src/backends/common.rs:env_token_host_decision`) closes that gap. It first
checks `host_is_acknowledged(host, auth_hosts, canonical_host)` (`src/backends/common.rs:host_is_acknowledged`) -- the host
is the backend's canonical one, **or** one the application already passed to
`allow_auth_host(..)` -- and returns `EnvTokenDecision::Sent` (no warning) when the token is not
env-sourced, has no host to check, or the host is acknowledged. Otherwise it branches on whether
the backend has a canonical host at all (`src/backends/common.rs:EnvTokenDecision` for the three-state
`EnvTokenDecision` enum):

- **github/gitlab/gitee** (a `CANONICAL_AUTH_HOST` const each, e.g. `src/backends/github.rs:CANONICAL_AUTH_HOST`, `src/backends/gitlab.rs:CANONICAL_AUTH_HOST`,
  `src/backends/gitee.rs:CANONICAL_AUTH_HOST`): logs a `log::warn!` naming both hosts and returns
  `EnvTokenDecision::WarnedAndSent` -- the token is still attached. This is today's original
  behavior, decided to stay unchanged; the only new escape hatch is `allow_auth_host(..)` above,
  which previously had no effect on the warning at all (`src/backends/github.rs:ReleaseListBuilder` `ReleaseListBuilder`,
  `src/backends/github.rs:Update` `Update`; gitlab.rs equivalents `src/backends/gitlab.rs:ReleaseListBuilder`/`src/backends/gitlab.rs:Update`; gitee.rs equivalents
  `src/backends/gitee.rs:ReleaseListBuilder`/`src/backends/gitee.rs:Update`).
- **gitea** (no canonical host -- it is always self-hosted, so passes `None`): logs a `log::warn!`
  naming the host and both remedies (`auth_token(..)` or `allow_auth_host(the_same_host)`) and
  returns `EnvTokenDecision::Withheld`. The caller (gitea's `build()` /
  `build_update()`, `src/backends/gitea.rs:build`/`src/backends/gitea.rs:build_update`) clears `request.auth_token` on that outcome, so
  the request goes out anonymous; `build()` still returns `Ok`, since a hard failure here would be
  a worse outcome than the anonymous request it replaces. **This is a change**: gitea previously
  passed `None` for symmetry and the call never warned or acted, silently binding an ambient
  `GITEA_TOKEN` to whatever host the application configured -- see the revision note below and
  `ref-gitea-backend.md`.

An explicitly-set token (`auth_token(..)`) is never flagged either way, since it is not
env-sourced (`EnvTokenDecision::Sent`).

*Revision note.* The original reading gave gitea *no* signal at all: the helper (then named
`warn_if_env_token_off_canonical_host`, since replaced by `env_token_host_decision` above)
early-returned on a `None` canonical host, so an ambient `GITEA_TOKEN` was bound to whatever host
the application was pointed at with nothing logged and nothing withheld. Review flagged this as
strictly worse than the warn-and-send behavior the other three backends get, since gitea's whole
premise is a self-hosted, caller-chosen host -- there is no "the token probably belongs here"
default to fall back to. The user decided: github/gitlab/gitee keep warn-and-send (a behavior
change there would surprise existing callers for no gain), but a backend with no canonical host
withholds instead, since silence is the wrong default when there is nothing to compare the host
against.

AUTH-1-9 (added 2026-08-27). Both config structs that can hold a raw token now carry a
hand-written `Debug` impl that redacts it to `"<token>"` (`None` still renders `None`):
`RequestConfig::fmt` (`src/backends/common.rs:RequestConfig::fmt`, unchanged in spirit) and, new, `CommonBuilderConfig::fmt`
(`src/backends/common.rs:CommonBuilderConfig::fmt`) -- `CommonBuilderConfig` holds its own separate `auth_token: Option<String>`
field (`src/backends/common.rs:CommonBuilderConfig::auth_token`, distinct from `RequestConfig::auth_token`), so a `#[derive(Debug)]` on it
(the pre-fix state) would have printed a live credential from a plain `log::debug!("{builder:?}")`,
including one the application author never typed themselves (an ambient CI token picked up by
`auth_token_from_env()`). `CommonBuilderConfig::fmt` also renders the `auth_token_from_env` flag
(AUTH-1-3) verbatim (it is not sensitive), so a debug dump answers "is a token set, and did it come
from the environment?" without leaking the value.

## AUTH-2: distinguishable rate-limit error

AUTH-2-1. New variant `Error::RateLimited { status, url, reset_at, retry_after }`
(`Error` is `#[non_exhaustive]`, `src/errors.rs:Error`, so this is a minor-version
addition; the variant itself is at `src/errors.rs:RateLimited`). `reset_at` is the parsed reset instant
(`Option<SystemTime>`) when the response carries one, `retry_after` the `Retry-After` delay
(`Option<Duration>`; only the delta-seconds form is parsed, the HTTP-date form
yields `None` rather than adding a date-parsing dependency). Both are capped at 24h; see
AUTH-2-7.

AUTH-2-2 (broadened 2026-08-27; zero-`Retry-After` floor added 2026-08-27, see below).
`classify_status(status, url, RateLimitSignals)`
(`src/errors.rs:classify_status`) classifies a response as `RateLimited` instead of falling through to
`status_to_error`:

- **429 is always `RateLimited`**, with or without any quota headers -- RFC 6585 defines the
  status as rate limiting, and it is what proxies, CDNs, and self-hosted gitea return with no
  quota headers at all. (The original reading required a zero remaining-quota header even on
  429, so a bare 429 misclassified as `HttpStatus`.)
- **403 is `RateLimited` when EITHER**: the remaining-quota header parses as `0`
  (`x-ratelimit-remaining: 0` on github/gitea/gitee, or gitlab's `RateLimit-Remaining: 0`), **or**
  a `Retry-After` header is present and parses to a nonzero delay (AUTH-2-7's clamp still
  applies). The second condition is new: GitHub's *secondary* rate limit (abuse-detection /
  high-frequency-request throttling, as opposed to the primary per-hour quota) answers with 403 +
  `Retry-After` while `x-ratelimit-remaining` is still nonzero, which the original remaining-only
  rule misclassified as `Unauthorized` -- a genuine credential failure and "back off, you're going
  too fast" produced the same variant. A `Retry-After: 0` does **not** satisfy this branch (see the
  zero-`Retry-After` floor below): a bare 403 whose only rate-limit header is a literal
  `Retry-After: 0` stays `Unauthorized` rather than becoming a `RateLimited` with a zero-second
  wait.
- A bare 403 with **neither** signal stays `Unauthorized` (a genuine credential failure); this
  carve-out is unchanged.
- Every other status is untouched by the quota headers and falls through to `status_to_error`.

The remaining-quota and reset headers are read from `x-ratelimit-remaining` / `x-ratelimit-reset`
falling back to `ratelimit-remaining` / `ratelimit-reset` (`src/errors.rs:status_to_error_with_headers`); `HeaderMap`
lookups are case-insensitive, so *that* fallback is only needed to bridge github/gitea/gitee's
`x-ratelimit-*` spelling and gitlab's differently-named `RateLimit-*` header, not to cover casing
within either spelling.

**Zero-`Retry-After` floor (added 2026-08-27).** `parse_retry_after` (`src/errors.rs:parse_retry_after`) treats a
literal `Retry-After: 0` the same as an absent or unparseable header: it returns `None` rather than
`Some(Duration::ZERO)`. Without this floor, `classify_status`'s 403 branch (which keys on
`retry_after.is_some()`) would promote a bare authorization failure to `RateLimited` carrying a
zero-second wait -- and a caller following this crate's own documented sleep-then-continue pattern
would spin in a tight loop against the server. A 429 is unaffected: it classifies as `RateLimited`
on the status code alone, whatever `Retry-After` says.

AUTH-2-3. `Error::http_status()` returns the status for `RateLimited` as it does
for the other HTTP variants, and `Error::url()` returns its URL
(`src/errors.rs:Error::http_status`, `src/errors.rs:Error::url`).

AUTH-2-4 (exact string corrected 2026-08-27). The `Display` string (`src/errors.rs:Error::fmt`) names
rate limiting, the reset time when known, and the token remedy, rather than reading as an auth
failure. Exact form: `"RateLimitedError: request to {url} was rate limited (HTTP {status})"`, then
a wait clause: `"; retry in {n}s"` when `retry_after` is `Some` (a requested back-off, not
necessarily proof the quota is spent), else `"; quota resets in {n}s"` when `rate_limit_delay()`
(AUTH-2-8) derives a still-future wait from `reset_at` (omitted when neither yields a wait), then
always `"; set an auth token to raise the limit, or check less often"`. All clauses are joined with
`"; "`. The wait clause and the `http_status()`/`url()`/`Display` precedence all route through the
same `rate_limit_delay()` accessor rather than re-deriving the `Retry-After`-then-`reset_at` choice
inline, so they cannot disagree.

AUTH-2-6 (added 2026-08-27; gap closed 2026-08-27, see revision note below). The same
classification is reachable from a custom `HttpClient` via the public
`Error::http_status_error_with_headers(status, url, &HeaderMap)` (`src/errors.rs:Error::http_status_error_with_headers`); the
header-less `Error::http_status_error` (`src/errors.rs:Error::http_status_error`) keeps its behavior and never
produces `RateLimited`, having no headers to read.

The ureq *injected-agent* path was originally the one built-in exception: an injected
`ureq::Agent` keeps ureq's own default `http_status_as_error(true)`, so a non-2xx response fired
`ureq::Error::StatusCode(code)` from `call()?`, which carries no headers, so that path fell back to
`Unauthorized` even for a spent-quota 403. **This gap is now closed.** `UreqClient::get`
(`src/http_client/ureq.rs:UreqClient::get`) applies a **per-request** override on an injected agent's request
builder, when the agent's own config has not already disabled ureq's status-error
(`needs_status_override`, `src/http_client/ureq.rs:needs_status_override`): `req.config().http_status_as_error(false).build()`
(`src/http_client/ureq.rs:UreqClient::get`), which does not touch the injected agent's own persistent timeout/TLS/proxy config,
only this one request's status handling. With that override, an injected agent's non-2xx response
reaches the same header-aware `status_to_error_with_headers` check at the bottom of `get`
(`src/http_client/ureq.rs:UreqClient::get`) as the default (per-call) agent, which is built with the same option at
agent-construction time (`build_call_agent`, `src/http_client/ureq.rs:build_call_agent`). All three client lanes (default ureq
agent, injected ureq agent, reqwest) now classify a given status + headers identically. See
`ref-http-client.md` for the full mechanism, including the TLS-config-cache cost the conditional
skip avoids.

The `Err(ureq::Error::StatusCode(code)) if is_injected` arm (`src/http_client/ureq.rs:UreqClient::get`) is retained only as
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
`MAX_RATE_LIMIT_WAIT` (`src/errors.rs:MAX_RATE_LIMIT_WAIT`) is `Duration::from_secs(24 * 60 * 60)`. `parse_reset_epoch`
(`src/errors.rs:parse_reset_epoch`, feeding `reset_at`) discards a parsed instant more than 24h in the future,
yielding `None` instead; an instant already in the past is kept as-is (it renders no wait via
AUTH-2-8). `parse_retry_after` (`src/errors.rs:parse_retry_after`, feeding `retry_after`) discards a delta-seconds
value above 24h, also yielding `None`. Both values are attacker-controlled -- anything able to shape
the response chooses them -- and the documented use is "sleep this long before retrying", so an
unbounded value is a way to park a caller (and with it its update/security-patch channel)
indefinitely. A value past the ceiling resolves to `None` rather than being clamped down to the
ceiling, so a caller falls back to its own policy instead of trusting an implausible number; 24h is
comfortably above any real forge window (GitHub's is one hour) while staying well short of
"indefinitely". Tests: `parse_retry_after_keeps_a_normal_delay`,
`parse_retry_after_clamps_at_twenty_four_hours`, `parse_retry_after_rejects_the_u64_max_delay`,
`classify_status_ignores_an_over_ceiling_retry_after_on_a_403` (`errors.rs`). The zero-`Retry-After`
floor (see AUTH-2-2) is a distinct rule tested separately:
`parse_retry_after_treats_a_zero_delay_as_no_signal`,
`classify_status_keeps_a_403_with_zero_retry_after_unauthorized`,
`classify_status_still_rate_limits_a_spent_quota_403_with_zero_retry_after`,
`classify_status_keeps_a_429_with_zero_retry_after_rate_limited` (`errors.rs`).

AUTH-2-8 (added 2026-08-27). `Error::rate_limit_delay(&self) -> Option<std::time::Duration>`
(`src/errors.rs:Error::rate_limit_delay`) is a public accessor giving "how long to wait before retrying", `None` for
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
&Error) -> bool` (`src/backends/mod.rs:is_rate_limited`) matches `Error::RateLimited { .. }`, and both `retry`
(`src/backends/mod.rs:retry`) and its async sibling `retry_async` (`src/backends/mod.rs:retry_async`) check it
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
  emitted by `impl_auth_token_from_env!` (`src/macros.rs:impl_auth_token_from_env`) and by the
  `impl_common_builder_setters!(auth_env: [..])` form. Resolution is
  `backends::common::token_from_env` -> `first_env_token`; the write goes through
  `fill_env_token_if_unset_with` (a lazy closure form so the environment is only read, and its
  diagnostics only logged, when the slot is actually blank -- `fill_env_token_if_unset` is kept as
  a thin wrapper over an already-resolved value, renamed from the earlier `apply_env_token`). Env
  var lists: github `["GH_TOKEN", "GITHUB_TOKEN"]` (flipped order), gitlab `["GITLAB_TOKEN"]`
  (`CI_JOB_TOKEN` removed), gitea `["GITEA_TOKEN"]`, gitee `["GITEE_TOKEN"]`, each also a
  crate-internal `AUTH_TOKEN_ENV_VARS` const (AUTH-1-7). An explicit `auth_token(..)` always wins
  over the environment regardless of call order (AUTH-1-3), via the shared
  `set_explicit_auth_token` helper. A blank explicit token (empty or all-whitespace) is treated as
  unset everywhere a token is consulted (the blank-token rule, AUTH-1-3). `has_auth_token()`
  (AUTH-1-6) queries whether a non-blank token is set from either source.
- `env_token_host_decision` (AUTH-1-8; replaces the earlier `warn_if_env_token_off_canonical_host`)
  flags an env-sourced token bound to an unacknowledged host: github/gitlab/gitee warn and still
  send it, gitea (no canonical host) withholds it instead. An `allow_auth_host(..)` entry
  acknowledges a host for either outcome, silencing the warning or letting gitea send. A
  hand-written redacting `Debug` on `CommonBuilderConfig` (AUTH-1-9) complements the existing one
  on `RequestConfig`.
- `Error::RateLimited` plus `classify_status` / `status_to_error_with_headers` in `src/errors.rs`,
  called by both built-in clients (sync + async reqwest, ureq), and the public
  `Error::http_status_error_with_headers` constructor. Classification broadened (AUTH-2-2): 429
  always classifies, and a 403 also classifies on a usable `Retry-After` (GitHub's secondary rate
  limit), not just a spent quota -- except a literal `Retry-After: 0`, which is floored to no
  signal so a bare 403 carrying only that header stays `Unauthorized`. Both `reset_at` and
  `retry_after` are capped at 24h
  (`MAX_RATE_LIMIT_WAIT`, AUTH-2-7); `Error::rate_limit_delay()` (AUTH-2-8) is the one place the
  `Retry-After`-then-`reset_at` precedence is computed, reused by `Display`. The ureq
  injected-agent classification gap is closed via a per-request `http_status_as_error(false)`
  override (AUTH-2-6). `retry` / `retry_async` (`backends/mod.rs`) short-circuit on
  `Error::RateLimited`, never spending retry budget on it (AUTH-2-9).
- Docs: the crate-level "GitHub rate limits" section now covers the per-source-IP mechanism, the
  `auth_token_from_env()` remedy, and `Error::RateLimited` (`src/lib.rs`, mirrored into `README.md`).
