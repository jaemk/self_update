/*!

[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

```shell
self_update = "0.4"
```

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`.
Note, the [`trust`](https://github.com/japaric/trust) project provides a nice setup for
producing release-builds via CI (travis/appveyor).


```
#[macro_use] extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    let status = self_update::backends::github::Update::configure()?
        .repo_owner("jaemk")
        .repo_name("self_update")
        .target(&target)
        .bin_name("self_update_example")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
# fn main() { }
```

Run the above example to see `self_update` in action: `cargo run --example github`

Separate utilities are also exposed:

```
extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .with_target(&target)
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    // get the first available release
    let asset = releases[0]
        .asset_for(&target).unwrap();

    let tmp_dir = self_update::TempDir::new_in(::std::env::current_dir()?, "self_update")?;
    let tmp_tarball_path = tmp_dir.path().join(&asset.name);
    let tmp_tarball = ::std::fs::File::open(&tmp_tarball_path)?;

    self_update::Download::from_url(&asset.download_url)
        .download_to(&tmp_tarball)?;

    self_update::Extract::from_source(&tmp_tarball_path)
        .archive(self_update::ArchiveKind::Tar)
        .encoding(self_update::EncodingKind::Gz)
        .extract_into(&tmp_dir.path())?;

    let tmp_file = tmp_dir.path().join("replacement_tmp");
    let bin_name = "self_update_bin";
    let bin_path = tmp_dir.path().join(bin_name);
    self_update::Move::from_source(&bin_path)
        .replace_using_temp(&tmp_file)
        .to_dest(&::std::env::current_exe()?)?;

    Ok(())
}
# fn main() { }
```

*/
extern crate serde_json;
extern crate reqwest;
extern crate tempdir;
extern crate flate2;
extern crate tar;
extern crate semver;
extern crate pbr;

pub use tempdir::TempDir;

use std::fs;
use std::io;
use std::path;


#[macro_use] mod macros;
pub mod errors;
pub mod backends;
pub mod version;

use errors::*;


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


/// Check if a version tag is greater than the current
#[deprecated(since="0.4.2", note="`should_update` functionality has been moved to `version::bump_is_greater`.\
                                  `version::bump_is_compatible` should be used instead.")]
pub fn should_update(current: &str, latest: &str) -> Result<bool> {
    use semver::Version;
    Ok(Version::parse(latest)? > Version::parse(current)?)
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
    if ! s.is_empty() && s != "y" {
        bail!(Error::Update, "Update aborted");
    }
    Ok(())
}


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


/// Supported archive formats
#[derive(Debug)]
pub enum ArchiveKind {
    Tar,
    Plain,
}


/// Supported encoding formats
#[derive(Debug)]
pub enum EncodingKind {
    Gz,
    Plain,
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
    archive: ArchiveKind,
    encoding: EncodingKind,
}
impl<'a> Extract<'a> {
    pub fn from_source(source: &'a path::Path) -> Extract<'a> {
        Self {
            source: source,
            archive: ArchiveKind::Plain,
            encoding: EncodingKind::Plain,
        }
    }
    pub fn archive(&mut self, kind: ArchiveKind) -> &mut Self {
        self.archive = kind;
        self
    }
    pub fn encoding(&mut self, kind: EncodingKind) -> &mut Self {
        self.encoding = kind;
        self
    }
    pub fn extract_into(&self, into_dir: &path::Path) -> Result<()> {
        let source = fs::File::open(self.source)?;
        let archive: Box<io::Read> = match self.encoding {
            EncodingKind::Plain => Box::new(source),
            EncodingKind::Gz => {
                let reader = flate2::read::GzDecoder::new(source);
                Box::new(reader)
            },
        };
        match self.archive {
            ArchiveKind::Plain => (),
            ArchiveKind::Tar => {
                let mut archive = tar::Archive::new(archive);
                archive.unpack(into_dir)?;
            }
        };
        Ok(())
    }
}


/// Moves a file from the given path to the specified destination.
///
/// `source` and `dest` must be on the same filesystem.
/// If `replace_using_temp` is provided, the destination file will be
/// replaced using the given temp path as a backup in case of `io` errors.
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
        Self {
            source: source,
            temp: None,
        }
    }

    /// If specified and the destination file already exists, the destination
    /// file will be "safely" replaced using a temp path.
    /// The `temp` dir should must be explicitly provided since `replace` operations require
    /// files to live on the same filesystem.
    pub fn replace_using_temp(&mut self, temp: &'a path::Path) -> &mut Self {
        self.temp = Some(temp);
        self
    }

    /// Move source file to specified destination
    pub fn to_dest(&self, dest: &path::Path) -> Result<()> {
        match self.temp {
            None => {
                fs::rename(self.source, dest)?;
            }
            Some(temp) => {
                if dest.exists() {
                    fs::rename(dest, temp)?;
                    match fs::rename(self.source, dest) {
                        Err(e) => {
                            fs::rename(temp, dest)?;
                            return Err(Error::from(e))
                        }
                        Ok(_) => (),
                    };
                } else {
                    fs::rename(self.source, dest)?;
                }
            }
        };
        Ok(())
    }
}


/// Download things into files
///
/// With optional progress bar
#[derive(Debug)]
pub struct Download {
    show_progress: bool,
    url: String,
}
impl Download {
    /// Specify download url
    pub fn from_url(url: &str) -> Self {
        Self {
            show_progress: false,
            url: url.to_owned(),
        }
    }

    /// Toggle download progress bar
    pub fn show_progress(&mut self, b: bool) -> &mut Self {
        self.show_progress = b;
        self
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
    pub fn download_to<T: io::Write>(&self, mut dest: T) -> Result<()> {
        use io::BufRead;

        set_ssl_vars!();
        let resp = reqwest::get(&self.url)?;
        let size = resp.headers()
            .get::<reqwest::header::ContentLength>()
            .map(|ct_len| **ct_len)
            .unwrap_or(0);
        if !resp.status().is_success() { bail!(Error::Update, "Download request failed with status: {:?}", resp.status()) }
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut src = io::BufReader::new(resp);
        let mut bar = if show_progress {
            let mut bar = pbr::ProgressBar::new(size);
            bar.set_units(pbr::Units::Bytes);
            bar.format("[=> ]");
            Some(bar)
        } else { None };
        loop {
            let n = {
                let mut buf = src.fill_buf()?;
                dest.write_all(&mut buf)?;
                buf.len()
            };
            if n == 0 { break; }
            src.consume(n);
            if let Some(ref mut bar) = bar {
                bar.add(n as u64);
            }
        }
        if show_progress { println!(" ... Done"); }
        Ok(())
    }
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

