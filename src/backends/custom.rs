/*!
Updates from a user-defined release source.

Use this backend to update from a host the built-in backends (`github`, `gitlab`, `gitea`, `s3`)
don't cover. Implement [`ReleaseSource`](crate::ReleaseSource) for your host — the three methods
that say *where releases come from* — then configure a [`custom::Update`](Update) with the same
shared options as any other backend (target, bin name, version, progress, transport, checksum,
verify hook, asset matcher, …) and call `update()`. The crate runs its usual compare →
select-asset → download → verify → extract → install flow over your source; you never touch the
low-level `Download`/`Extract`/`Move` primitives.

```no_run
use self_update::{Release, ReleaseAsset, ReleaseSource, cargo_crate_version};

struct MyHost;
impl ReleaseSource for MyHost {
    fn get_latest_release(&self) -> self_update::Result<Release> {
        // ... your own HTTP request + parsing ...
        Ok(Release::builder()
            .version("1.2.3")
            .asset(ReleaseAsset::new("app-x86_64-unknown-linux-gnu.tar.gz", "https://host/app.tar.gz"))
            .build()?)
    }
    fn get_latest_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.get_latest_release()?])
    }
    fn get_release_version(&self, _ver: &str) -> self_update::Result<Release> {
        self.get_latest_release()
    }
}

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let status = self_update::backends::custom::Update::configure()
    .source(MyHost)
    .bin_name("app")
    .current_version(cargo_crate_version!())
    .build()?
    .update()?;
# Ok(())
# }
```

The source owns its own listing transport (HTTP client, auth, pagination). Of the shared transport
knobs, `.timeout()` and `.request_header()` apply to the crate-controlled **download**; if the
download itself needs an auth header, set it scheme-agnostically with
`.request_header(self_update::http::header::AUTHORIZATION, "Bearer …".parse()?)` (there is no
`auth_token` on this backend — its `token <…>` scheme is github-specific). `.retries()` has **no
effect** here: it only ever retried the built-in release-listing requests, and on this backend the
listing is entirely the source's responsibility. An injected client (`reqwest_client`,
`reqwest_async_client`, or `ureq_agent`) is also honored for the download — `build_download`
forwards the override, so the same client you supplied controls the actual file transfer.

# Async

With the `async` feature, there is an async custom updater too. For a natively-async listing
transport, implement [`AsyncReleaseSource`](crate::AsyncReleaseSource) and drive it through
[`AsyncUpdate`], which runs the same compare → download → verify → install flow asynchronously:

```no_run
# #[cfg(feature = "async")]
# mod demo {
use self_update::{AsyncReleaseSource, AsyncReleaseUpdate, Release, ReleaseAsset, cargo_crate_version};
use self_update::backends::custom::AsyncUpdate;

struct MyHost;
impl AsyncReleaseSource for MyHost {
    async fn get_latest_release(&self) -> self_update::Result<Release> {
        // ... your own async HTTP request + parsing ...
        Ok(Release::builder()
            .version("1.2.3")
            .asset(ReleaseAsset::new("app-x86_64-unknown-linux-gnu.tar.gz", "https://host/app.tar.gz"))
            .build()?)
    }
    async fn get_latest_releases(&self) -> self_update::Result<Vec<Release>> {
        Ok(vec![self.get_latest_release().await?])
    }
    async fn get_release_version(&self, _ver: &str) -> self_update::Result<Release> {
        self.get_latest_release().await
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _status = AsyncUpdate::configure()
        .source(MyHost)
        .bin_name("app")
        .current_version(cargo_crate_version!())
        .build_async()?
        .update_async()
        .await?;
    Ok(())
}
# }
```

If you already have a `Clone` *sync* [`ReleaseSource`] and just want to use it from the async API,
wrap it in [`Blocking`], which runs the sync fetches on [`tokio::task::spawn_blocking`]:

```no_run
# #[cfg(feature = "async")]
# {
use self_update::backends::custom::{AsyncUpdate, Blocking};
# #[derive(Clone)]
# struct MySyncHost;
# impl self_update::ReleaseSource for MySyncHost {
#     fn get_latest_release(&self) -> self_update::Result<self_update::Release> { unimplemented!() }
#     fn get_latest_releases(&self) -> self_update::Result<Vec<self_update::Release>> { unimplemented!() }
#     fn get_release_version(&self, _: &str) -> self_update::Result<self_update::Release> { unimplemented!() }
# }
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let _builder = AsyncUpdate::configure()
    .source(Blocking::new(MySyncHost))
    .bin_name("app")
    .current_version("1.0.0")
    .build_async()?;
# Ok(())
# }
# }
```

There is also no `custom::ReleaseList` (unlike the built-in backends): release listing is entirely
your [`ReleaseSource`]'s job, so query it directly instead.
*/

use std::sync::Arc;

use crate::backends::common::{CommonBuilderConfig, CommonConfig};
use crate::errors::*;
use crate::update::{Release, ReleaseSource, ReleaseUpdate, Releases};

/// `custom::Update` builder.
///
/// **Transport knobs and release listing:** the shared `.timeout()`, `.request_header()`, and
/// `.retries()` setters configure only the crate-controlled **download** on this backend, never
/// the release listing — the listing is performed entirely by your [`ReleaseSource`], which owns
/// its own HTTP client, auth, and pagination. In particular `.retries()` has **no effect at all**
/// here (it only ever retried the built-in listing requests), and `.timeout()` /
/// `.request_header()` apply to the download but not to your source's requests. Configure listing
/// transport inside your `ReleaseSource` implementation instead. An injected client
/// (`reqwest_client`, `reqwest_async_client`, or `ureq_agent`) is also honored for the download
/// — `build_download` forwards the override to the crate-controlled file transfer.
#[must_use]
#[derive(Clone, Default)]
pub struct UpdateBuilder {
    source: Option<Arc<dyn ReleaseSource>>,
    common: CommonBuilderConfig,
}

impl std::fmt::Debug for UpdateBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateBuilder")
            .field("source", &self.source.as_ref().map(|_| "<source>"))
            .field("common", &self.common)
            .finish()
    }
}

impl UpdateBuilder {
    /// Initialize a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the [`ReleaseSource`] that supplies releases for this update. Required.
    pub fn source(&mut self, source: impl ReleaseSource + 'static) -> &mut Self {
        self.source = Some(Arc::new(source));
        self
    }

    impl_common_builder_setters!(no_auth_token);

    /// Confirm config and create a ready-to-use `Update`.
    ///
    /// * Errors:
    ///     * Config - no `source` was set, or an invalid `Update` configuration
    pub fn build(&self) -> Result<Box<dyn ReleaseUpdate>> {
        let source = self
            .source
            .clone()
            .ok_or(Error::MissingField { field: "source" })?;
        Ok(Box::new(Update {
            source,
            common: self.common.build()?,
        }))
    }
}

/// Updates to a specified or latest release from a user-defined [`ReleaseSource`].
#[non_exhaustive]
pub struct Update {
    source: Arc<dyn ReleaseSource>,
    common: CommonConfig,
}

impl std::fmt::Debug for Update {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Update")
            .field("source", &"<source>")
            .field("common", &self.common)
            .finish()
    }
}

impl Update {
    /// Initialize a new `Update` builder.
    pub fn configure() -> UpdateBuilder {
        UpdateBuilder::new()
    }
}

impl crate::update::sealed::Sealed for Update {}

impl_update_config_accessors!(Update);

impl ReleaseUpdate for Update {
    fn get_latest_release(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let release = self.source.get_latest_release()?;
        Ok(Releases::new(vec![release], current_version))
    }

    fn get_latest_releases(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = self.source.get_latest_releases()?;
        Ok(Releases::new(releases, current_version))
    }

    fn get_release_version(&self, ver: &str) -> Result<Release> {
        self.source.get_release_version(ver)
    }
}

/// Builder for an [`AsyncUpdate`].
///
/// Generic over the [`AsyncReleaseSource`](crate::AsyncReleaseSource) so the async updater never
/// uses a trait object — the source's `async fn`s need no boxing. The same transport-knob caveats
/// as [`UpdateBuilder`] apply: `.timeout()` / `.request_header()` configure only the
/// crate-controlled **download**, and `.retries()` has no effect (your source owns listing).
#[cfg(feature = "async")]
#[must_use]
pub struct AsyncUpdateBuilder<S: crate::update::AsyncReleaseSource> {
    source: Option<Arc<S>>,
    common: CommonBuilderConfig,
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> Default for AsyncUpdateBuilder<S> {
    fn default() -> Self {
        Self {
            source: None,
            common: CommonBuilderConfig::default(),
        }
    }
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> std::fmt::Debug for AsyncUpdateBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncUpdateBuilder")
            .field("source", &self.source.as_ref().map(|_| "<source>"))
            .field("common", &self.common)
            .finish()
    }
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> AsyncUpdateBuilder<S> {
    /// Initialize a new builder.
    pub fn new() -> Self {
        Default::default()
    }

    /// Set the [`AsyncReleaseSource`](crate::AsyncReleaseSource) that supplies releases for this
    /// update. Required.
    pub fn source(&mut self, source: S) -> &mut Self {
        self.source = Some(Arc::new(source));
        self
    }

    impl_common_builder_setters!(no_auth_token);

    /// Confirm config and create a ready-to-use [`AsyncUpdate`].
    ///
    /// * Errors:
    ///     * Config - no `source` was set, or an invalid `Update` configuration
    pub fn build_async(&self) -> Result<AsyncUpdate<S>> {
        let source = self
            .source
            .clone()
            .ok_or(Error::MissingField { field: "source" })?;
        Ok(AsyncUpdate {
            source,
            common: self.common.build()?,
        })
    }
}

/// Async sibling of [`Update`]: updates to a specified or latest release from a user-defined
/// [`AsyncReleaseSource`](crate::AsyncReleaseSource).
///
/// Generic over the source (no trait object), so the source's `async fn`s need no boxing.
#[cfg(feature = "async")]
#[non_exhaustive]
pub struct AsyncUpdate<S: crate::update::AsyncReleaseSource> {
    source: Arc<S>,
    common: CommonConfig,
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> std::fmt::Debug for AsyncUpdate<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncUpdate")
            .field("source", &"<source>")
            .field("common", &self.common)
            .finish()
    }
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> AsyncUpdate<S> {
    /// Initialize a new [`AsyncUpdate`] builder.
    pub fn configure() -> AsyncUpdateBuilder<S> {
        AsyncUpdateBuilder::new()
    }
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> crate::update::sealed::Sealed for AsyncUpdate<S> {}

#[cfg(feature = "async")]
impl_update_config_accessors!(AsyncUpdate<S>, where (S: crate::update::AsyncReleaseSource));

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> crate::update::AsyncReleaseUpdate for AsyncUpdate<S> {
    async fn get_latest_release_async(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let release = self.source.get_latest_release().await?;
        Ok(Releases::new(vec![release], current_version))
    }
    async fn get_latest_releases_async(&self) -> Result<Releases> {
        let current_version = crate::update::UpdateConfig::current_version(self).to_owned();
        let releases = self.source.get_latest_releases().await?;
        Ok(Releases::new(releases, current_version))
    }
    async fn get_release_version_async(&self, ver: &str) -> Result<Release> {
        self.source.get_release_version(ver).await
    }
}

/// Adapter that lets a `Clone` *sync* [`ReleaseSource`] be used with the async
/// [`AsyncUpdate`] — its fetches run on [`tokio::task::spawn_blocking`].
///
/// Wrap a sync source: `AsyncUpdate::configure().source(Blocking::new(my_sync_source))`. The inner
/// source must be `Clone + 'static` because each fetch clones it into the blocking task.
#[cfg(feature = "async")]
pub struct Blocking<S> {
    source: S,
}

#[cfg(feature = "async")]
impl<S> Blocking<S> {
    /// Wrap a sync [`ReleaseSource`] so it can drive an [`AsyncUpdate`].
    pub fn new(source: S) -> Self {
        Self { source }
    }

    /// Consume the adapter and return the wrapped source.
    pub fn into_inner(self) -> S {
        self.source
    }

    /// Borrow the wrapped source.
    pub fn as_inner(&self) -> &S {
        &self.source
    }
}

#[cfg(feature = "async")]
impl<S: ReleaseSource + Clone + 'static> crate::update::AsyncReleaseSource for Blocking<S> {
    async fn get_latest_release(&self) -> Result<Release> {
        let s = self.source.clone();
        tokio::task::spawn_blocking(move || s.get_latest_release())
            .await
            .map_err(|e| Error::Internal {
                message: "blocking task failed".to_string(),
                source: Some(Box::new(e)),
            })?
    }
    async fn get_latest_releases(&self) -> Result<Vec<Release>> {
        let s = self.source.clone();
        tokio::task::spawn_blocking(move || s.get_latest_releases())
            .await
            .map_err(|e| Error::Internal {
                message: "blocking task failed".to_string(),
                source: Some(Box::new(e)),
            })?
    }
    async fn get_release_version(&self, ver: &str) -> Result<Release> {
        let s = self.source.clone();
        let ver = ver.to_owned();
        tokio::task::spawn_blocking(move || s.get_release_version(&ver))
            .await
            .map_err(|e| Error::Internal {
                message: "blocking task failed".to_string(),
                source: Some(Box::new(e)),
            })?
    }
}

#[cfg(test)]
mod tests {
    use super::Update;
    use crate::update::{Release, ReleaseAsset, ReleaseSource, ReleaseUpdate};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A canned source that records how many times each method was called.
    struct FakeSource {
        latest_calls: Arc<AtomicUsize>,
    }

    impl ReleaseSource for FakeSource {
        fn get_latest_release(&self) -> crate::errors::Result<Release> {
            self.latest_calls.fetch_add(1, Ordering::SeqCst);
            Release::builder()
                .version("2.0.0")
                .asset(ReleaseAsset::new(
                    "app-x86_64-unknown-linux-gnu.tar.gz",
                    "https://example/app-2.0.0.tar.gz",
                ))
                .build()
        }
        fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
            Ok(vec![self.get_latest_release()?])
        }
        fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
            Release::builder().version(ver).build()
        }
    }

    fn configured(calls: Arc<AtomicUsize>) -> Box<dyn ReleaseUpdate> {
        Update::configure()
            .source(FakeSource {
                latest_calls: calls,
            })
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .build()
            .unwrap()
    }

    #[test]
    fn build_requires_a_source() {
        let res = Update::configure()
            .bin_name("app")
            .current_version("1.0.0")
            .build();
        assert!(res.is_err(), "build must fail without a source");
    }

    #[test]
    fn build_is_repeatable() {
        // `build` takes `&self` and clones the (Arc) source, so a configured builder can be built
        // more than once — matching every other backend. On the old `&mut self` + `take()` build
        // the second call would fail with "`source` required".
        let mut builder = Update::configure();
        builder
            .source(FakeSource {
                latest_calls: Arc::new(AtomicUsize::new(0)),
            })
            .bin_name("app")
            .current_version("1.0.0");
        builder.build().expect("first build");
        builder.build().expect("second build");
    }

    #[test]
    fn fetches_delegate_to_the_source() {
        let calls = Arc::new(AtomicUsize::new(0));
        let upd = configured(calls.clone());

        let latest = upd.get_latest_release().unwrap();
        let rel = latest.latest().expect("one-element Releases");
        assert_eq!(rel.version(), "2.0.0");
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(
            latest.all().len(),
            1,
            "get_latest_release yields one element"
        );

        let rels = upd.get_latest_releases().unwrap();
        assert_eq!(rels.all().len(), 1);

        let tagged = upd.get_release_version("1.5.0").unwrap();
        assert_eq!(tagged.version(), "1.5.0");

        // get_latest_release once + once inside get_latest_releases = 2.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn shared_accessors_are_wired() {
        let upd = configured(Arc::new(AtomicUsize::new(0)));
        assert_eq!(upd.target(), "x86_64-unknown-linux-gnu");
        assert_eq!(upd.bin_name(), "app");
        assert_eq!(upd.current_version(), "1.0.0");
        // The custom backend has no auth token (its source owns listing auth).
        assert_eq!(upd.auth_token(), None);
    }

    #[test]
    fn is_update_available_true_when_latest_is_newer() {
        // D1: the pre-check now lives on `Releases`. FakeSource's latest release is 2.0.0; with
        // current_version 1.0.0 an update is available.
        let upd = configured(Arc::new(AtomicUsize::new(0)));
        assert!(
            upd.get_latest_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "2.0.0 > 1.0.0 => update available"
        );
    }

    #[test]
    fn is_update_available_false_when_latest_not_newer() {
        // D1 complement: when current_version equals the latest release version, no update is
        // available. FakeSource reports 2.0.0, so configure current_version at 2.0.0.
        let upd = Update::configure()
            .source(FakeSource {
                latest_calls: Arc::new(AtomicUsize::new(0)),
            })
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("2.0.0")
            .build()
            .unwrap();
        assert!(
            !upd.get_latest_releases()
                .unwrap()
                .is_update_available()
                .unwrap(),
            "latest (2.0.0) is not newer than current (2.0.0) => no update"
        );
    }

    #[test]
    fn get_latest_release_carries_current_version_for_the_precheck() {
        // D1: `get_latest_release` returns a one-element `Releases` carrying the configured
        // current_version, so `.is_update_available()` works directly off the single newest
        // release without a second fetch.
        let upd = configured(Arc::new(AtomicUsize::new(0)));
        let releases = upd.get_latest_release().unwrap();
        assert_eq!(releases.all().len(), 1);
        assert!(
            releases.is_update_available().unwrap(),
            "2.0.0 > 1.0.0 via the one-element Releases pre-check"
        );
    }

    #[test]
    fn selects_asset_from_a_source_release() {
        let upd = configured(Arc::new(AtomicUsize::new(0)));
        let releases = upd.get_latest_release().unwrap();
        let rel = releases.latest().expect("one-element Releases");
        // The crate's asset selection runs over the source's release just like any backend.
        let asset = rel
            .asset_for("x86_64-unknown-linux-gnu", None)
            .expect("asset matches the target");
        assert_eq!(asset.download_url(), "https://example/app-2.0.0.tar.gz");
    }

    // --- Sync end-to-end path tests (analogue of the async OrchestratedSource tests) -----------
    //
    // These mirror `update_extended_async_resolves_explicit_tag_then_selects_that_release` and
    // `update_extended_async_selects_newest_compatible_then_fails_at_missing_asset` from the async
    // submodule below. The async tests proved those paths work through `AsyncUpdate`; the tests here
    // prove the same paths work through the sync `Update` and its `update_extended()` orchestrator.

    /// A sync source whose `get_latest_releases` returns several candidates (out of order, some
    /// older than current) so `choose_latest_release` runs over real source data. Each release
    /// carries a single asset whose name embeds its version (`app-<ver>.bin`), letting an
    /// asset-matcher report which release the orchestrator actually selected.
    struct SyncOrchestratedSource;

    fn versioned_release_sync(v: &str) -> Release {
        Release::builder()
            .version(v)
            .asset(ReleaseAsset::new(
                format!("app-{}.bin", v),
                format!("https://example/app-{}.bin", v),
            ))
            .build()
            .unwrap()
    }

    impl ReleaseSource for SyncOrchestratedSource {
        fn get_latest_release(&self) -> crate::errors::Result<Release> {
            self.get_release_version("9.9.9")
        }
        fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
            // Newest-but-incompatible, current, older, and the compatible winner — out of order.
            // The orchestrator must drop current/older and pick the newest compatible (1.4.0),
            // not 2.0.0 and not 1.0.0.
            Ok(vec![
                versioned_release_sync("1.0.0"),
                versioned_release_sync("2.0.0"),
                versioned_release_sync("0.9.0"),
                versioned_release_sync("1.4.0"),
                versioned_release_sync("1.2.0"),
            ])
        }
        fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
            Ok(versioned_release_sync(ver))
        }
    }

    #[test]
    fn update_extended_resolves_explicit_tag_then_selects_that_release() {
        // With an explicit release_tag the sync orchestrator takes the get_release_version path
        // (not choose_latest_release) and runs asset selection over *that* tagged release.
        // The asset-matcher records the asset names it sees (proving which release was fetched),
        // then returns None so the update fails at resolve_and_confirm rather than attempting a
        // real download.
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(vec![]));
        let seen_cb = seen.clone();
        let upd = Update::configure()
            .source(SyncOrchestratedSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .release_tag("7.7.7")
            .no_confirm(true)
            .show_output(false)
            .asset_matcher(move |assets| {
                let mut s = seen_cb.lock().unwrap();
                for a in assets {
                    s.push(a.name().to_string());
                }
                None
            })
            .build()
            .unwrap();
        let err = upd
            .update_extended()
            .expect_err("matcher returning None must fail the update");
        assert!(
            matches!(err, crate::errors::Error::NoReleaseFound { .. }),
            "explicit-tag path still runs asset selection, got {:?}",
            err
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["app-7.7.7.bin".to_string()],
            "the explicit-tag path must resolve and select the tagged release (7.7.7)"
        );
    }

    #[test]
    fn update_extended_selects_newest_compatible_then_fails_at_missing_asset() {
        // Reaches past choose_latest_release into asset selection. A recording asset-matcher
        // captures the asset names of the *chosen* release, proving the sync orchestrator selected
        // the newest **compatible** release (1.4.0) over the source's out-of-order list — not the
        // newer-but-incompatible 2.0.0, nor the current 1.0.0. The matcher then returns None, so
        // the update fails at resolve_and_confirm with an Error::Release (rather than attempting a
        // real download).
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(vec![]));
        let seen_cb = seen.clone();
        let upd = Update::configure()
            .source(SyncOrchestratedSource)
            .bin_name("app")
            .target("x86_64-unknown-linux-gnu")
            .current_version("1.0.0")
            .no_confirm(true)
            .show_output(false)
            .asset_matcher(move |assets| {
                let mut s = seen_cb.lock().unwrap();
                for a in assets {
                    s.push(a.name().to_string());
                }
                None
            })
            .build()
            .unwrap();
        let err = upd
            .update_extended()
            .expect_err("matcher returning None must fail the update");
        assert!(
            matches!(err, crate::errors::Error::NoReleaseFound { .. }),
            "no asset selected -> Error::NoReleaseFound, got {:?}",
            err
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["app-1.4.0.bin".to_string()],
            "the sync orchestrator must select the newest compatible release (1.4.0)"
        );
    }

    #[cfg(feature = "async")]
    mod async_tests {
        use super::super::{AsyncUpdate, Blocking};
        use crate::update::{
            AsyncReleaseSource, AsyncReleaseUpdate, Release, ReleaseAsset, ReleaseSource,
        };
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// A natively-async source recording how many times each async method ran.
        struct NativeAsyncSource {
            latest_calls: Arc<AtomicUsize>,
            releases_calls: Arc<AtomicUsize>,
            version_calls: Arc<AtomicUsize>,
        }

        impl AsyncReleaseSource for NativeAsyncSource {
            async fn get_latest_release(&self) -> crate::errors::Result<Release> {
                // Genuinely yields to the executor (no blocking thread).
                tokio::task::yield_now().await;
                self.latest_calls.fetch_add(1, Ordering::SeqCst);
                Release::builder()
                    .version("2.0.0")
                    .asset(ReleaseAsset::new(
                        "app-x86_64-unknown-linux-gnu.tar.gz",
                        "https://example/app-2.0.0.tar.gz",
                    ))
                    .build()
            }
            async fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                tokio::task::yield_now().await;
                self.releases_calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![Release::builder().version("2.0.0").build()?])
            }
            async fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                tokio::task::yield_now().await;
                self.version_calls.fetch_add(1, Ordering::SeqCst);
                Release::builder().version(ver).build()
            }
        }

        #[tokio::test]
        async fn build_async_requires_a_source() {
            let res = AsyncUpdate::<NativeAsyncSource>::configure()
                .bin_name("app")
                .current_version("1.0.0")
                .build_async();
            assert!(res.is_err(), "build_async must fail without a source");
        }

        #[tokio::test]
        async fn build_async_is_repeatable() {
            // `build_async` clones the (Arc) source, so a configured builder builds more than once.
            let mut builder = AsyncUpdate::configure();
            builder
                .source(NativeAsyncSource {
                    latest_calls: Arc::new(AtomicUsize::new(0)),
                    releases_calls: Arc::new(AtomicUsize::new(0)),
                    version_calls: Arc::new(AtomicUsize::new(0)),
                })
                .bin_name("app")
                .current_version("1.0.0");
            builder.build_async().expect("first build_async");
            builder.build_async().expect("second build_async");
        }

        #[tokio::test]
        async fn async_fetches_delegate_to_the_native_source() {
            let latest = Arc::new(AtomicUsize::new(0));
            let releases = Arc::new(AtomicUsize::new(0));
            let version = Arc::new(AtomicUsize::new(0));
            let upd = AsyncUpdate::configure()
                .source(NativeAsyncSource {
                    latest_calls: latest.clone(),
                    releases_calls: releases.clone(),
                    version_calls: version.clone(),
                })
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .build_async()
                .unwrap();

            let latest_releases = upd.get_latest_release_async().await.unwrap();
            let rel = latest_releases.latest().expect("one-element Releases");
            assert_eq!(rel.version(), "2.0.0");
            assert_eq!(rel.assets.len(), 1);

            let rels = AsyncReleaseUpdate::get_latest_releases_async(&upd)
                .await
                .unwrap();
            assert_eq!(rels.all().len(), 1);

            let tagged = AsyncReleaseUpdate::get_release_version_async(&upd, "1.5.0")
                .await
                .unwrap();
            assert_eq!(tagged.version(), "1.5.0");

            assert_eq!(latest.load(Ordering::SeqCst), 1);
            assert_eq!(releases.load(Ordering::SeqCst), 1);
            assert_eq!(version.load(Ordering::SeqCst), 1);
        }

        /// A `Clone` sync source recording its sync-method calls.
        #[derive(Clone)]
        struct SyncSource {
            calls: Arc<AtomicUsize>,
        }

        impl ReleaseSource for SyncSource {
            fn get_latest_release(&self) -> crate::errors::Result<Release> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Release::builder().version("3.0.0").build()
            }
            fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(vec![Release::builder().version("3.0.0").build()?])
            }
            fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Release::builder().version(ver).build()
            }
        }

        #[tokio::test]
        async fn blocking_adapter_drives_async_update_from_a_sync_source() {
            let calls = Arc::new(AtomicUsize::new(0));
            let upd = AsyncUpdate::configure()
                .source(Blocking::new(SyncSource {
                    calls: calls.clone(),
                }))
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .build_async()
                .unwrap();

            // The async fetches run the sync source's methods on spawn_blocking and return them.
            assert_eq!(
                upd.get_latest_release_async()
                    .await
                    .unwrap()
                    .latest()
                    .unwrap()
                    .version(),
                "3.0.0"
            );
            assert_eq!(
                AsyncReleaseUpdate::get_latest_releases_async(&upd)
                    .await
                    .unwrap()
                    .all()
                    .len(),
                1
            );
            assert_eq!(
                AsyncReleaseUpdate::get_release_version_async(&upd, "9.9.9")
                    .await
                    .unwrap()
                    .version(),
                "9.9.9"
            );

            assert_eq!(
                calls.load(Ordering::SeqCst),
                3,
                "each async fetch must delegate to the sync source exactly once"
            );
        }

        /// An end-to-end async source whose `get_latest_releases` returns several candidates
        /// (out of order, some older than current) so the async orchestrator's
        /// `choose_latest_release` selection runs over real source data. Each release carries a
        /// single asset whose name embeds its version (`app-<ver>.bin`), so a downstream
        /// asset-matcher can report which release the orchestrator actually selected.
        struct OrchestratedSource;

        fn versioned_release(v: &str) -> Release {
            Release::builder()
                .version(v)
                .asset(ReleaseAsset::new(
                    format!("app-{}.bin", v),
                    format!("https://example/app-{}.bin", v),
                ))
                .build()
                .unwrap()
        }

        impl AsyncReleaseSource for OrchestratedSource {
            async fn get_latest_release(&self) -> crate::errors::Result<Release> {
                self.get_release_version("9.9.9").await
            }
            async fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                tokio::task::yield_now().await;
                // Newest-but-incompatible, current, older, and the compatible winner — out of
                // order. The orchestrator must drop current/older and pick the newest compatible
                // (1.4.0), not 2.0.0 and not 1.0.0.
                Ok(vec![
                    versioned_release("1.0.0"),
                    versioned_release("2.0.0"),
                    versioned_release("0.9.0"),
                    versioned_release("1.4.0"),
                    versioned_release("1.2.0"),
                ])
            }
            async fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                tokio::task::yield_now().await;
                Ok(versioned_release(ver))
            }
        }

        #[tokio::test]
        async fn update_extended_async_reports_up_to_date_through_orchestrator() {
            // A source returning only the current/older versions must drive the async orchestrator
            // (fetch -> choose_latest_release) to an UpToDate outcome without touching the download.
            struct OldOnly;
            impl AsyncReleaseSource for OldOnly {
                async fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    Release::builder().version("1.0.0").build()
                }
                async fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    tokio::task::yield_now().await;
                    Ok(vec![
                        Release::builder().version("1.0.0").build()?,
                        Release::builder().version("0.9.0").build()?,
                    ])
                }
                async fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                    Release::builder().version(ver).build()
                }
            }
            let upd = AsyncUpdate::configure()
                .source(OldOnly)
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .no_confirm(true)
                .show_output(false)
                .build_async()
                .unwrap();
            let status = upd.update_extended_async().await.unwrap();
            assert!(
                status.is_up_to_date(),
                "only current/older releases -> up-to-date through the async orchestrator"
            );
        }

        #[tokio::test]
        async fn update_extended_async_selects_newest_compatible_then_fails_at_missing_asset() {
            // Reaches past choose_latest_release into asset selection. A recording asset-matcher
            // captures the asset names of the *chosen* release, proving the async orchestrator
            // selected the newest **compatible** release (1.4.0) over the source's out-of-order
            // list — not the newer-but-incompatible 2.0.0, nor the current 1.0.0. The matcher then
            // returns `None`, so the update fails at `resolve_and_confirm` with the target message
            // (rather than attempting a real download).
            let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(vec![]));
            let seen_cb = seen.clone();
            let upd = AsyncUpdate::configure()
                .source(OrchestratedSource)
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .no_confirm(true)
                .show_output(false)
                .asset_matcher(move |assets| {
                    let mut s = seen_cb.lock().unwrap();
                    for a in assets {
                        s.push(a.name().to_string());
                    }
                    None
                })
                .build_async()
                .unwrap();

            let err = upd
                .update_extended_async()
                .await
                .expect_err("matcher returning None must fail the update");
            assert!(
                matches!(err, crate::errors::Error::NoReleaseFound { .. }),
                "no asset selected -> Error::NoReleaseFound, got {:?}",
                err
            );
            assert_eq!(
                *seen.lock().unwrap(),
                vec!["app-1.4.0.bin".to_string()],
                "the async orchestrator must select the newest compatible release (1.4.0)"
            );
        }

        #[tokio::test]
        async fn update_extended_async_resolves_explicit_tag_then_selects_that_release() {
            // With an explicit release_tag the orchestrator takes the get_release_version path
            // (not choose_latest_release) and runs asset selection over *that* tagged release.
            let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(vec![]));
            let seen_cb = seen.clone();
            let mut builder = AsyncUpdate::configure();
            builder
                .source(OrchestratedSource)
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .release_tag("7.7.7")
                .no_confirm(true)
                .show_output(false)
                .asset_matcher(move |assets| {
                    let mut s = seen_cb.lock().unwrap();
                    for a in assets {
                        s.push(a.name().to_string());
                    }
                    None
                });
            let upd = builder.build_async().unwrap();
            let err = upd
                .update_extended_async()
                .await
                .expect_err("matcher returning None must fail the update");
            assert!(
                matches!(err, crate::errors::Error::NoReleaseFound { .. }),
                "explicit-tag path still runs asset selection, got {:?}",
                err
            );
            assert_eq!(
                *seen.lock().unwrap(),
                vec!["app-7.7.7.bin".to_string()],
                "the explicit-tag path must resolve and select the tagged release (7.7.7)"
            );
        }

        #[tokio::test]
        async fn async_update_works_with_a_non_clone_source() {
            // `AsyncUpdate<S>` must not require `S: Clone` (the builder stores `Arc<S>`). This
            // source holds a non-Clone field and is itself non-Clone; if a `Clone` bound ever
            // crept back into `AsyncUpdate`/the builder/the macro, this would fail to compile.
            // `Mutex<()>` is `Send + Sync` (satisfying `AsyncReleaseSource`) but is not `Clone`,
            // and a struct is only `Clone` if it derives it — so `NonClone` is genuinely non-Clone.
            struct NonClone {
                _lock: std::sync::Mutex<()>,
            }
            impl AsyncReleaseSource for NonClone {
                async fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    Release::builder().version("1.0.0").build()
                }
                async fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    Ok(vec![Release::builder().version("1.0.0").build()?])
                }
                async fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                    Release::builder().version(ver).build()
                }
            }
            // Compile-time proof that NonClone is genuinely not Clone: a Clone bound here would be
            // unsatisfiable. (Kept as a function so the negative reasoning is explicit.)
            fn assert_not_requiring_clone<S: AsyncReleaseSource>(_: &AsyncUpdate<S>) {}

            let upd = AsyncUpdate::configure()
                .source(NonClone {
                    _lock: std::sync::Mutex::new(()),
                })
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("2.0.0")
                .no_confirm(true)
                .show_output(false)
                .build_async()
                .unwrap();
            assert_not_requiring_clone(&upd);
            // Drive it to an observable outcome (current is newer than the only release).
            let status = upd.update_extended_async().await.unwrap();
            assert!(status.is_up_to_date());
        }

        #[tokio::test]
        async fn blocking_adapter_propagates_sync_error() {
            #[derive(Clone)]
            struct FailingSource;
            impl ReleaseSource for FailingSource {
                fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    Err(crate::errors::Error::NoReleaseFound { target: None })
                }
                fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    Err(crate::errors::Error::HttpStatus {
                        status: 503,
                        url: "u".into(),
                    })
                }
                fn get_release_version(&self, _ver: &str) -> crate::errors::Result<Release> {
                    Err(crate::errors::Error::MissingField { field: "cfg" })
                }
            }

            let blk = Blocking::new(FailingSource);
            assert!(matches!(
                AsyncReleaseSource::get_latest_release(&blk).await,
                Err(crate::errors::Error::NoReleaseFound { .. })
            ));
            assert!(matches!(
                AsyncReleaseSource::get_latest_releases(&blk).await,
                Err(crate::errors::Error::HttpStatus { .. })
            ));
            assert!(matches!(
                AsyncReleaseSource::get_release_version(&blk, "1.0.0").await,
                Err(crate::errors::Error::MissingField { .. })
            ));
        }

        // E3/E6: a panic inside the spawned blocking task fails the join, which the adapter maps to
        // `Error::Internal` carrying the tokio `JoinError` as a boxed `source()` (previously the
        // `JoinError` was stringified and dropped, so `source()` returned `None`).
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn blocking_adapter_join_failure_chains_source() {
            use std::error::Error as _;
            #[derive(Clone)]
            struct PanickingSource;
            impl ReleaseSource for PanickingSource {
                fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    panic!("boom in blocking task");
                }
                fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    unreachable!()
                }
                fn get_release_version(&self, _ver: &str) -> crate::errors::Result<Release> {
                    unreachable!()
                }
            }

            let blk = Blocking::new(PanickingSource);
            let err = AsyncReleaseSource::get_latest_release(&blk)
                .await
                .expect_err("a panicking blocking task must fail the join");
            assert!(
                matches!(err, crate::errors::Error::Internal { .. }),
                "join failure must surface as Error::Internal, got {:?}",
                err
            );
            assert!(
                err.source().is_some(),
                "Internal from a JoinError must chain a non-None source()"
            );
        }

        // B7a: `Blocking`'s inner source field is private; the only ways to construct/inspect it
        // are `new`, `as_inner`, and `into_inner`. (A `Blocking(SyncSource { .. })` tuple-struct
        // literal would no longer compile, which is the breaking change this pins.)
        #[test]
        fn blocking_new_as_inner_and_into_inner_round_trip() {
            #[derive(Clone, PartialEq, Debug)]
            struct Marker(u32);
            impl ReleaseSource for Marker {
                fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    Release::builder().version("1.0.0").build()
                }
                fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    Ok(vec![])
                }
                fn get_release_version(&self, v: &str) -> crate::errors::Result<Release> {
                    Release::builder().version(v).build()
                }
            }
            let blk = Blocking::new(Marker(7));
            assert_eq!(blk.as_inner(), &Marker(7), "as_inner borrows the source");
            assert_eq!(blk.into_inner(), Marker(7), "into_inner returns the source");
        }

        // D2: the `AsyncReleaseSource` methods return `impl Future + Send`, so this generic helper
        // (which requires the returned future to be `Send`) must compile for any conforming impl.
        // If the `+ Send` bound were dropped from the trait, this `fn` would fail to compile.
        fn assert_fetch_future_is_send<S: AsyncReleaseSource>(s: &S) {
            fn is_send<T: Send>(_: &T) {}
            is_send(&s.get_latest_release());
        }

        #[tokio::test]
        async fn async_release_source_future_is_send() {
            // Drive the D2 Send-enforcement helper against the native async source, proving the
            // returned future satisfies the `Send` bound declared on the trait method.
            let src = NativeAsyncSource {
                latest_calls: Arc::new(AtomicUsize::new(0)),
                releases_calls: Arc::new(AtomicUsize::new(0)),
                version_calls: Arc::new(AtomicUsize::new(0)),
            };
            assert_fetch_future_is_send(&src);
            // And it still resolves correctly when awaited.
            assert_eq!(src.get_latest_release().await.unwrap().version(), "2.0.0");
        }

        #[tokio::test]
        async fn is_update_available_async_true_then_false() {
            // D2 (async): the pre-check is `get_latest_releases_async().await?.is_update_available()`.
            // The native source's latest is 2.0.0, so an update is available from 1.0.0 but not
            // from 2.0.0.
            let mk = |cur: &str| {
                AsyncUpdate::configure()
                    .source(NativeAsyncSource {
                        latest_calls: Arc::new(AtomicUsize::new(0)),
                        releases_calls: Arc::new(AtomicUsize::new(0)),
                        version_calls: Arc::new(AtomicUsize::new(0)),
                    })
                    .bin_name("app")
                    .target("x86_64-unknown-linux-gnu")
                    .current_version(cur)
                    .build_async()
                    .unwrap()
            };
            assert!(
                mk("1.0.0")
                    .get_latest_releases_async()
                    .await
                    .unwrap()
                    .is_update_available()
                    .unwrap(),
                "2.0.0 > 1.0.0 => update available"
            );
            assert!(
                !mk("2.0.0")
                    .get_latest_releases_async()
                    .await
                    .unwrap()
                    .is_update_available()
                    .unwrap(),
                "2.0.0 not newer than 2.0.0 => no update"
            );
        }

        // --- WS2 invariant 6: spawn_blocking finish tail driven to a real install ---------------
        //
        // No other async test drives the full async pipeline (fetch -> download -> spawn_blocking
        // verify/extract/install) to a successful install. This one does: a loopback HTTP server
        // serves a real tar.gz; the async source returns a release whose asset points at it; the
        // updater downloads it asynchronously and the blocking finish tail (run off-executor on
        // `tokio::task::spawn_blocking`) extracts `app` and installs it to a temp path. A successful
        // `ReleaseStatus::Updated` plus the installed file on disk proves the spawn_blocking tail
        // ran and completed.
        #[cfg(all(feature = "archive-tar", feature = "compression-tar-gz"))]
        #[tokio::test]
        async fn update_extended_async_downloads_and_installs_through_the_spawn_blocking_tail() {
            use std::io::{Read as _, Write as _};
            use std::net::TcpListener;

            // Build a tiny tar.gz in memory containing a single file named `app` (the default
            // `bin_path_in_archive` on a unix target, EXE_SUFFIX empty).
            let archive_bytes: Vec<u8> = {
                let mut tar = tar::Builder::new(Vec::new());
                let contents = b"installed-binary-payload";
                let mut header = tar::Header::new_gnu();
                header.set_path("app").unwrap();
                header.set_size(contents.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                tar.append(&header, &contents[..]).unwrap();
                let tar_bytes = tar.into_inner().unwrap();
                let mut enc =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                enc.write_all(&tar_bytes).unwrap();
                enc.finish().unwrap()
            };

            // Serve the archive once over loopback.
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let body = archive_bytes.clone();
            std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                }
            });

            // A source returning a single newer release whose asset points at the loopback archive.
            struct ServingSource {
                url: String,
            }
            impl AsyncReleaseSource for ServingSource {
                async fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    self.get_release_version("9.9.9").await
                }
                async fn get_latest_releases(&self) -> crate::errors::Result<Vec<Release>> {
                    tokio::task::yield_now().await;
                    Ok(vec![
                        Release::builder()
                            .version("2.0.0")
                            .asset(ReleaseAsset::new("app.tar.gz", self.url.clone()))
                            .build()?,
                    ])
                }
                async fn get_release_version(&self, ver: &str) -> crate::errors::Result<Release> {
                    tokio::task::yield_now().await;
                    Release::builder()
                        .version(ver)
                        .asset(ReleaseAsset::new("app.tar.gz", self.url.clone()))
                        .build()
                }
            }

            let install_dir = tempfile::tempdir().unwrap();
            let install_path = install_dir.path().join("installed-app");

            let upd = AsyncUpdate::configure()
                .source(ServingSource {
                    url: format!("{base}/app.tar.gz"),
                })
                .bin_name("app")
                .target("x86_64-unknown-linux-gnu")
                .current_version("1.0.0")
                .bin_install_path(&install_path)
                .no_confirm(true)
                .show_output(false)
                // Pick the single served asset directly, sidestepping target-name matching.
                .asset_matcher(|assets| assets.first().cloned())
                .build_async()
                .unwrap();

            let status = upd.update_extended_async().await.expect(
                "the async update must download and install through the spawn_blocking tail",
            );
            assert!(
                status.is_updated(),
                "a newer release served as a real tar.gz must install -> Updated, got {:?}",
                status
            );
            assert_eq!(status.updated_release().map(|r| r.version()), Some("2.0.0"));
            // The blocking finish tail actually wrote the extracted binary to the install path.
            assert!(
                install_path.exists(),
                "the spawn_blocking finish tail must have installed the binary to {:?}",
                install_path
            );
            assert_eq!(
                std::fs::read(&install_path).unwrap(),
                b"installed-binary-payload",
                "the installed file must be the binary extracted from the archive"
            );
        }
    }
}
