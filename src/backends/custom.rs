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
listing is entirely the source's responsibility.

The `async` update API is **not** available on this backend: [`ReleaseSource`] is sync-only and
`UpdateBuilder` has no `build_async()`. From a `tokio` application, drive a custom update from a
blocking context with [`tokio::task::spawn_blocking`].

There is also no `custom::ReleaseList` (unlike the built-in backends): release listing is entirely
your [`ReleaseSource`]'s job, so query it directly instead.
*/

use std::sync::Arc;

use crate::backends::common::{CommonBuilderConfig, CommonConfig};
use crate::errors::*;
use crate::update::{Release, ReleaseSource, ReleaseUpdate};

/// `custom::Update` builder.
#[must_use]
#[derive(Default)]
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

    impl_release_update_accessors!();
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
}
