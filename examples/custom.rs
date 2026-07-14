/*!
Example updating an executable from a custom release source (user-defined backend)

`cargo build --example custom --features "archive-tar archive-zip compression-tar-gz compression-zip-deflate"`

Use this when the built-in backends (github, gitlab, gitea, s3) don't cover your host. Implement
`ReleaseSource` for your host — only `get_releases`, the fetch that says *where releases come
from*, is required (`get_latest_release` / `get_release_version` have default implementations
derived from it) — then configure a `custom::Update` with the same shared options as any other
backend and call `update()`. The crate runs its usual compare -> select-asset -> download ->
verify -> extract -> install flow over your source.

With the `async` feature, implement `AsyncReleaseSource` (or wrap a `Clone` sync source in
`Blocking`) and drive the update through `custom::AsyncUpdate`.

`cargo build --example custom --features "async archive-tar archive-zip compression-tar-gz compression-zip-deflate"`
*/

use self_update::{Release, ReleaseAsset, ReleaseSource, cargo_crate_version};

// ---------------------------------------------------------------------------
// Minimal `ReleaseSource` implementation
//
// In a real application this would make HTTP requests to your artifact host,
// parse the response, and return the discovered releases. Here it returns a
// hard-coded release so the example compiles and runs without network access.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MyHost;

impl ReleaseSource for MyHost {
    // `get_releases` is the only required method: `get_latest_release` and
    // `get_release_version` are derived from it by default. Override those two
    // when your host has cheaper dedicated endpoints (a `/latest` route, a
    // fetch-by-tag lookup).
    fn get_releases(&self) -> self_update::Result<Vec<Release>> {
        // Return the full candidate list. The updater discards releases that are not
        // strictly newer than the current version and picks the newest semver-compatible
        // one, so you do not need to pre-filter by `current`.
        //
        // Build each release with `Release::builder()` (`Release` is `#[non_exhaustive]`
        // and can't be constructed with a struct literal from outside the crate).
        Ok(vec![
            Release::builder()
                .version("1.2.3")
                .asset(ReleaseAsset::new(
                    // The asset name must contain the target triple so the crate's
                    // built-in asset selection can match it against the running platform.
                    "myapp-x86_64-unknown-linux-gnu.tar.gz",
                    "https://releases.example.com/myapp/v1.2.3/myapp-x86_64-unknown-linux-gnu.tar.gz",
                ))
                .build()?,
        ])
    }

    fn get_release_version(&self, ver: &str) -> self_update::Result<Release> {
        // Optional override: the default scans `get_releases()` for an exact version
        // match. In a real implementation this would fetch that exact version from
        // your API; here we build it for demonstration.
        Release::builder()
            .version(ver)
            .asset(ReleaseAsset::new(
                format!("myapp-{ver}-x86_64-unknown-linux-gnu.tar.gz"),
                format!(
                    "https://releases.example.com/myapp/v{ver}/myapp-{ver}-x86_64-unknown-linux-gnu.tar.gz"
                ),
            ))
            .build()
    }
}

// ---------------------------------------------------------------------------
// Async variant (requires `--features async`)
//
// For a natively-async listing transport, implement `AsyncReleaseSource`
// and drive it through `custom::AsyncUpdate`. Alternatively, if you already
// have a `Clone` sync `ReleaseSource`, wrap it in `custom::Blocking` to run
// the sync fetches on `tokio::task::spawn_blocking` — no second impl needed.
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
#[allow(dead_code)]
mod async_update {
    use self_update::backends::custom::{AsyncUpdate, Blocking};
    use self_update::{AsyncReleaseSource, Release, ReleaseAsset};

    // A natively-async source: implement `AsyncReleaseSource` when your listing
    // transport is already async (e.g. you hold a `reqwest::Client`).
    struct MyAsyncHost;

    impl AsyncReleaseSource for MyAsyncHost {
        // As with the sync trait, `get_releases` is the only required method.
        async fn get_releases(&self) -> self_update::Result<Vec<Release>> {
            // ... your own async HTTP request + parsing ...
            Ok(vec![
                Release::builder()
                    .version("1.2.3")
                    .asset(ReleaseAsset::new(
                        "myapp-x86_64-unknown-linux-gnu.tar.gz",
                        "https://releases.example.com/myapp/v1.2.3/myapp-x86_64-unknown-linux-gnu.tar.gz",
                    ))
                    .build()?,
            ])
        }
    }

    pub async fn run_native_async() -> Result<(), Box<dyn std::error::Error>> {
        let status = AsyncUpdate::configure()
            .source(MyAsyncHost)
            .bin_name("myapp")
            .show_download_progress(true)
            //.release_tag("1.3.0")
            //.no_confirm(true)
            //
            // `.timeout()` and `.request_header()` configure the crate-controlled download only;
            // configure your source's listing transport inside `AsyncReleaseSource` instead.
            // `.retries()` has no effect on the custom backend (listing is your source's job).
            .current_version(self_update::cargo_crate_version!())
            .build_async()?
            .update_async()
            .await?;
        println!("Update status: `{}`!", status.version());
        Ok(())
    }

    // Alternative: wrap a `Clone` sync `ReleaseSource` in `Blocking` and drive it
    // through `AsyncUpdate`. The sync fetches run on `tokio::task::spawn_blocking`.
    pub async fn run_blocking_adapter() -> Result<(), Box<dyn std::error::Error>> {
        use super::MyHost;

        let status = AsyncUpdate::configure()
            .source(Blocking::new(MyHost))
            .bin_name("myapp")
            .show_download_progress(true)
            .current_version(self_update::cargo_crate_version!())
            .build_async()?
            .update_async()
            .await?;
        println!("Update status: `{}`!", status.version());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sync update
// ---------------------------------------------------------------------------

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let status = self_update::backends::custom::Update::configure()
        .source(MyHost)
        .bin_name("myapp")
        .show_download_progress(true)
        //.release_tag("1.3.0")
        //.show_output(false)
        //.no_confirm(true)
        //
        // `.timeout()` and `.request_header()` configure only the crate-controlled download;
        // set listing transport options (auth, timeouts, retries) inside your `ReleaseSource`.
        // `.retries()` has no effect on the custom backend (listing is your source's job).
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}

pub fn main() {
    if let Err(e) = run() {
        println!("[ERROR] {}", e);
        ::std::process::exit(1);
    }
}
