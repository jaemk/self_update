/*!

[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Supported backends: **GitHub**, **GitLab**, **Gitea**, and **S3** (Amazon S3, Google GCS,
DigitalOcean Spaces, or any S3-compatible endpoint). Each exposes the same `Update`
(configure → build → update) and `ReleaseList` builder API.

> **Upgrading from 0.x?** 1.0 makes a focused set of breaking changes to clean up the public
> API. See the [1.0 migration guide](https://github.com/jaemk/self_update/blob/master/docs/migrations/0.x-to-1.0-human.md)
> for a step-by-step walkthrough, or the
> [agent-oriented guide](https://github.com/jaemk/self_update/blob/master/docs/migrations/0.x-to-1.0.md)
> for automated migration tooling.

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`.
Note, the [`trust`](https://github.com/japaric/trust) project provides a nice setup for
producing release-builds via CI (travis/appveyor).

### Features

Exactly **one** HTTP client and **one** TLS backend must be selected (they are mutually
exclusive — enabling both, or neither, is a compile error):

* `reqwest` (default): use the [`reqwest`](https://docs.rs/reqwest) HTTP client;
* `ureq`: use the [`ureq`](https://docs.rs/ureq) HTTP client instead (set `default-features = false`);
* `default-tls` (default): native TLS for the selected client;
* `rustls`: use a [pure rust TLS implementation](https://github.com/rustls/rustls) instead. This feature does _not_ support 32bit macOS.

The following optional [cargo features](https://doc.rust-lang.org/cargo/reference/manifest.html#the-features-section)
are _disabled_ by default; activate the one(s) your release files need:

* `archive-tar`: Support for _tar_ archive format;
* `archive-zip`: Support for _zip_ archive format;
* `compression-flate2`: Support for _gzip_ compression;
* `compression-zip-deflate`: Support for _zip_'s _deflate_ compression format;
* `compression-zip-bzip2`: Support for _zip_'s _bzip2_ compression format;
* `signatures`: Use [zipsign](https://github.com/Kijewski/zipsign) to verify `.zip` and `.tar.gz` artifacts. Artifacts are assumed to have been signed using zipsign;
* `checksums`: Verify a downloaded artifact against a known SHA-256/SHA-512 checksum (e.g. from a `SHA256SUMS` file) before installing it;
* `s3-auth`: Sign S3 requests (AWS SigV4) to update from private buckets via the S3 backend;
* `async`: Add async (`*_async`) update methods alongside the unchanged blocking API. tokio-only and reqwest-only (incompatible with `ureq`); see [Async](#async) below.

The S3 backend needs **no feature** — it is always compiled. (A no-op `s3` alias feature exists only so `features = ["s3"]` resolves for symmetry with the other backends; only private-bucket request signing needs an actual feature, `s3-auth`.)

### Example

Run the following example to see `self_update` in action:

`cargo run --example github --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`.

There are equivalent examples for the other backends (`gitlab`, `gitea`, `s3`), e.g.:

`cargo run --example gitlab --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`.

which runs something roughly equivalent to:

```rust
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
```

Amazon S3, Google GCS, and DigitalOcean Spaces, as well as any S3 compatible server are also supported
through the `S3` backend to check for new releases.  Provided a `bucket_name`
and `asset_prefix` string, `self_update` will look up all matching files using the following format
as a convention for the filenames: `[directory/]<asset name>-<semver>-<platform/target>.<extension>`.
Leading directories will be stripped from the file name allowing the use of subdirectories in the S3 bucket,
and any file not matching the format, or not matching the provided prefix string, will be ignored.

```rust
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<dyn ::std::error::Error>> {
    let status = self_update::backends::s3::Update::configure()
        // .end_point(self_update::backends::s3::EndPoint::GCS)
        // .end_point("https://s3.example.com")
        .bucket_name("self_update_releases")
        .asset_prefix("something/self_update")
        .region("eu-west-2")
        .bin_name("self_update_example")
        // To authenticate (requires the `s3-auth` feature), read the credentials at
        // runtime rather than baking them into the binary with `env!`:
        // .access_key((std::env::var("AWS_ACCESS_KEY_ID")?, std::env::var("AWS_SECRET_ACCESS_KEY")?))
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("S3 Update status: `{}`!", status.version());
    Ok(())
}
```

Separate utilities are also exposed (**NOTE**: the following example extracts a `.tar.gz`, which
_requires_ both the `archive-tar` and `compression-flate2` features -- `archive-tar` reads the tar
archive and `compression-flate2` decodes the gzip layer; see the [features](#features) section
above). It downloads, extracts, and replaces the running binary
by hand; the staging directory and the in-place replacement use the [`tempfile`](https://crates.io/crates/tempfile)
and [`self_replace`](https://crates.io/crates/self-replace) crates, which you add as your own dependencies
(they are no longer re-exported from `self_update`):

```rust
# #[cfg(feature = "archive-tar")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    // get the first available release
    let asset = releases[0]
        .asset_for(&self_update::get_target(), None)
        .unwrap();

    let tmp_dir = tempfile::Builder::new()
            .prefix("self_update")
            .tempdir_in(::std::env::current_dir()?)?;
    let tmp_tarball_path = tmp_dir.path().join(&asset.name);
    let tmp_tarball = ::std::fs::File::create(&tmp_tarball_path)?;

    self_update::Download::from_url(&asset.download_url)
        .header(self_update::http::header::ACCEPT, "application/octet-stream".parse()?)
        .download_to(&tmp_tarball)?;

    let bin_name = std::path::PathBuf::from("self_update_bin");
    self_update::Extract::from_source(&tmp_tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_file(&tmp_dir.path(), &bin_name)?;

    let new_exe = tmp_dir.path().join(bin_name);
    self_replace::self_replace(new_exe)?;

    Ok(())
}
```

### Multi-file / non-executable install

The high-level `update()` flow replaces a single executable. To update a tool that ships **more
than one file** (a binary plus sidecar libraries/resources), or to install files that aren't the
running executable, download and extract the whole archive yourself and then install the files
with `MoveAll`, which applies a set of `(source -> dest)` moves **transactionally**: either every
move succeeds, or — on the first failure — all already-applied moves are rolled back, so a failed
update can't leave a half-installed tool. Because it uses `rename` (which can't cross
filesystems), the source files, every destination, and the temp dir must all be on the same
filesystem.

**NOTE**: this example extracts a `.tar.gz`, which requires both the `archive-tar` and
`compression-flate2` features.

```rust
# #[cfg(all(feature = "archive-tar", feature = "compression-flate2"))]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let tmp_dir = tempfile::TempDir::new()?;
    let tarball_path = tmp_dir.path().join("release.tar.gz");
    // ... download the archive to `tarball_path` (see the example above) ...

    // The extracted files are renamed into place, so the staging dir (the move sources) and the
    // stash dir must be on the same filesystem as the destinations — create both next to them
    // rather than in $TMPDIR.
    let staging = tempfile::TempDir::new_in("/usr/local")?;
    self_update::Extract::from_source(&tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_into(staging.path())?;

    // Install several files atomically (all-or-nothing).
    let stash = tempfile::TempDir::new_in("/usr/local")?;
    self_update::MoveAll::from_temp(stash.path())
        .add(staging.path().join("app"), "/usr/local/bin/app")
        .add(staging.path().join("libapp.so"), "/usr/local/lib/libapp.so")
        .commit()?;
    Ok(())
}
```

### Checksum verification

With the `checksums` feature, pass a known digest (e.g. one published in a `SHA256SUMS` file
alongside the release) and the crate verifies the downloaded artifact against it **before**
installing — a mismatch aborts the update. The algorithm is chosen by the
`Checksum` variant (`Sha256` / `Sha512`); it complements the `signatures`
feature (zipsign), which verifies authenticity rather than a published digest.

```rust
# #[cfg(feature = "checksums")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        // hex digest, obtained out of band (e.g. parsed from the release's SHA256SUMS)
        .checksum(self_update::Checksum::Sha256("abc123…".into()))
        .build()?
        .update()?;
    Ok(())
}
```

### Custom backends

To update from a host the built-in backends (`github`, `gitlab`, `gitea`, `s3`) don't cover —
another forge, a private artifact registry, a plain HTTP directory — implement the
`ReleaseSource` trait (three fetch methods that say *where releases come from*) and drive a full
update through the `backends::custom` backend, which reuses the crate's compare → select-asset →
download → verify → extract → install flow. You build `Release`s with `Release::builder` and
`ReleaseAsset::new`; the `ReleaseUpdate` trait stays sealed.

`ReleaseSource` is **synchronous**. For a natively-async source, implement `AsyncReleaseSource`
(the same three fetches as `async fn`) and drive it through
`backends::custom::AsyncUpdate` + `build_async()`; to reuse a
`Clone` sync source from the async API, wrap it in
`backends::custom::Blocking`.

```rust
use self_update::{Release, ReleaseAsset, ReleaseSource, cargo_crate_version};

struct MyHost;
impl ReleaseSource for MyHost {
    fn get_latest_release(&self) -> self_update::Result<Release> {
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

fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::custom::Update::configure()
        .source(MyHost)
        .bin_name("app")
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("custom backend update status: `{}`!", status.version());
    Ok(())
}
```

### Async

With the `async` feature, every built-in backend's `Update` builder gains a `build_async()` that
returns a concrete `Update` with async (`*_async`) verbs — `update_async()`,
`update_extended_async()`, and `get_latest_release_async()` — so a `tokio` application can update
without wrapping the blocking calls in `spawn_blocking`. The blocking API is unchanged; the async
path is purely additive. It is **tokio-only and reqwest-only** (ureq has no async story, so `async`
is incompatible with `ureq`). Network IO becomes async; the extract/replace step stays synchronous.

```rust
# #[cfg(feature = "async")]
async fn update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .build_async()?
        .update_async()
        .await?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
```

### Custom HTTP client

The `.timeout()` / `.request_header()` / `.retries()` builder knobs cover most transport needs, but
for full control — custom TLS roots / mTLS, connection pooling, redirect policy, proxy-with-auth, or
simply reusing your application's existing client — you can hand the crate a **pre-built client**.
It is used for both the release listing and the download. The setters are client-specific (the
client types differ and are mutually exclusive): `reqwest_client` (a blocking
`reqwest::blocking::Client`, used by the blocking API), `reqwest_async_client`
(an async `reqwest::Client`, used by the `*_async` verbs), and `ureq_agent` (a
`ureq::Agent`). The selected client crate is re-exported (`self_update::reqwest` /
`self_update::ureq`) so you don't need a separate dependency to name the type.

When you inject a client, `.request_header()` still applies, and `.retries()` still applies to the
release-listing requests (the download is never retried), and for `reqwest` the per-request
`.timeout()` is layered on too; but `HTTP(S)_PROXY` env and the crate's TLS feature are left entirely
to your client (and a `ureq::Agent` owns its own timeout, so `.timeout()` does not apply to an
injected agent — configure it on the agent). `reqwest_client` feeds the sync verbs and
`reqwest_async_client` the async ones — injecting only one and calling the other half just uses the
crate's per-call client for that half.

```rust
# #[cfg(feature = "reqwest")]
fn update() -> Result<(), Box<dyn std::error::Error>> {
    let client = self_update::reqwest::blocking::Client::builder()
        // .add_root_certificate(...) / .proxy(...) / .danger_accept_invalid_certs(...) etc.
        .build()?;
    self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .current_version(self_update::cargo_crate_version!())
        .reqwest_client(client)
        .build()?
        .update()?;
    Ok(())
}
```

### Troubleshooting

When using cross compilation tools such as cross if you want to use rustls and not openssl

```toml
self_update = { version = "1", features = ["rustls"], default-features = false }
```

**TLS certificate errors on Linux (`default-tls` / OpenSSL).** With the native-TLS backend,
OpenSSL finds the system CA bundle on its own on most distributions. In a minimal environment where
it can't (some containers, `musl` static builds, or a non-standard cert layout) a request may fail
with a certificate-verification error. Point OpenSSL at the bundle by exporting `SSL_CERT_FILE`
(and, if needed, `SSL_CERT_DIR`) before running your program — the paths vary by distribution, e.g.
on a Debian/Ubuntu base:

```bash
export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
export SSL_CERT_DIR=/etc/ssl/certs
```

Alternatively build with the `rustls` feature, which uses a bundled root store and does not depend
on the system OpenSSL cert layout.

*/

// Exactly one HTTP client must be selected. Surface a human-readable diagnostic instead of
// the raw symbol collision (both) or undefined-item error (neither) that would otherwise occur.
#[cfg(all(feature = "reqwest", feature = "ureq"))]
compile_error!(
    "features `reqwest` and `ureq` are mutually exclusive — enable exactly one HTTP client \
     (for `ureq`, set `default-features = false`)"
);
#[cfg(not(any(feature = "reqwest", feature = "ureq")))]
compile_error!(
    "no HTTP client selected — enable exactly one of the `reqwest` (default) or `ureq` features"
);

// The TLS backend is also a single choice; enabling both forwards conflicting TLS features to
// the selected client.
#[cfg(all(feature = "default-tls", feature = "rustls"))]
compile_error!(
    "features `default-tls` and `rustls` are mutually exclusive — to use `rustls`, set \
     `default-features = false`"
);

// The async API is reqwest-only — ureq has no async story.
#[cfg(all(feature = "async", feature = "ureq"))]
compile_error!(
    "feature `async` requires the `reqwest` client and is incompatible with `ureq` — \
     `ureq` has no async API"
);

pub use http;
// Re-export the selected HTTP client so callers can name the types accepted by the client-injection
// setters (`reqwest_client` / `reqwest_async_client` / `ureq_agent`) without a separate dependency.
#[cfg(feature = "reqwest")]
pub use reqwest;
#[cfg(feature = "async")]
pub use update::AsyncReleaseSource;
pub use update::{
    Release, ReleaseAsset, ReleaseBuilder, ReleaseSource, ReleaseUpdate, UpdateConfig, UpdateStatus,
};
#[cfg(feature = "ureq")]
pub use ureq;

/// Re-export of the [`zipsign_api`] crate, whose [`PUBLIC_KEY_LENGTH`] constant defines the
/// size of the ed25519 verifying keys accepted by the `verifying_keys` builder methods.
///
/// [`PUBLIC_KEY_LENGTH`]: zipsign_api::PUBLIC_KEY_LENGTH
#[cfg(feature = "signatures")]
pub use zipsign_api;

/// An ed25519ph verifying key used to validate a signed download (see the `signatures` feature).
///
/// This is a convenience alias for the fixed-size key array accepted by the `verifying_keys`
/// builder methods, so consumers don't need to depend on `zipsign-api` directly.
#[cfg(feature = "signatures")]
pub type VerifyingKey = [u8; zipsign_api::PUBLIC_KEY_LENGTH];

#[cfg(feature = "compression-flate2")]
use either::Either;
use indicatif::{ProgressBar, ProgressStyle};
use log::debug;
use std::cmp::min;
use std::fs;
use std::io;
use std::path;

#[macro_use]
mod macros;
pub mod backends;
#[cfg(feature = "checksums")]
mod checksum;
pub mod errors;
mod http_client;
pub mod update;
pub mod version;

/// Re-export the crate's [`Error`](errors::Error) and [`Result`](errors::Result) at the crate root,
/// so consumers (and `ReleaseSource` implementors) can write `self_update::Result<T>` /
/// `self_update::Error` without naming the `errors` module.
pub use errors::{Error, Result};

#[cfg(feature = "checksums")]
pub use checksum::Checksum;

use http_client::{header, HttpResponse};

pub const DEFAULT_PROGRESS_TEMPLATE: &str =
    "[{elapsed_precise}] [{bar:40}] {bytes}/{total_bytes} ({eta}) {msg}";
pub const DEFAULT_PROGRESS_CHARS: &str = "=>-";

/// Get the current target triple.
///
/// Returns a target triple (e.g. `x86_64-unknown-linux-gnu` or `i686-pc-windows-msvc`)
pub fn get_target() -> &'static str {
    env!("TARGET")
}

/// Flush a message to stdout and check if they respond `yes`.
/// Interprets a blank response as yes.
///
/// * Errors:
///     * Io flushing
///     * User entered anything other than enter/Y/y
fn confirm(msg: &str) -> Result<()> {
    print_flush!("{}", msg);

    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    let s = s.trim().to_lowercase();
    if !s.is_empty() && s != "y" {
        bail!(Error::Update, "Update aborted");
    }
    Ok(())
}

/// Status returned after updating
///
/// Wrapped `String`s are version tags
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Status {
    UpToDate(String),
    Updated(String),
}
impl Status {
    /// Return the version tag
    pub fn version(&self) -> &str {
        use Status::*;
        match *self {
            UpToDate(ref s) => s,
            Updated(ref s) => s,
        }
    }

    /// Returns `true` if `Status::UpToDate`
    pub fn uptodate(&self) -> bool {
        matches!(*self, Status::UpToDate(_))
    }

    /// Returns `true` if `Status::Updated`
    pub fn updated(&self) -> bool {
        matches!(*self, Status::Updated(_))
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Status::*;
        match *self {
            UpToDate(ref s) => write!(f, "UpToDate({})", s),
            Updated(ref s) => write!(f, "Updated({})", s),
        }
    }
}

/// Supported archive formats
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ArchiveKind {
    #[cfg(feature = "archive-tar")]
    Tar(Option<Compression>),
    Plain(Option<Compression>),
    #[cfg(feature = "archive-zip")]
    Zip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Compression {
    Gz,
}

fn detect_archive(path: &path::Path) -> Result<ArchiveKind> {
    let ext = path.extension();

    debug!("Detecting archive type using extension: {:?}", ext);

    let res = match ext {
        Some(extension) if extension == std::ffi::OsStr::new("zip") => {
            #[cfg(feature = "archive-zip")]
            {
                debug!("Detected .zip archive");
                Ok(ArchiveKind::Zip)
            }
            #[cfg(not(feature = "archive-zip"))]
            {
                Err(Error::ArchiveNotEnabled("zip".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("tar") => {
            #[cfg(feature = "archive-tar")]
            {
                debug!("Detected .tar archive");
                Ok(ArchiveKind::Tar(None))
            }
            #[cfg(not(feature = "archive-tar"))]
            {
                Err(Error::ArchiveNotEnabled("tar".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("tgz") => {
            #[cfg(feature = "archive-tar")]
            {
                debug!("Detected .tgz archive");
                Ok(ArchiveKind::Tar(Some(Compression::Gz)))
            }
            #[cfg(not(feature = "archive-tar"))]
            {
                Err(Error::ArchiveNotEnabled("tar".to_string()))
            }
        }
        Some(extension) if extension == std::ffi::OsStr::new("gz") => match path
            .file_stem()
            .map(path::Path::new)
            .and_then(|f| f.extension())
        {
            Some(extension) if extension == std::ffi::OsStr::new("tar") => {
                #[cfg(feature = "archive-tar")]
                {
                    debug!("Detected .tar.gz archive");
                    Ok(ArchiveKind::Tar(Some(Compression::Gz)))
                }
                #[cfg(not(feature = "archive-tar"))]
                {
                    Err(Error::ArchiveNotEnabled("tar".to_string()))
                }
            }
            _ => Ok(ArchiveKind::Plain(Some(Compression::Gz))),
        },
        _ => Ok(ArchiveKind::Plain(None)),
    };

    debug!("Detected archive type: {:?}", res);

    res
}

/// Extract contents of an encoded archive (e.g. tar.gz) file to a specified directory
///
/// * Errors:
///     * Io - opening files
///     * Io - gzip decoding
///     * Io - archive unpacking
#[derive(Debug)]
pub struct Extract<'a> {
    source: &'a path::Path,
    archive: Option<ArchiveKind>,
}
#[cfg(feature = "compression-flate2")]
type GetArchiveReaderResult = Either<fs::File, flate2::read::GzDecoder<fs::File>>;
#[cfg(not(feature = "compression-flate2"))]
type GetArchiveReaderResult = fs::File;

impl<'a> Extract<'a> {
    /// Create an `Extract`or from a source path
    pub fn from_source(source: &'a path::Path) -> Extract<'a> {
        Self {
            source,
            archive: None,
        }
    }

    /// Specify an archive format of the source being extracted. If not specified, the
    /// archive format will determined from the file extension.
    pub fn archive(&mut self, kind: ArchiveKind) -> &mut Self {
        self.archive = Some(kind);
        self
    }

    #[allow(unused_variables)]
    fn get_archive_reader(
        source: fs::File,
        compression: Option<Compression>,
    ) -> GetArchiveReaderResult {
        #[cfg(feature = "compression-flate2")]
        match compression {
            Some(Compression::Gz) => Either::Right(flate2::read::GzDecoder::new(source)),
            None => Either::Left(source),
        }
        #[cfg(not(feature = "compression-flate2"))]
        source
    }

    /// Extract an entire source archive into a specified path. If the source is a single compressed
    /// file and not an archive, it will be extracted into a file with the same name inside of
    /// `into_dir`.
    pub fn extract_into(&self, into_dir: &path::Path) -> Result<()> {
        let source = fs::File::open(self.source)?;
        let archive = match self.archive {
            Some(archive) => archive,
            None => detect_archive(self.source)?,
        };

        // We cannot use a feature flag in a match arm. To bypass this the code block is
        // isolated in a closure and called accordingly.
        let extract_into_plain_or_tar = |source: fs::File, compression: Option<Compression>| {
            let mut reader = Self::get_archive_reader(source, compression);

            match archive {
                ArchiveKind::Plain(_) => {
                    match fs::create_dir_all(into_dir) {
                        Ok(_) => (),
                        Err(e) => {
                            if e.kind() != io::ErrorKind::AlreadyExists {
                                return Err(Error::Io(e));
                            }
                        }
                    }
                    let file_name = self
                        .source
                        .file_name()
                        .ok_or_else(|| Error::Update("Extractor source has no file-name".into()))?;
                    let mut out_path = into_dir.join(file_name);
                    out_path.set_extension("");
                    let mut out_file = fs::File::create(&out_path)?;
                    io::copy(&mut reader, &mut out_file)?;
                }
                #[cfg(feature = "archive-tar")]
                ArchiveKind::Tar(_) => {
                    let mut archive = tar::Archive::new(reader);
                    archive.unpack(into_dir)?;
                }
                #[allow(unreachable_patterns)]
                _ => unreachable!(
                    "detect_archive() returns in case the proper feature flag is not enabled"
                ),
            };

            Ok(())
        };

        match archive {
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
                extract_into_plain_or_tar(source, compression)?;
            }
            #[cfg(not(feature = "archive-tar"))]
            ArchiveKind::Plain(compression) => {
                extract_into_plain_or_tar(source, compression)?;
            }
            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;

                    let output_path = into_dir.join(file.name());
                    if let Some(parent_dir) = output_path.parent() {
                        if let Err(e) = fs::create_dir_all(parent_dir) {
                            if e.kind() != io::ErrorKind::AlreadyExists {
                                return Err(Error::Io(e));
                            }
                        }
                    }

                    let mut output = fs::File::create(output_path)?;
                    io::copy(&mut file, &mut output)?;
                }
            }
        };
        Ok(())
    }

    /// Extract a single file from a source and save to a file of the same name in `into_dir`.
    /// If the source is a single compressed file, it will be saved with the name `file_to_extract`
    /// in the specified `into_dir`.
    pub fn extract_file<T: AsRef<path::Path>>(
        &self,
        into_dir: &path::Path,
        file_to_extract: T,
    ) -> Result<()> {
        let file_to_extract = file_to_extract.as_ref();
        let source = fs::File::open(self.source)?;
        let archive = match self.archive {
            Some(archive) => archive,
            None => detect_archive(self.source)?,
        };

        debug!(
            "Attempting to extract {:?} file from {:?}",
            file_to_extract, self.source
        );

        // We cannot use a feature flag in a match arm. To bypass this the code block is
        // isolated in a closure and called accordingly.
        let extract_file_plain_or_tar = |source: fs::File, compression: Option<Compression>| {
            let mut reader = Self::get_archive_reader(source, compression);

            match archive {
                ArchiveKind::Plain(_) => {
                    debug!("Copying file directly");
                    match fs::create_dir_all(into_dir) {
                        Ok(_) => (),
                        Err(e) => {
                            if e.kind() != io::ErrorKind::AlreadyExists {
                                return Err(Error::Io(e));
                            }
                        }
                    }
                    let file_name = file_to_extract
                        .file_name()
                        .ok_or_else(|| Error::Update("Extractor source has no file-name".into()))?;
                    let out_path = into_dir.join(file_name);
                    let mut out_file = fs::File::create(out_path)?;
                    io::copy(&mut reader, &mut out_file)?;
                }
                #[cfg(feature = "archive-tar")]
                ArchiveKind::Tar(_) => {
                    debug!("Extracting from tar");

                    let mut archive = tar::Archive::new(reader);
                    let mut entry = archive
                        .entries()?
                        .filter_map(|e| e.ok())
                        .find(|e| {
                            let p = e.path();
                            debug!("Archive path: {:?}", p);
                            p.ok().filter(|p| p == file_to_extract).is_some()
                        })
                        .ok_or_else(|| {
                            Error::Update(format!(
                                "Could not find the required path in the archive: {:?}",
                                file_to_extract
                            ))
                        })?;
                    entry.unpack_in(into_dir)?;
                }
                #[allow(unreachable_patterns)]
                _ => unreachable!(
                    "detect_archive() returns in case the proper feature flag is not enabled"
                ),
            };

            Ok(())
        };

        match archive {
            #[cfg(feature = "archive-tar")]
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
                extract_file_plain_or_tar(source, compression)?;
            }
            #[cfg(not(feature = "archive-tar"))]
            ArchiveKind::Plain(compression) => {
                extract_file_plain_or_tar(source, compression)?;
            }
            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;
                let file_name = file_to_extract.to_str().ok_or_else(|| {
                    Error::Update(format!(
                        "cannot extract file with a non-UTF-8 path: {:?}",
                        file_to_extract
                    ))
                })?;
                let mut file = archive.by_name(file_name)?;

                let output_path = into_dir.join(file.name());
                if let Some(parent_dir) = output_path.parent() {
                    if let Err(e) = fs::create_dir_all(parent_dir) {
                        if e.kind() != io::ErrorKind::AlreadyExists {
                            return Err(Error::Io(e));
                        }
                    }
                }

                let mut output = fs::File::create(output_path)?;
                io::copy(&mut file, &mut output)?;
            }
        };
        Ok(())
    }
}

/// Moves a file from the given path to the specified destination.
///
/// `source` and `dest` must be on the same filesystem.
/// If `replace_using_temp` is specified, the destination file will be
/// replaced using the given temporary path.
/// If the existing `dest` file is a currently running long running program,
/// `replace_using_temp` may run into errors cleaning up the temp dir.
/// If that's the case for your use-case, consider not specifying a temp dir to use.
///
/// * Errors:
///     * Io - copying / renaming
#[derive(Debug)]
pub struct Move<'a> {
    source: &'a path::Path,
    temp: Option<&'a path::Path>,
}
impl<'a> Move<'a> {
    /// Specify source file
    pub fn from_source(source: &'a path::Path) -> Move<'a> {
        Self { source, temp: None }
    }

    /// If specified and the destination file already exists, the "destination"
    /// file will be moved to the given temporary location before the "source"
    /// file is moved to the "destination" file.
    ///
    /// In the event of an `io` error while renaming "source" to "destination",
    /// the temporary file will be moved back to "destination".
    ///
    /// The `temp` dir must be explicitly provided since `rename` operations require
    /// files to live on the same filesystem.
    pub fn replace_using_temp(&mut self, temp: &'a path::Path) -> &mut Self {
        self.temp = Some(temp);
        self
    }

    /// Move source file to specified destination
    pub fn to_dest(&self, dest: &path::Path) -> Result<()> {
        match self.temp {
            // Move the existing dest to a temp location so we can move it back if
            // there's an error. If the existing `dest` file is a long running program,
            // this may prevent the temp dir from being cleaned up.
            Some(temp) if dest.exists() => {
                fs::rename(dest, temp)?;
                if let Err(e) = fs::rename(self.source, dest) {
                    fs::rename(temp, dest)?;
                    return Err(Error::from(e));
                }
            }
            // No temp set, or nothing to preserve at `dest`: just move source into place.
            _ => {
                fs::rename(self.source, dest)?;
            }
        };
        Ok(())
    }
}

/// Transactionally install a *set* of files: either every `(source -> dest)` move is applied, or
/// — on the first failure — all already-applied moves are rolled back, restoring every
/// destination to its prior contents. Use it to update a tool that ships more than one file (a
/// binary plus sidecar libraries/resources) without risking a half-applied update.
///
/// This is the multi-file analogue of [`Move`]. It relies on `rename`, so **every source, every
/// destination, and the `temp` directory must live on the same filesystem** (the same constraint
/// [`Move::replace_using_temp`] has) — in particular the staging dir holding the files you `add`
/// must be co-located with the destinations, not in `$TMPDIR`. The `temp` directory is used to
/// stash each displaced destination so it can be restored on rollback; a [`tempfile::TempDir`] is a
/// convenient choice.
///
/// [`commit`](Self::commit) drains the queued moves as it applies them, so a `MoveAll` is
/// single-use: a second `commit` has nothing left to do and is a no-op returning `Ok(())`. Rollback
/// is best-effort — if a rollback step itself fails it is logged via `log::error!` rather than
/// surfaced, and the error returned to the caller is always the original one that triggered the
/// rollback.
///
/// ```no_run
/// # fn run() -> Result<(), Box<dyn std::error::Error>> {
/// // The stash dir must be on the same filesystem as the destinations (rename can't cross
/// // filesystems), so create it next to them rather than in $TMPDIR.
/// let tmp = tempfile::TempDir::new_in("/usr/local")?;
/// // `new_bin` / `new_lib` are files you already extracted into a temp dir.
/// let new_bin = std::path::Path::new("/tmp/extracted/app");
/// let new_lib = std::path::Path::new("/tmp/extracted/libapp.so");
/// self_update::MoveAll::from_temp(tmp.path())
///     .add(new_bin, "/usr/local/bin/app")
///     .add(new_lib, "/usr/local/lib/libapp.so")
///     .commit()?; // all-or-nothing
/// # Ok(())
/// # }
/// ```
///
/// * Errors:
///     * Io - renaming a source into place or stashing an existing destination
#[derive(Debug)]
#[must_use = "queued moves are only applied when `.commit()` is called"]
pub struct MoveAll<'a> {
    temp: &'a path::Path,
    moves: Vec<(path::PathBuf, path::PathBuf)>,
}

impl<'a> MoveAll<'a> {
    /// Start a transactional install, stashing displaced destinations under `temp` so they can be
    /// restored if a later move fails. `temp` must be on the same filesystem as every destination.
    pub fn from_temp(temp: &'a path::Path) -> Self {
        Self {
            temp,
            moves: Vec::new(),
        }
    }

    /// Queue a `source -> dest` move. Moves are applied by [`commit`](Self::commit) in the order
    /// added.
    pub fn add(
        &mut self,
        source: impl AsRef<path::Path>,
        dest: impl AsRef<path::Path>,
    ) -> &mut Self {
        self.moves
            .push((source.as_ref().to_path_buf(), dest.as_ref().to_path_buf()));
        self
    }

    /// Apply every queued move. On success all destinations have been replaced. On the first
    /// failure, every already-applied move (and the failing one's partial state) is rolled back so
    /// each destination is left with its original contents, and the underlying error is returned.
    ///
    /// The queued moves are drained as they are applied, so calling `commit` again is a no-op that
    /// returns `Ok(())`.
    pub fn commit(&mut self) -> Result<()> {
        // Drain the queue so a second `commit` is a no-op rather than re-running already-applied
        // moves against now-missing sources.
        let moves = std::mem::take(&mut self.moves);

        // For each applied move we remember the destination and where its previous contents (if
        // any) were stashed, so a later failure can restore them in reverse order.
        let mut applied: Vec<Applied> = Vec::with_capacity(moves.len());

        for (i, (source, dest)) in moves.iter().enumerate() {
            // Stash an existing destination so we can move it back on rollback.
            let stash = if dest.exists() {
                let stash = self.temp.join(format!("self_update-stash-{i}"));
                if let Err(e) = fs::rename(dest, &stash) {
                    rollback(&applied);
                    return Err(Error::from(e));
                }
                Some(stash)
            } else {
                None
            };

            // Move the new file into place.
            if let Err(e) = fs::rename(source, dest) {
                // Undo this step's stash before rolling back the earlier ones.
                if let Some(stash) = &stash {
                    if let Err(restore_err) = fs::rename(stash, dest) {
                        log::error!(
                            "failed to restore {:?} from stash {:?} during rollback: {}",
                            dest,
                            stash,
                            restore_err
                        );
                    }
                }
                rollback(&applied);
                return Err(Error::from(e));
            }

            applied.push(Applied {
                dest: dest.clone(),
                stash,
            });
        }

        Ok(())
    }
}

/// A move that [`MoveAll::commit`] has applied, retained so it can be undone on a later failure.
#[derive(Debug)]
struct Applied {
    dest: path::PathBuf,
    stash: Option<path::PathBuf>,
}

/// Best-effort rollback of already-applied moves, in reverse order. For a destination that
/// previously existed, the stashed original is `rename`d back over the newly installed file — a
/// single atomic replace (the same technique [`Move::replace_using_temp`] uses), so the original
/// is never deleted before its restore can fail. For a destination that didn't previously exist
/// (a fresh install), the newly installed file is simply removed. Rollback failures are logged
/// rather than propagated — the original error that triggered the rollback is what callers see.
fn rollback(applied: &[Applied]) {
    for entry in applied.iter().rev() {
        match &entry.stash {
            // Previously existed: atomically restore the original over the new file.
            Some(stash) => {
                if let Err(e) = fs::rename(stash, &entry.dest) {
                    log::error!(
                        "failed to restore {:?} from stash {:?} during rollback: {}",
                        entry.dest,
                        stash,
                        e
                    );
                }
            }
            // Fresh install (nothing to restore): remove the file we added.
            None => {
                if let Err(e) = fs::remove_file(&entry.dest) {
                    log::error!("failed to remove {:?} during rollback: {}", entry.dest, e);
                }
            }
        }
    }
}

/// A download-progress callback: `(bytes_downloaded_so_far, total_bytes_if_known)`.
pub(crate) type DynProgressFn = dyn Fn(u64, Option<u64>) + Send + Sync;

/// Wrapper around a [`DynProgressFn`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct ProgressCallback(pub(crate) std::sync::Arc<DynProgressFn>);

impl std::fmt::Debug for ProgressCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressCallback(..)")
    }
}

/// A post-update verification callback: given the path to the freshly-extracted binary (before
/// it is installed), returns `true` to accept it or `false` to reject it (aborting the update).
pub(crate) type DynVerifyFn = dyn Fn(&std::path::Path) -> bool + Send + Sync;

/// Wrapper around a [`DynVerifyFn`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct VerifyCallback(pub(crate) std::sync::Arc<DynVerifyFn>);

impl std::fmt::Debug for VerifyCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifyCallback(..)")
    }
}

/// A custom asset-selection callback: given the release's assets, returns the asset to download
/// (or `None` to fail the update). Overrides the built-in target/identifier substring matching.
pub(crate) type DynAssetMatcher = dyn Fn(&[ReleaseAsset]) -> Option<ReleaseAsset> + Send + Sync;

/// Wrapper around a [`DynAssetMatcher`] so structs holding one can still derive `Clone`/`Debug`.
#[derive(Clone)]
pub(crate) struct AssetMatcher(pub(crate) std::sync::Arc<DynAssetMatcher>);

impl std::fmt::Debug for AssetMatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssetMatcher(..)")
    }
}

/// Download things into files
///
/// With optional progress bar
#[derive(Debug)]
pub struct Download {
    show_progress: bool,
    url: String,
    headers: http_client::header::HeaderMap,
    progress_template: String,
    progress_chars: String,
    timeout: Option<std::time::Duration>,
    on_progress: Option<ProgressCallback>,
    client: http_client::ClientOverride,
}
impl Download {
    /// Specify download url
    pub fn from_url(url: &str) -> Self {
        Self {
            show_progress: false,
            url: url.to_owned(),
            headers: http_client::header::HeaderMap::new(),
            progress_template: DEFAULT_PROGRESS_TEMPLATE.to_string(),
            progress_chars: DEFAULT_PROGRESS_CHARS.to_string(),
            timeout: None,
            on_progress: None,
            client: http_client::ClientOverride::default(),
        }
    }

    /// Toggle the download progress bar. Named to match the `Update` builder's setter of the same
    /// name.
    #[doc(alias = "show_progress")]
    pub fn show_download_progress(&mut self, b: bool) -> &mut Self {
        self.show_progress = b;
        self
    }

    /// Set a timeout for the download request. Defaults to no timeout.
    #[doc(alias = "set_timeout")]
    pub fn timeout(&mut self, timeout: std::time::Duration) -> &mut Self {
        self.timeout = Some(timeout);
        self
    }

    /// Register a callback invoked as the download streams, with
    /// `(bytes_downloaded_so_far, total_bytes)` — `total_bytes` is `None` when the server does
    /// not send a `Content-Length`. Independent of the terminal progress bar
    /// ([`show_download_progress`](Self::show_download_progress)); use it to drive a GUI, structured logging, or
    /// any non-terminal progress display. The callback is `Fn`, so track state via interior
    /// mutability (e.g. an `AtomicU64` or a channel).
    #[doc(alias = "set_progress_callback")]
    pub fn progress_callback(
        &mut self,
        callback: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
    ) -> &mut Self {
        self.on_progress = Some(ProgressCallback(std::sync::Arc::new(callback)));
        self
    }

    /// Internal: set the progress callback from an already-wrapped `Arc` (used by the update
    /// flow to forward an `Update`'s callback to its download).
    pub(crate) fn set_progress_callback_arc(
        &mut self,
        callback: std::sync::Arc<DynProgressFn>,
    ) -> &mut Self {
        self.on_progress = Some(ProgressCallback(callback));
        self
    }

    /// Set the progress style
    #[doc(alias = "set_progress_style")]
    pub fn progress_style(
        &mut self,
        progress_template: impl Into<String>,
        progress_chars: impl Into<String>,
    ) -> &mut Self {
        self.progress_template = progress_template.into();
        self.progress_chars = progress_chars.into();
        self
    }

    /// Replace the entire download request `HeaderMap`. To add a single header without discarding
    /// the others, use [`header`](Self::header) instead.
    #[doc(alias = "set_headers")]
    pub fn replace_headers(&mut self, headers: http_client::header::HeaderMap) -> &mut Self {
        self.headers = headers;
        self
    }

    /// Use a pre-built blocking [`reqwest::Client`](::reqwest::blocking::Client) for the download
    /// instead of the per-call client. See the `Update` builder's `reqwest_client` for the
    /// rationale and precedence rules.
    #[cfg(feature = "reqwest")]
    pub fn reqwest_client(&mut self, client: ::reqwest::blocking::Client) -> &mut Self {
        self.client.blocking = Some(client);
        self
    }

    /// Async sibling of [`reqwest_client`](Self::reqwest_client), used by
    /// [`download_to_async`](Self::download_to_async).
    #[cfg(feature = "async")]
    pub fn reqwest_async_client(&mut self, client: ::reqwest::Client) -> &mut Self {
        self.client.r#async = Some(client);
        self
    }

    /// Use a pre-built [`ureq::Agent`](::ureq::Agent) for the download instead of the per-call
    /// agent. The agent owns its own timeout / TLS / proxy config.
    #[cfg(feature = "ureq")]
    pub fn ureq_agent(&mut self, agent: ::ureq::Agent) -> &mut Self {
        self.client.agent = Some(agent);
        self
    }

    /// Internal: set the client override from an already-built one (used by the update flow to
    /// forward an `Update`'s injected client to its download).
    pub(crate) fn set_client_override(&mut self, client: http_client::ClientOverride) -> &mut Self {
        self.client = client;
        self
    }

    /// Set a download request header, inserts into the existing `HeaderMap`
    #[doc(alias = "set_header")]
    pub fn header(
        &mut self,
        name: http_client::header::HeaderName,
        value: http_client::header::HeaderValue,
    ) -> &mut Self {
        self.headers.insert(name, value);
        self
    }

    /// Download the file behind the given `url` into the specified `dest`.
    /// Show a sliding progress bar if specified.
    /// If the resource doesn't specify a content-length, the progress bar will not be shown
    ///
    /// * Errors:
    ///     * HTTP client network errors
    ///     * Unsuccessful response status
    ///     * Progress-bar errors
    ///     * Reading from response to `BufReader`-buffer
    ///     * Writing from `BufReader`-buffer to `File`
    pub fn download_to<T: io::Write>(&self, mut dest: T) -> Result<()> {
        use io::BufRead;
        let mut headers = self.headers.clone();
        if !headers.contains_key(header::USER_AGENT) {
            headers.insert(
                header::USER_AGENT,
                "rust-reqwest/self-update"
                    .parse()
                    .expect("invalid user-agent"),
            );
        }

        let resp = http_client::get(&self.url, headers, self.timeout, &self.client)?;
        let size = resp
            .headers()
            .get(http_client::header::CONTENT_LENGTH)
            .map(|val| {
                val.to_str()
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        // `http_client::get` already errored on a non-success status (see `download_to_async`).
        let total = if size == 0 { None } else { Some(size) };
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut src = io::BufReader::new(resp.body());
        let mut downloaded: u64 = 0;
        let mut bar = if show_progress {
            let pb = ProgressBar::new(size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(&self.progress_template)
                    .expect("set ProgressStyle template failed")
                    .progress_chars(&self.progress_chars),
            );

            Some(pb)
        } else {
            None
        };
        loop {
            let n = {
                let buf = src.fill_buf()?;
                dest.write_all(buf)?;
                buf.len()
            };
            if n == 0 {
                break;
            }
            src.consume(n);
            downloaded += n as u64;

            if let Some(ref mut bar) = bar {
                bar.set_position(min(downloaded, size));
            }
            if let Some(ref cb) = self.on_progress {
                (cb.0)(downloaded, total);
            }
        }
        if let Some(ref mut bar) = bar {
            bar.finish_with_message("Done");
        }
        Ok(())
    }

    /// Async sibling of [`download_to`](Self::download_to): stream the download into `dest` using
    /// the async (reqwest) transport, driving the same progress bar / callback. `dest` is a
    /// synchronous writer (chunks are written as they arrive); file IO is not offloaded.
    #[cfg(feature = "async")]
    pub async fn download_to_async<T: io::Write>(&self, mut dest: T) -> Result<()> {
        use futures_util::StreamExt;

        let mut headers = self.headers.clone();
        if !headers.contains_key(header::USER_AGENT) {
            headers.insert(
                header::USER_AGENT,
                "rust-reqwest/self-update"
                    .parse()
                    .expect("invalid user-agent"),
            );
        }

        let resp = http_client::get_async(&self.url, headers, self.timeout, &self.client).await?;
        let size = resp
            .headers()
            .get(http_client::header::CONTENT_LENGTH)
            .map(|val| {
                val.to_str()
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        // `get_async` already errored on a non-success status.
        let total = if size == 0 { None } else { Some(size) };
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut downloaded: u64 = 0;
        let mut bar = if show_progress {
            let pb = ProgressBar::new(size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(&self.progress_template)
                    .expect("set ProgressStyle template failed")
                    .progress_chars(&self.progress_chars),
            );
            Some(pb)
        } else {
            None
        };

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            dest.write_all(&chunk)?;
            downloaded += chunk.len() as u64;

            if let Some(ref mut bar) = bar {
                bar.set_position(min(downloaded, size));
            }
            if let Some(ref cb) = self.on_progress {
                (cb.0)(downloaded, total);
            }
        }
        if let Some(ref mut bar) = bar {
            bar.finish_with_message("Done");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(dead_code, unused_mut, unused_variables)]
    // #![warn(unused_mut)]

    use super::*;
    #[cfg(feature = "compression-flate2")]
    use flate2::{self, write::GzEncoder};
    #[allow(unused_imports)]
    use std::{
        fs::{self, File},
        io::{self, Read, Write},
        path::{Path, PathBuf},
    };

    #[test]
    fn detect_plain() {
        assert_eq!(
            ArchiveKind::Plain(None),
            detect_archive(&PathBuf::from("Something.exe")).unwrap()
        );
    }

    #[test]
    fn move_all_commits_every_move() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Two new files to install over two existing destinations.
        let src_a = dir.path().join("src_a");
        let src_b = dir.path().join("src_b");
        fs::write(&src_a, b"new-a").unwrap();
        fs::write(&src_b, b"new-b").unwrap();
        let dst_a = dir.path().join("dst_a");
        let dst_b = dir.path().join("dst_b");
        fs::write(&dst_a, b"old-a").unwrap();
        fs::write(&dst_b, b"old-b").unwrap();

        MoveAll::from_temp(temp.path())
            .add(&src_a, &dst_a)
            .add(&src_b, &dst_b)
            .commit()
            .unwrap();

        assert_eq!(fs::read(&dst_a).unwrap(), b"new-a");
        assert_eq!(fs::read(&dst_b).unwrap(), b"new-b");
    }

    #[test]
    fn move_all_rolls_back_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Three moves: the first two are valid and overwrite existing destinations (so both are
        // stashed and applied), the third points at a non-existent source so its move fails. This
        // drives the already-applied first two back through `rollback()` (the stash-restore path).
        let src_a = dir.path().join("src_a");
        let src_b = dir.path().join("src_b");
        fs::write(&src_a, b"new-a").unwrap();
        fs::write(&src_b, b"new-b").unwrap();
        let missing_src = dir.path().join("does_not_exist");

        let dst_a = dir.path().join("dst_a");
        let dst_b = dir.path().join("dst_b");
        let dst_c = dir.path().join("dst_c");
        fs::write(&dst_a, b"old-a").unwrap();
        fs::write(&dst_b, b"old-b").unwrap();
        fs::write(&dst_c, b"old-c").unwrap();

        let res = MoveAll::from_temp(temp.path())
            .add(&src_a, &dst_a)
            .add(&src_b, &dst_b)
            .add(&missing_src, &dst_c)
            .commit();
        assert!(res.is_err(), "a failing move must abort the transaction");

        // Every destination is restored to its original contents — both the applied moves
        // (rolled back via the stash) and the one whose move failed mid-step.
        assert_eq!(
            fs::read(&dst_a).unwrap(),
            b"old-a",
            "the first applied move must be rolled back"
        );
        assert_eq!(
            fs::read(&dst_b).unwrap(),
            b"old-b",
            "the second applied move must be rolled back"
        );
        assert_eq!(
            fs::read(&dst_c).unwrap(),
            b"old-c",
            "the failed move's stashed destination must be restored"
        );
    }

    #[test]
    fn move_all_installs_fresh_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        // Destination does not pre-exist (fresh install, no stash needed).
        let src = dir.path().join("src");
        fs::write(&src, b"fresh").unwrap();
        let dst = dir.path().join("new_dst");

        MoveAll::from_temp(temp.path())
            .add(&src, &dst)
            .commit()
            .unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"fresh");
    }

    #[test]
    fn move_all_second_commit_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let temp = tempfile::tempdir().unwrap();

        let src = dir.path().join("src");
        fs::write(&src, b"new").unwrap();
        let dst = dir.path().join("dst");
        fs::write(&dst, b"old").unwrap();

        let mut mover = MoveAll::from_temp(temp.path());
        mover.add(&src, &dst);
        mover.commit().unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new");

        // The queue was drained, so a second commit does nothing and succeeds (rather than trying
        // to re-apply the move against the now-missing source and erroring out).
        mover.commit().unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"new");
    }

    #[test]
    fn download_invokes_progress_callback() {
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        // Serve a known-length body from a loopback server (no external network).
        let body = "x".repeat(20_000);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = body.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                served.len(),
                served
            );
            let _ = stream.write_all(resp.as_bytes());
        });

        let progress = Arc::new(Mutex::new(Vec::<(u64, Option<u64>)>::new()));
        let sink_progress = progress.clone();
        let mut out = Vec::new();
        Download::from_url(&format!("http://{addr}/file"))
            .progress_callback(move |downloaded, total| {
                sink_progress.lock().unwrap().push((downloaded, total));
            })
            .download_to(&mut out)
            .unwrap();

        assert_eq!(out.len(), 20_000);
        let calls = progress.lock().unwrap();
        assert!(!calls.is_empty(), "callback should have been invoked");
        // `total` reflects the Content-Length on every call.
        assert!(calls.iter().all(|(_, total)| *total == Some(20_000)));
        // `downloaded` is monotonically non-decreasing and reaches the full size.
        let mut last = 0u64;
        for (downloaded, _) in calls.iter() {
            assert!(*downloaded >= last);
            last = *downloaded;
        }
        assert_eq!(calls.last().unwrap().0, 20_000);
    }

    #[test]
    fn detect_plain_gz() {
        assert_eq!(
            ArchiveKind::Plain(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.exe.gz")).unwrap()
        );
    }

    #[cfg(not(feature = "archive-tar"))]
    #[test]
    #[ignore]
    fn detect_tar_gz() {
        println!("WARNING: Please enable 'archive-tar' feature!");
    }
    #[cfg(feature = "archive-tar")]
    #[test]
    fn detect_tar_gz() {
        assert_eq!(
            ArchiveKind::Tar(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.tar.gz")).unwrap()
        );
    }

    #[cfg(not(feature = "archive-tar"))]
    #[test]
    #[ignore]
    fn detect_plain_tar() {
        println!("WARNING: Please enable 'archive-tar' feature!");
    }
    #[cfg(feature = "archive-tar")]
    #[test]
    fn detect_plain_tar() {
        assert_eq!(
            ArchiveKind::Tar(None),
            detect_archive(&PathBuf::from("Something.tar")).unwrap()
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn detect_zip() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn detect_zip() {
        assert_eq!(
            ArchiveKind::Zip,
            detect_archive(&PathBuf::from("Something.zip")).unwrap()
        );
    }

    #[allow(dead_code)]
    fn cmp_content<T: AsRef<Path>>(path: T, s: &str) {
        let mut content = String::new();
        let mut f = File::open(&path).unwrap();
        f.read_to_string(&mut content).unwrap();
        assert!(s == content);
    }

    #[cfg(not(feature = "compression-flate2"))]
    #[test]
    #[ignore]
    fn unpack_plain_gzip() {
        println!("WARNING: Please enable 'compression-flate2' feature!");
    }
    #[cfg(feature = "compression-flate2")]
    #[test]
    fn unpack_plain_gzip() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(feature = "compression-flate2"))]
    #[test]
    #[ignore]
    fn unpack_plain_gzip_double_ext() {
        println!("WARNING: Please enable 'compression-flate2' feature!");
    }
    #[cfg(feature = "compression-flate2")]
    #[test]
    fn unpack_plain_gzip_double_ext() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_double_ext_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.txt.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_plain_gzip_double_ext_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(all(feature = "archive-tar", feature = "compression-flate2")))]
    #[test]
    #[ignore]
    fn unpack_tar_gzip() {
        println!("WARNING: Please enable 'archive-tar compression-flate2' features!");
    }
    #[cfg(all(feature = "archive-tar", feature = "compression-flate2"))]
    #[test]
    fn unpack_tar_gzip() {
        test_extract_into(
            "self_update_unpack_tar_gzip_src",
            "archive.tar.gz",
            ArchiveKind::Tar(Some(Compression::Gz)),
        );
    }

    #[cfg(not(feature = "compression-flate2"))]
    #[test]
    #[ignore]
    fn unpack_file_plain_gzip() {
        println!("WARNING: Please enable 'compression-flate2' feature!");
    }
    #[cfg(feature = "compression-flate2")]
    #[test]
    fn unpack_file_plain_gzip() {
        let tmp_dir = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_gzip_src")
            .tempdir()
            .expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = tempfile::Builder::new()
            .prefix("self_update_unpack_file_plain_gzip_outdir")
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_file(out_path, "renamed_file")
            .expect("extract fail");
        let out_file = out_path.join("renamed_file");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[cfg(not(all(feature = "archive-tar", feature = "compression-flate2")))]
    #[test]
    #[ignore]
    fn unpack_file_tar_gzip() {
        println!("WARNING: Please enable 'archive-tar compression-flate2' features!");
    }
    #[cfg(all(feature = "archive-tar", feature = "compression-flate2"))]
    #[test]
    fn unpack_file_tar_gzip() {
        test_extract_file(
            "self_update_unpack_file_tar_gzip_src",
            "archive.tar.gz",
            ArchiveKind::Tar(Some(Compression::Gz)),
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn unpack_zip() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn unpack_zip() {
        test_extract_into(
            "self_update_unpack_zip_src",
            "archive.zip",
            ArchiveKind::Zip,
        );
    }

    #[cfg(not(feature = "archive-zip"))]
    #[test]
    #[ignore]
    fn unpack_zip_file() {
        println!("WARNING: Please enable 'archive-zip' feature!");
    }
    #[cfg(feature = "archive-zip")]
    #[test]
    fn unpack_zip_file() {
        test_extract_file(
            "self_update_unpack_zip_src",
            "archive.zip",
            ArchiveKind::Zip,
        );
    }

    fn test_extract_into(tmpfile_prefix: &str, src_archive_path: &str, archive_kind: ArchiveKind) {
        let tmp_dir = tempfile::Builder::new()
            .prefix(tmpfile_prefix)
            .tempdir()
            .expect("Failed to create temp dir");

        let tmp_path = tmp_dir.path();
        let archive_file_path = tmp_path.join(src_archive_path);
        let archive_file = File::create(&archive_file_path).expect("Failed to create archive file");

        build_test_archive(archive_file, &archive_file_path, archive_kind);

        let out_tmp = tempfile::Builder::new()
            .prefix(&format!("{}_outdir", tmpfile_prefix))
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();

        Extract::from_source(&archive_file_path)
            .extract_into(out_path)
            .expect("extract fail");

        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        let out_file = out_path.join("inner_archive/temp2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }

    fn test_extract_file(tmpfile_prefix: &str, src_archive_path: &str, archive_kind: ArchiveKind) {
        let tmp_dir = tempfile::Builder::new()
            .prefix(tmpfile_prefix)
            .tempdir()
            .expect("Failed to create temp dir");

        let tmp_path = tmp_dir.path();
        let archive_file_path = tmp_path.join(src_archive_path);
        let archive_file = File::create(&archive_file_path).expect("Failed to create archive file");

        build_test_archive(archive_file, &archive_file_path, archive_kind);

        let out_tmp = tempfile::Builder::new()
            .prefix(&format!("{}_outdir", tmpfile_prefix))
            .tempdir()
            .expect("tempdir fail");
        let out_path = out_tmp.path();

        Extract::from_source(&archive_file_path)
            .extract_file(out_path, "temp.txt")
            .expect("extract fail");
        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        Extract::from_source(&archive_file_path)
            .extract_file(out_path, "inner_archive/temp2.txt")
            .expect("extract fail");
        let out_file = out_path.join("inner_archive/temp2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }

    fn build_test_archive<T: AsRef<Path>>(
        mut archive_file: fs::File,
        archive_file_path: T,
        archive_kind: ArchiveKind,
    ) {
        let archive_file_path = archive_file_path.as_ref();

        match archive_kind {
            #[cfg(all(feature = "archive-tar", feature = "compression-flate2"))]
            ArchiveKind::Tar(Some(Compression::Gz)) => {
                let tmp_tar_path = archive_file_path
                    .parent()
                    .expect("Missing archive file path parent")
                    .join("tar_contents");
                let tmp_tar_inner_path = tmp_tar_path.join("inner_archive");
                fs::create_dir_all(&tmp_tar_inner_path).expect("Failed to create temp tar path");

                let fp = tmp_tar_path.join("temp.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a test!").unwrap();

                let fp = tmp_tar_inner_path.join("temp2.txt");
                let mut tmp_file = File::create(fp).expect("temp file create fail");
                tmp_file.write_all(b"This is a second test!").unwrap();

                let mut ar = tar::Builder::new(vec![]);
                ar.append_dir_all(".", &tmp_tar_path)
                    .expect("tar append dir all fail");
                let tar_writer = ar.into_inner().expect("failed getting tar writer");

                let mut e = GzEncoder::new(&mut archive_file, flate2::Compression::default());
                io::copy(&mut tar_writer.as_slice(), &mut e)
                    .expect("failed writing from tar archive to gz encoder");
                e.finish().expect("gz finish fail");
            }

            #[cfg(feature = "archive-zip")]
            ArchiveKind::Zip => {
                let mut zip = zip::ZipWriter::new(archive_file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                zip.start_file("temp.txt", options)
                    .expect("failed starting zip file");
                zip.write_all(b"This is a test!")
                    .expect("failed writing to zip");
                zip.start_file("inner_archive/temp2.txt", options)
                    .expect("failed starting second zip file");
                zip.write_all(b"This is a second test!")
                    .expect("failed writing to second zip");
                zip.finish().expect("failed finishing zip");
            }

            _ => {
                unimplemented!("{:?} not handled", archive_kind);
            }
        }
    }
}
