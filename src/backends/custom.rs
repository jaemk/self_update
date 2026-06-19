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
    fn get_latest_releases(&self, _current: &str) -> self_update::Result<Vec<Release>> {
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
use self_update::{AsyncReleaseSource, Release, ReleaseAsset, cargo_crate_version};
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
    async fn get_latest_releases(&self, _current: &str) -> self_update::Result<Vec<Release>> {
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
#     fn get_latest_releases(&self, _: &str) -> self_update::Result<Vec<self_update::Release>> { unimplemented!() }
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
use crate::update::{Release, ReleaseSource, ReleaseUpdate};

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
            .ok_or_else(|| Error::Config("`source` required".to_string()))?;
        Ok(Box::new(Update {
            source,
            common: self.common.build()?,
        }))
    }
}

/// Updates to a specified or latest release from a user-defined [`ReleaseSource`].
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
    fn get_latest_release(&self) -> Result<Release> {
        self.source.get_latest_release()
    }

    fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>> {
        self.source.get_latest_releases(current_version)
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
            .ok_or_else(|| Error::Config("`source` required".to_string()))?;
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

    impl_async_update_methods!();
}

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> crate::update::sealed::Sealed for AsyncUpdate<S> {}

#[cfg(feature = "async")]
impl_update_config_accessors!(AsyncUpdate<S>, where (S: crate::update::AsyncReleaseSource));

#[cfg(feature = "async")]
impl<S: crate::update::AsyncReleaseSource> crate::update::AsyncFetch for AsyncUpdate<S> {
    async fn get_latest_release_async(&self) -> Result<Release> {
        self.source.get_latest_release().await
    }
    async fn get_latest_releases_async(&self, current_version: &str) -> Result<Vec<Release>> {
        self.source.get_latest_releases(current_version).await
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
pub struct Blocking<S>(pub S);

#[cfg(feature = "async")]
impl<S> Blocking<S> {
    /// Wrap a sync [`ReleaseSource`] so it can drive an [`AsyncUpdate`].
    pub fn new(s: S) -> Self {
        Self(s)
    }
}

#[cfg(feature = "async")]
impl<S: ReleaseSource + Clone + 'static> crate::update::AsyncReleaseSource for Blocking<S> {
    async fn get_latest_release(&self) -> Result<Release> {
        let s = self.0.clone();
        tokio::task::spawn_blocking(move || s.get_latest_release())
            .await
            .map_err(|e| Error::Update(format!("blocking task failed: {e}")))?
    }
    async fn get_latest_releases(&self, current_version: &str) -> Result<Vec<Release>> {
        let s = self.0.clone();
        let current_version = current_version.to_owned();
        tokio::task::spawn_blocking(move || s.get_latest_releases(&current_version))
            .await
            .map_err(|e| Error::Update(format!("blocking task failed: {e}")))?
    }
    async fn get_release_version(&self, ver: &str) -> Result<Release> {
        let s = self.0.clone();
        let ver = ver.to_owned();
        tokio::task::spawn_blocking(move || s.get_release_version(&ver))
            .await
            .map_err(|e| Error::Update(format!("blocking task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::Update;
    use crate::update::{Release, ReleaseAsset, ReleaseSource, ReleaseUpdate};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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
        fn get_latest_releases(&self, _current: &str) -> crate::errors::Result<Vec<Release>> {
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

        let rel = upd.get_latest_release().unwrap();
        assert_eq!(rel.version, "2.0.0");
        assert_eq!(rel.assets.len(), 1);

        let rels = upd.get_latest_releases("1.0.0").unwrap();
        assert_eq!(rels.len(), 1);

        let tagged = upd.get_release_version("1.5.0").unwrap();
        assert_eq!(tagged.version, "1.5.0");

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
    fn selects_asset_from_a_source_release() {
        let upd = configured(Arc::new(AtomicUsize::new(0)));
        let rel = upd.get_latest_release().unwrap();
        // The crate's asset selection runs over the source's release just like any backend.
        let asset = rel
            .asset_for("x86_64-unknown-linux-gnu", None)
            .expect("asset matches the target");
        assert_eq!(asset.download_url, "https://example/app-2.0.0.tar.gz");
    }

    #[cfg(feature = "async")]
    mod async_tests {
        use super::super::{AsyncUpdate, Blocking};
        use crate::update::{AsyncFetch, AsyncReleaseSource, Release, ReleaseAsset, ReleaseSource};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

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
            async fn get_latest_releases(
                &self,
                _current: &str,
            ) -> crate::errors::Result<Vec<Release>> {
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

            let rel = upd.get_latest_release_async().await.unwrap();
            assert_eq!(rel.version, "2.0.0");
            assert_eq!(rel.assets.len(), 1);

            let rels = AsyncFetch::get_latest_releases_async(&upd, "1.0.0")
                .await
                .unwrap();
            assert_eq!(rels.len(), 1);

            let tagged = AsyncFetch::get_release_version_async(&upd, "1.5.0")
                .await
                .unwrap();
            assert_eq!(tagged.version, "1.5.0");

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
            fn get_latest_releases(&self, _current: &str) -> crate::errors::Result<Vec<Release>> {
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
                upd.get_latest_release_async().await.unwrap().version,
                "3.0.0"
            );
            assert_eq!(
                AsyncFetch::get_latest_releases_async(&upd, "1.0.0")
                    .await
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(
                AsyncFetch::get_release_version_async(&upd, "9.9.9")
                    .await
                    .unwrap()
                    .version,
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
            async fn get_latest_releases(
                &self,
                _current: &str,
            ) -> crate::errors::Result<Vec<Release>> {
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
                async fn get_latest_releases(
                    &self,
                    _current: &str,
                ) -> crate::errors::Result<Vec<Release>> {
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
                status.uptodate(),
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
                        s.push(a.name.clone());
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
                matches!(err, crate::errors::Error::Release(_)),
                "no asset selected -> Error::Release, got {:?}",
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
                        s.push(a.name.clone());
                    }
                    None
                });
            let upd = builder.build_async().unwrap();
            let err = upd
                .update_extended_async()
                .await
                .expect_err("matcher returning None must fail the update");
            assert!(
                matches!(err, crate::errors::Error::Release(_)),
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
                async fn get_latest_releases(
                    &self,
                    _current: &str,
                ) -> crate::errors::Result<Vec<Release>> {
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
            assert!(status.uptodate());
        }

        #[tokio::test]
        async fn blocking_adapter_propagates_sync_error() {
            #[derive(Clone)]
            struct FailingSource;
            impl ReleaseSource for FailingSource {
                fn get_latest_release(&self) -> crate::errors::Result<Release> {
                    Err(crate::errors::Error::Release("boom".into()))
                }
                fn get_latest_releases(
                    &self,
                    _current: &str,
                ) -> crate::errors::Result<Vec<Release>> {
                    Err(crate::errors::Error::Network("net".into()))
                }
                fn get_release_version(&self, _ver: &str) -> crate::errors::Result<Release> {
                    Err(crate::errors::Error::Config("cfg".into()))
                }
            }

            let blk = Blocking::new(FailingSource);
            assert!(matches!(
                AsyncReleaseSource::get_latest_release(&blk).await,
                Err(crate::errors::Error::Release(_))
            ));
            assert!(matches!(
                AsyncReleaseSource::get_latest_releases(&blk, "1.0.0").await,
                Err(crate::errors::Error::Network(_))
            ));
            assert!(matches!(
                AsyncReleaseSource::get_release_version(&blk, "1.0.0").await,
                Err(crate::errors::Error::Config(_))
            ));
        }
    }
}
