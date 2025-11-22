# self_update


[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`.
Note, the [`trust`](https://github.com/japaric/trust) project provides a nice setup for
producing release-builds via CI (travis/appveyor).

### Features

The following [cargo features](https://doc.rust-lang.org/cargo/reference/manifest.html#the-features-section) are
available (but _disabled_ by default):

* `archive-tar`: Support for _tar_ archive format;
* `archive-zip`: Support for _zip_ archive format;
* `compression-flate2`: Support for _gzip_ compression;
* `compression-zip-deflate`: Support for _zip_'s _deflate_ compression format;
* `compression-zip-bzip2`: Support for _zip_'s _bzip2_ compression format;
* `rustls`: Use [pure rust TLS implementation](https://github.com/ctz/rustls) for network requests. This feature does _not_ support 32bit macOS;
* `signatures`: Use [zipsign](https://github.com/Kijewski/zipsign) to verify `.zip` and `.tar.gz` artifacts. Artifacts are assumed to have been signed using zipsign.

Please activate the feature(s) needed by your release files.

### Example

Run the following example to see `self_update` in action:

`cargo run --example github --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`.

There's also an equivalent example for gitlab:

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

Amazon S3, Google GCS, and DigitalOcean Spaces are also supported through the `S3` backend to check for new releases. Provided a `bucket_name`
and `asset_prefix` string, `self_update` will look up all matching files using the following format
as a convention for the filenames: `[directory/]<asset name>-<semver>-<platform/target>.<extension>`.
Leading directories will be stripped from the file name allowing the use of subdirectories in the S3 bucket,
and any file not matching the format, or not matching the provided prefix string, will be ignored.

```rust
use self_update::cargo_crate_version;

fn update() -> Result<(), Box<::std::error::Error>> {
    let status = self_update::backends::s3::Update::configure()
        .bucket_name("self_update_releases")
        .asset_prefix("something/self_update")
        .region("eu-west-2")
        .bin_name("self_update_example")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("S3 Update status: `{}`!", status.version());
    Ok(())
}
```

Separate utilities are also exposed (**NOTE**: the following example _requires_ the `archive-tar` feature,
see the [features](#features) section above). The `self_replace` crate is re-exported for convenience:

```rust
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
    let tmp_tarball = ::std::fs::File::open(&tmp_tarball_path)?;

    self_update::Download::from_url(&asset.download_url)
        .set_header(reqwest::header::ACCEPT, "application/octet-stream".parse()?)
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

In case the application has been packaged as a bundle (using `cargo bundle` e.g. on macOS as `Application.app`), the following example can be used:
Note that for zipped releases, the deflate feature is required:
```toml
features = ["archive-zip", "compression-zip-deflate"]
```
```rust
const MACOS_APP_NAME: &str = "Application.app";

/// method to copy the complete directory `src` to `dest` but skipping the binary `binary_name`
/// since we have to  use `self-replace` for that.
fn copy_dir(src: &Path, dest: &Path, binary_name: &str) -> io::Result<()> {
    // Ensure the destination directory exists
    if !dest.exists() {
        fs::create_dir_all(dest)?;
    }

    // Iterate through entries in the source directory
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if path.is_dir() {
            // Recursively copy subdirectories
            copy_dir(&path, &dest_path, binary_name)?;
        } else if let Some(file_name) = path.file_name() {
            if file_name != binary_name {
                // Copy files except for the binary
                fs::copy(&path, &dest_path)?;
            }
        }
    }

    Ok(())
}

/// custom update function for use with bundles
pub fn update(release: Release) -> Result<(), Box<dyn std::error::Error>> {
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
    
    let tmp_archive_dir = tempfile::TempDir::new()?;
    let tmp_archive_path = tmp_archive_dir.path().join(&asset.name);
    let tmp_archive = fs::File::create(&tmp_archive_path)?;

    self_update::Download::from_url(&asset.download_url)
        .set_header(reqwest::header::ACCEPT, "application/octet-stream".parse()?)
        .download_to(&tmp_archive)?;

    self_update::Extract::from_source(&tmp_archive_path).extract_into(tmp_archive_dir.path())?;
    let new_exe = if cfg!(target_os = "windows") {
        // only get the exe path on windows
        let binary = env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        tmp_archive_dir.path().join(binary)
    } else if cfg!(target_os = "macos") {
        // get the binary path on macOS
        let binary = env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        
        // get the parent directory of the `Application.app` bundle
        let app_dir = env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();

        let app_name = app_dir
            .clone()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let _ = copy_dir(&tmp_archive_dir.path().join(&app_name), &app_dir, &binary);

        // MACOS_APP_NAME either needs to be hardcoded or extracted from the downloaded and
        // extracted archive, but we cannot just assume that the parent directory of the
        // currently running executable is equal to the app name - this is especially not
        // the case if we run the code with `cargo run`.
        tmp_archive_dir
            .path()
            .join(format!("{}/Contents/MacOS/{}", MACOS_APP_NAME, binary))
    } else if cfg!(target_os = "linux") {
        // only get the binary path from linux
        let binary = env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        tmp_archive_dir.path().join(binary)
    } else {
        panic!("Running on an unsupported OS");
    };
    
    // replace as usual
    self_replace::self_replace(new_exe)?;
    Ok(())
}


```

### Troubleshooting

When using cross compilation tools such as cross if you want to use rustls and not openssl

```toml
self_update = { version = "0.27.0", features = ["rustls"], default-features = false }
```


License: MIT
