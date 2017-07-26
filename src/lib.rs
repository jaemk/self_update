/*!

[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Currently only GitHub releases are supported.

```shell
self_update = "0.1"
```

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`

```rust
#[macro_use] extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    let status = self_update::backends::github::Updater::configure()?
        .repo_owner("jaemk")
        .repo_name("self_update")
        .target(&target)
        .bin_name("self_update_example")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `v{}`!", status.version());
    Ok(())
}

# fn main() { }
```


Run the above example to see `self_update` in action: `cargo run --example github`

*/
extern crate serde_json;
extern crate reqwest;
extern crate tempdir;
extern crate flate2;
extern crate tar;
extern crate semver;


use std::fs;
use std::io::{self, Write, BufRead};
use std::path;


#[macro_use] pub mod macros;
pub mod errors;
pub mod backends;

use errors::*;


/// Status returned after updating
///
/// Wrapped `String`s are version tags
#[derive(Debug, Clone)]
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
        match *self {
            Status::UpToDate(_) => true,
            _ => false,
        }
    }

    /// Returns `true` if `Status::Updated`
    pub fn updated(&self) -> bool {
        match *self {
            Status::Updated(_) => true,
            _ => false,
        }
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


/// Try to determine the current target triple.
///
/// Returns a target triple (e.g. `x86_64-unknown-linux-gnu` or `i686-pc-windows-msvc`) or an
/// `Error::Config` if the current config cannot be determined or is not some combination of the
/// following values:
/// `linux, mac, windows` -- `i686, x86, armv7` -- `gnu, musl, msvc`
///
/// * Errors:
///     * Unexpected system config
pub fn get_target() -> Result<String> {
    let arch_config = (cfg!(target_arch = "x86"), cfg!(target_arch = "x86_64"), cfg!(target_arch = "arm"));
    let arch = match arch_config {
        (true, _, _) => "i686",
        (_, true, _) => "x86_64",
        (_, _, true) => "armv7",
        _ => bail!(Error::Update, "Unable to determine target-architecture"),
    };

    let os_config = (cfg!(target_os = "linux"), cfg!(target_os = "macos"), cfg!(target_os = "windows"));
    let os = match os_config {
        (true, _, _) => "unknown-linux",
        (_, true, _) => "apple-darwin",
        (_, _, true) => "pc-windows",
        _ => bail!(Error::Update, "Unable to determine target-os"),
    };

    let s;
    let os = if cfg!(target_os = "macos") {
        os
    } else {
        let env_config = (cfg!(target_env = "gnu"), cfg!(target_env = "musl"), cfg!(target_env = "msvc"));
        let env = match env_config {
            (true, _, _) => "gnu",
            (_, true, _) => "musl",
            (_, _, true) => "msvc",
            _ => bail!(Error::Update, "Unable to determine target-environment"),
        };
        s = format!("{}-{}", os, env);
        &s
    };

    Ok(format!("{}-{}", arch, os))
}


/// Flush a message to stdout and check if they respond `yes`
///
/// * Errors:
///     * Io flushing
///     * User entered anything other than Y/y
fn prompt_ok(msg: &str) -> Result<()> {
    print_flush!("{}", msg);

    let stdin = io::stdin();
    let mut s = String::new();
    stdin.read_line(&mut s)?;
    if s.trim().to_lowercase() != "y" {
        bail!(Error::Update, "Update aborted");
    }
    Ok(())
}


/// Display a download progress bar, returning the size of the
/// bar that needs to be cleared on the next run
///
/// * Errors:
///     * Io flushing
fn display_dl_progress(total_size: u64, bytes_read: u64, clear_size: usize) -> Result<usize> {
    let bar_width = 75;
    let ratio = bytes_read as f64 / total_size as f64;
    let percent = (ratio * 100.) as u8;
    let n_complete = (bar_width as f64 * ratio) as usize;
    let mut complete_bars = std::iter::repeat("=").take(n_complete).collect::<String>();
    if ratio != 1. { complete_bars.push('>'); }

    let clear_chars = std::iter::repeat("\x08").take(clear_size).collect::<String>();
    print_flush!("{}\r", clear_chars);

    let bar = format!("{percent: >3}% [{compl: <full_size$}] {total}kB", percent=percent, compl=complete_bars, full_size=bar_width, total=total_size/1000);
    print_flush!("{}", bar);

    Ok(bar.len())
}


/// Download the file behind the given `url` into the specified `dest`.
/// Show a sliding progress bar if specified.
/// If the resource doesn't specify a content-length, the progress bar will not be shown
///
/// * Errors:
///     * `reqwest` network errors
///     * Unsuccessful response status
///     * Progress-bar errors
///     * Reading from response to `BufReader`-buffer
///     * Writing from `BufReader`-buffer to `File`
fn download_to_file_with_progress<T: io::Write>(url: &str, mut dest: T, mut show_progress: bool) -> Result<()> {
    let resp = reqwest::get(url)?;
    let size = resp.headers()
        .get::<reqwest::header::ContentLength>()
        .map(|ct_len| **ct_len)
        .unwrap_or(0);
    if !resp.status().is_success() { bail!(Error::Update, "Download request failed with status: {:?}", resp.status()) }
    if size == 0 { show_progress = false; }

    let mut bytes_read = 0;
    let mut clear_size = 0;
    let mut src = io::BufReader::new(resp);
    loop {
        if show_progress {
            clear_size = display_dl_progress(size, bytes_read as u64, clear_size)?;
        }
        let n = {
            let mut buf = src.fill_buf()?;
            dest.write_all(&mut buf)?;
            buf.len()
        };
        if n == 0 { break; }
        src.consume(n);
        bytes_read += n;
    }
    if show_progress { println!(" ... Done"); }
    Ok(())
}


/// Extract contents of a tar.gz file to a specified directory, returning the
/// temp path to our new executable
///
/// * Errors:
///     * Io - opening files
///     * Io - gzip decoding
///     * Io - archive unpacking
fn extract_targz(tarball: &path::Path, into_dir: &path::Path) -> Result<()> {
    let tarball = fs::File::open(tarball)?;
    let tar = flate2::read::GzDecoder::new(tarball)?;
    let mut archive = tar::Archive::new(tar);
    archive.unpack(into_dir)?;
    Ok(())
}


/// Copy existing executable to a temp directory and try putting our new one in its place.
/// If something goes wrong, copy the original executable back
///
/// * Errors:
///     * Io - copying / renaming
fn replace_exe(current_exe: &path::Path, new_exe: &path::Path, tmp_file: &path::Path) -> Result<()> {
    fs::copy(current_exe, tmp_file)?;
    match fs::remove_file(current_exe) {
        Err(e) => {
            fs::copy(tmp_file, current_exe)?;
            return Err(Error::from(e))
        }
        Ok(_) => (),
    };
    match fs::rename(new_exe, current_exe) {
        Err(e) => {
            fs::copy(tmp_file, current_exe)?;
            return Err(Error::from(e))
        }
        Ok(_) => (),
    };
    Ok(())
}


/// Content if the latest cersion tag is greater than the current
fn should_update(current: &str, latest: &str) -> Result<bool> {
    use semver::Version;
    Ok(Version::parse(latest)? > Version::parse(current)?)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    #[test]
    fn can_determine_target_arch() {
        let target = get_target();
        assert!(target.is_ok(), "{:?}", target);
        let target = target.unwrap();
        if let Ok(env_target) = env::var("TARGET") {
            assert_eq!(target, env_target);
        }
    }
}

