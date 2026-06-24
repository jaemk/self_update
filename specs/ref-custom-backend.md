# Custom backends (reference)

Status: implemented

## Scope

The custom backend lets a downstream crate update from a host the built-in backends
(`github`, `gitlab`, `gitea`, `s3`) do not cover. The downstream crate implements a release
*source* trait that says where releases come from; the crate owns the rest of the update
flow. This spec documents the two source traits (`ReleaseSource`, `AsyncReleaseSource`), the
`backends::custom` adapter types (`Update`, `AsyncUpdate`, `Blocking`), the `Send` contract on
the async futures, and how a source plugs into the shared pipeline.

Source files: `src/update.rs` (trait definitions), `src/backends/custom.rs` (adapter types),
`examples/custom.rs` (usage).

## Behavior

### The source traits and the Send contract

`ReleaseSource` (`update.rs:325-340`) is synchronous and has three methods, all returning the
public `Result` type:

- `get_latest_release(&self) -> Result<Release>`
- `get_latest_releases(&self) -> Result<Vec<Release>>`
- `get_release_version(&self, ver: &str) -> Result<Release>`

The trait requires `Send + Sync`. It is documented as **not sealed**, so downstream crates may
implement it. `get_latest_releases` takes no `current_version` (the dead advisory parameter was
dropped): the updater re-filters downstream, discarding releases not strictly newer than current,
preferring the newest semver-compatible one, and otherwise offering the newest available, so the
source need not pre-filter. Releases should be returned newest-first.

`AsyncReleaseSource` (`update.rs:372-392`, gated on `feature = "async"`) is the async analog.
It also requires `Send + Sync` and has the same three methods, but each returns a
return-position `impl Trait` future rather than a value:

- `get_latest_release(&self) -> impl Future<Output = Result<Release>> + Send + '_`
- `get_latest_releases(&self) -> impl Future<Output = Result<Vec<Release>>> + Send + '_`
- `get_release_version<'a>(&'a self, ver: &'a str) -> impl Future<Output = Result<Release>> + Send + 'a`

The `+ Send` bound on each returned future is load-bearing. Because the trait is consumed
through generics (the async updater is generic over its source, never `dyn AsyncReleaseSource`),
the futures stay unboxed and there is **no `async-trait` dependency** (`update.rs:355-359`). The
`+ Send` bound forces the `Send` check at the impl site: a non-`Send` implementation fails to
compile where it is defined, not later at the spawn site (`update.rs:357-361`, `375-377`).
Implementors may still write the bodies as `async fn`; the compiler checks the resulting future
is `Send`.

On failure, both traits return public `Error` variants (`Error::Release`, `Error::Config`, or a
request variant such as `Error::NotFound { url }` / `Error::HttpStatus { status, url }` /
`Error::Transport`), which are constructible from a custom source (`update.rs:321-324`,
`367-370`).

### The custom adapter and Blocking

`custom::Update` (`custom.rs:189-192`, `#[non_exhaustive]`) holds an `Arc<dyn ReleaseSource>`
and a `CommonConfig`. It is built through `UpdateBuilder` (`custom.rs:143-185`): `.source(...)`
takes `impl ReleaseSource + 'static` and boxes it into the `Arc` (`custom.rs:164-167`);
`build()` (`custom.rs:175-184`) takes `&self`, clones the `Arc` source, and errors with
`Error::Config("`source` required")` when no source was set, so a configured builder can be
built repeatedly. `Update::configure()` returns a fresh `UpdateBuilder` (`custom.rs:205-207`).

`AsyncUpdate<S>` (`custom.rs:303-306`, `#[non_exhaustive]`, `feature = "async"`) is generic over
`S: AsyncReleaseSource` and stores `Arc<S>`. A trait object is never used, so the source's
`async fn`s need no boxing and `S` is not required to be `Clone` (the builder stores
`Arc<S>`). It is built through `AsyncUpdateBuilder<S>` (`custom.rs:240-295`) whose
`build_async()` (`custom.rs:285-294`) mirrors the sync builder (`&self`, clones the `Arc`,
errors when no source).

`Blocking<S>` (`custom.rs:357-359`, `feature = "async"`) adapts a `Clone` sync `ReleaseSource`
into an `AsyncReleaseSource`. Its single `source: S` field is **private**; the only ways to
construct or inspect it are the three methods (`custom.rs:362-377`):

- `Blocking::new(source: S) -> Self` (`custom.rs:364`)
- `into_inner(self) -> S` (`custom.rs:369`)
- `as_inner(&self) -> &S` (`custom.rs:374`)

`impl AsyncReleaseSource for Blocking<S> where S: ReleaseSource + Clone + 'static`
(`custom.rs:380-401`) runs each sync fetch on `tokio::task::spawn_blocking`, cloning the inner
source into the blocking task; a `JoinError` is mapped to `Error::Update("blocking task
failed: ...")`. The inner source's own error is returned unchanged.

### Integration with the pipeline

`Update` implements the sealed `ReleaseUpdate` trait (`custom.rs:214-230`) by delegating its
three fetch methods to the source. `get_latest_release` wraps the single release in a
one-element `Releases` carrying the configured `current_version` (so `is_update_available()`
works without a second fetch); `get_latest_releases` wraps the source's `Vec` likewise;
`get_release_version` returns the source's `Release` directly. Shared config accessors come from
`impl_update_config_accessors!(Update)` (`custom.rs:212`), and the sealed marker is
`impl sealed::Sealed for Update` (`custom.rs:210`). From there the crate runs its usual
compare -> select-asset -> download -> verify -> extract -> install flow over the source's
releases; the implementor never touches the low-level `Download`/`Extract`/`Move` primitives
(`custom.rs:5-10`).

`AsyncUpdate<S>` implements the public sealed `AsyncReleaseUpdate` trait the same way, plus
`sealed::Sealed` and the config accessors, so the async orchestrator drives the same flow
asynchronously. The `*_async` fetch verbs delegate to the source; `update_async` /
`update_extended_async` are `AsyncReleaseUpdate` default methods.

Transport caveats: the shared `.timeout()`, `.request_header()`, and injected-client setters
configure only the crate-controlled **download**; `.retries()` has no effect here because the
listing is entirely the source's responsibility. There is no `auth_token` on this backend (the
builders use `impl_common_builder_setters!(no_auth_token)`), and there is no `custom::ReleaseList`
(`custom.rs:43-51`, `120-121`, `130-140`).

## Public surface

- `trait ReleaseSource: Send + Sync` with `get_latest_release`, `get_latest_releases`,
  `get_release_version` (re-exported at crate root).
- `trait AsyncReleaseSource: Send + Sync` (feature `async`), same three methods returning
  `impl Future<Output = ...> + Send`.
- `backends::custom::Update` (`#[non_exhaustive]`) and `UpdateBuilder` with `configure()`,
  `source()`, `build()`. Note: `UpdateBuilder` is sync-only; it has `build()` but no
  `build_async()`. For async, use `AsyncUpdate::configure()` which returns an
  `AsyncUpdateBuilder<S>` with `build_async()`.
- `backends::custom::AsyncUpdate<S>` (`#[non_exhaustive]`, feature `async`) and
  `AsyncUpdateBuilder<S>` with `configure()`, `source()`, `build_async()`.
- `backends::custom::Blocking<S>` (feature `async`) with `new`, `into_inner`, `as_inner`.
- `Release`, `ReleaseAsset` and their builders (used to construct the values a source returns).

## Invariants and regression checklist

- `ReleaseSource` and `AsyncReleaseSource` are not sealed; downstream crates can implement them.
- Async futures must be `Send` at the type level: each `AsyncReleaseSource` method returns
  `impl Future<...> + Send`, so a non-`Send` impl fails to compile at the impl site. No
  `async-trait` dependency.
- `AsyncUpdate<S>` must not require `S: Clone` (it stores `Arc<S>`).
- `Blocking`'s inner `source` field stays private; construction/inspection only via
  `new`/`into_inner`/`as_inner` (a tuple-struct literal must not compile from outside).
- `build()` / `build_async()` take `&self` (repeatable) and error with `Error::Config` when no
  source is set.
- `Update` and `AsyncUpdate` carry `#[non_exhaustive]`.
- `.retries()` has no effect; no `auth_token` setter; download honors injected clients.
- A `Blocking` `JoinError` becomes `Error::Update`; the inner source error passes through.

## Tests

In `src/backends/custom.rs` (`custom.rs:403-1191`):

- `build_requires_a_source`, `build_is_repeatable`, `fetches_delegate_to_the_source`,
  `shared_accessors_are_wired` (including `auth_token() == None`).
- `is_update_available_*` and `get_latest_release_carries_current_version_for_the_precheck`
  (one-element `Releases` pre-check), `selects_asset_from_a_source_release`.
- Sync orchestrator end-to-end: `update_extended_resolves_explicit_tag_then_selects_that_release`,
  `update_extended_selects_newest_compatible_then_fails_at_missing_asset`.
- Async submodule: `build_async_requires_a_source`, `build_async_is_repeatable`,
  `async_fetches_delegate_to_the_native_source`,
  `blocking_adapter_drives_async_update_from_a_sync_source`,
  `blocking_adapter_propagates_sync_error`,
  `async_update_works_with_a_non_clone_source` (compile-time non-`Clone` proof),
  `blocking_new_as_inner_and_into_inner_round_trip` (private-field round trip),
  `async_release_source_future_is_send` plus the `assert_fetch_future_is_send` helper (Send
  enforcement), and the async orchestrator end-to-end tests.

## Related

- `custom-backends.md` - narrative design doc for this backend.
- `async-api.md` - the async API and `AsyncReleaseUpdate`/orchestrator integration.
- `custom-asset-matching.md` - asset selection over a source's releases.
- `choose-latest-release-sort.md` - newest-compatible selection over unsorted source lists.
