#![cfg_attr(feature = "cargo-clippy", deny(clippy::all))]
#![cfg_attr(feature = "cargo-clippy", allow(clippy::new_ret_no_self))]
/*!

[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

```shell
self_update = "0.5"
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

    let bin_name = std::path::PathBuf::from("self_update_bin");
    self_update::Extract::from_source(&tmp_tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_file(&tmp_dir.path(), &bin_name)?;

    let tmp_file = tmp_dir.path().join("replacement_tmp");
    let bin_path = tmp_dir.path().join(bin_name);
    self_update::Move::from_source(&bin_path)
        .replace_using_temp(&tmp_file)
        .to_dest(&::std::env::current_exe()?)?;

    Ok(())
}
# fn main() { }
```

*/
extern crate either;
extern crate flate2;
extern crate hyper_old_types;
extern crate pbr;
extern crate reqwest;
extern crate semver;
extern crate serde_json;
extern crate tar;
extern crate tempdir;
extern crate zip;

pub use tempdir::TempDir;

use either::Either;
use std::fs;
use std::io;
use std::path;

#[macro_use]
mod macros;
pub mod backends;
pub mod errors;
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
    let arch_config = (
        cfg!(target_arch = "x86"),
        cfg!(target_arch = "x86_64"),
        cfg!(target_arch = "arm"),
    );
    let arch = match arch_config {
        (true, _, _) => "i686",
        (_, true, _) => "x86_64",
        (_, _, true) => "armv7",
        _ => bail!(Error::Update, "Unable to determine target-architecture"),
    };

    let os = if cfg!(target_os = "linux") {
        "unknown-linux"
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "windows") {
        "pc-windows"
    } else if cfg!(target_os = "freebsd") {
        "unknown-freebsd"
    } else {
        bail!(Error::Update, "Unable to determine target-os");
    };

    let s;
    let os = if cfg!(target_os = "macos") || cfg!(target_os = "freebsd") {
        os
    } else {
        let env_config = (
            cfg!(target_env = "gnu"),
            cfg!(target_env = "musl"),
            cfg!(target_env = "msvc"),
        );
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
#[deprecated(
    since = "0.4.2",
    note = "`should_update` functionality has been moved to `version::bump_is_greater`.\
            `version::bump_is_compatible` should be used instead."
)]
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
    if !s.is_empty() && s != "y" {
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArchiveKind {
    Tar(Option<Compression>),
    Plain(Option<Compression>),
    Zip,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compression {
    Gz,
}

fn detect_archive(path: &path::Path) -> ArchiveKind {
    match path.extension() {
        Some(extension) if extension == std::ffi::OsStr::new("zip") => ArchiveKind::Zip,
        Some(extension) if extension == std::ffi::OsStr::new("tar") => ArchiveKind::Tar(None),
        Some(extension) if extension == std::ffi::OsStr::new("gz") => match path
            .file_stem()
            .map(|e| path::Path::new(e))
            .and_then(|f| f.extension())
        {
            Some(extension) if extension == std::ffi::OsStr::new("tar") => {
                ArchiveKind::Tar(Some(Compression::Gz))
            }
            _ => ArchiveKind::Plain(Some(Compression::Gz)),
        },
        _ => ArchiveKind::Plain(None),
    }
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

    fn get_archive_reader(
        source: fs::File,
        compression: Option<Compression>,
    ) -> Either<fs::File, flate2::read::GzDecoder<fs::File>> {
        match compression {
            Some(Compression::Gz) => Either::Right(flate2::read::GzDecoder::new(source)),
            None => Either::Left(source),
        }
    }

    /// Extract an entire source archive into a specified path. If the source is a single compressed
    /// file and not an archive, it will be extracted into a file with the same name inside of
    /// `into_dir`.
    pub fn extract_into(&self, into_dir: &path::Path) -> Result<()> {
        let source = fs::File::open(self.source)?;
        let archive = self.archive.unwrap_or_else(|| detect_archive(&self.source));

        match archive {
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
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
                        let file_name = self.source.file_name().ok_or_else(|| {
                            Error::Update("Extractor source has no file-name".into())
                        })?;
                        let mut out_path = into_dir.join(file_name);
                        out_path.set_extension("");
                        let mut out_file = fs::File::create(&out_path)?;
                        io::copy(&mut reader, &mut out_file)?;
                    }
                    ArchiveKind::Tar(_) => {
                        let mut archive = tar::Archive::new(reader);
                        archive.unpack(into_dir)?;
                    }
                    _ => unreachable!(),
                };
            }
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i)?;
                    let path = into_dir.join(file.name());
                    let mut output = fs::File::create(path)?;
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
        let archive = self.archive.unwrap_or_else(|| detect_archive(&self.source));

        match archive {
            ArchiveKind::Plain(compression) | ArchiveKind::Tar(compression) => {
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
                        let file_name = file_to_extract.file_name().ok_or_else(|| {
                            Error::Update("Extractor source has no file-name".into())
                        })?;
                        let mut out_path = into_dir.join(file_name);
                        let mut out_file = fs::File::create(&out_path)?;
                        io::copy(&mut reader, &mut out_file)?;
                    }
                    ArchiveKind::Tar(_) => {
                        let mut archive = tar::Archive::new(reader);
                        let mut entry = archive
                            .entries()?
                            .filter_map(|e| e.ok())
                            .find(|e| e.path().ok().filter(|p| p == file_to_extract).is_some())
                            .ok_or_else(|| {
                                Error::Update(format!(
                                    "Could not find the required path in the archive: {:?}",
                                    file_to_extract
                                ))
                            })?;
                        entry.unpack_in(into_dir)?;
                    }
                    _ => {
                        panic!("Unreasonable code");
                    }
                };
            }
            ArchiveKind::Zip => {
                let mut archive = zip::ZipArchive::new(source)?;
                let mut file = archive.by_name(file_to_extract.to_str().unwrap())?;
                let mut output = fs::File::create(into_dir.join(file.name()))?;
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
            None => {
                fs::rename(self.source, dest)?;
            }
            Some(temp) => {
                if dest.exists() {
                    fs::rename(dest, temp)?;
                    if let Err(e) = fs::rename(self.source, dest) {
                        fs::rename(temp, dest)?;
                        return Err(Error::from(e));
                    }
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
        let size = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .map(|val| {
                val.to_str()
                    .map(|s| s.parse::<u64>().unwrap_or(0))
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        if !resp.status().is_success() {
            bail!(
                Error::Update,
                "Download request failed with status: {:?}",
                resp.status()
            )
        }
        let show_progress = if size == 0 { false } else { self.show_progress };

        let mut src = io::BufReader::new(resp);
        let mut bar = if show_progress {
            let mut bar = pbr::ProgressBar::new(size);
            bar.set_units(pbr::Units::Bytes);
            bar.format("[=> ]");
            Some(bar)
        } else {
            None
        };
        loop {
            let n = {
                let mut buf = src.fill_buf()?;
                dest.write_all(&buf)?;
                buf.len()
            };
            if n == 0 {
                break;
            }
            src.consume(n);
            if let Some(ref mut bar) = bar {
                bar.add(n as u64);
            }
        }
        if show_progress {
            println!(" ... Done");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2;
    use flate2::write::GzEncoder;
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};
    use tar;
    use tempdir::TempDir;
    use zip;

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

    #[test]
    fn detect_plain() {
        assert_eq!(
            ArchiveKind::Plain(None),
            detect_archive(&PathBuf::from("Something.exe"))
        );
    }

    #[test]
    fn detect_plain_gz() {
        assert_eq!(
            ArchiveKind::Plain(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.exe.gz"))
        );
    }

    #[test]
    fn detect_tar_gz() {
        assert_eq!(
            ArchiveKind::Tar(Some(Compression::Gz)),
            detect_archive(&PathBuf::from("Something.tar.gz"))
        );
    }

    #[test]
    fn detect_plain_tar() {
        assert_eq!(
            ArchiveKind::Tar(None),
            detect_archive(&PathBuf::from("Something.tar"))
        );
    }

    #[test]
    fn detect_zip() {
        assert_eq!(
            ArchiveKind::Zip,
            detect_archive(&PathBuf::from("Something.zip"))
        );
    }

    fn cmp_content<T: AsRef<Path>>(path: T, s: &str) {
        let mut content = String::new();
        let mut f = File::open(&path).unwrap();
        f.read_to_string(&mut content).unwrap();
        assert!(s == content);
    }

    #[test]
    fn unpack_plain_gzip() {
        let tmp_dir = TempDir::new("self_update_unpack_plain_gzip_src").expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp = TempDir::new("self_update_unpack_plain_gzip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(&out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[test]
    fn unpack_plain_gzip_double_ext() {
        let tmp_dir =
            TempDir::new("self_update_unpack_plain_gzip_double_ext_src").expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.txt.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp =
            TempDir::new("self_update_unpack_plain_gzip_double_ext_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_into(&out_path)
            .expect("extract fail");
        let out_file = out_path.join("temp.txt");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[test]
    fn unpack_tar_gzip() {
        let tmp_dir = TempDir::new("self_update_unpack_tar_gzip_src").expect("tempdir fail");
        let tmp_path = tmp_dir.path();

        let archive_src = tmp_path.join("src_archive");
        fs::create_dir_all(&archive_src).expect("tmp archive-dir create fail");

        let fp = archive_src.join("temp.txt");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        tmp_file.write_all(b"This is a test!").unwrap();

        let fp2 = archive_src.join("temp2.txt");
        let mut tmp_file = File::create(&fp2).expect("temp file 2 create fail");
        tmp_file.write_all(b"This is a second test!").unwrap();

        let mut ar = tar::Builder::new(vec![]);
        ar.append_dir_all("inner_archive", &archive_src)
            .expect("tar append dir all fail");
        let tar_writer = ar.into_inner().expect("failed getting tar writer");

        let archive_fp = tmp_path.with_file_name("archive_file.tar.gz");
        let mut archive_file = File::create(&archive_fp).expect("failed creating archive file");
        let mut e = GzEncoder::new(&mut archive_file, flate2::Compression::default());
        io::copy(&mut tar_writer.as_slice(), &mut e)
            .expect("failed writing from tar archive to gz encoder");
        e.finish().expect("gz finish fail");

        let out_tmp = TempDir::new("self_update_unpack_tar_gzip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&archive_fp)
            .extract_into(&out_path)
            .expect("extract fail");

        let out_file = out_path.join("inner_archive/temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        let out_file = out_path.join("inner_archive/temp2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }

    #[test]
    fn unpack_file_plain_gzip() {
        let tmp_dir = TempDir::new("self_update_unpack_file_plain_gzip_src").expect("tempdir fail");
        let fp = tmp_dir.path().with_file_name("temp.gz");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        let mut e = GzEncoder::new(&mut tmp_file, flate2::Compression::default());
        e.write_all(b"This is a test!").expect("gz encode fail");
        e.finish().expect("gz finish fail");

        let out_tmp =
            TempDir::new("self_update_unpack_file_plain_gzip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&fp)
            .extract_file(&out_path, "renamed_file")
            .expect("extract fail");
        let out_file = out_path.join("renamed_file");
        assert!(out_file.exists());
        cmp_content(out_file, "This is a test!");
    }

    #[test]
    fn unpack_file_tar_gzip() {
        let tmp_dir = TempDir::new("self_update_unpack_file_tar_gzip_src").expect("tempdir fail");
        let tmp_path = tmp_dir.path();

        let archive_src = tmp_path.join("src_archive");
        fs::create_dir_all(&archive_src).expect("tmp archive-dir create fail");

        let fp = archive_src.join("temp.txt");
        let mut tmp_file = File::create(&fp).expect("temp file create fail");
        tmp_file.write_all(b"This is a test!").unwrap();

        let mut ar = tar::Builder::new(vec![]);
        ar.append_dir_all("inner_archive", &archive_src)
            .expect("tar append dir all fail");
        let tar_writer = ar.into_inner().expect("failed getting tar writer");

        let archive_fp = tmp_path.with_file_name("archive_file.tar.gz");
        let mut archive_file = File::create(&archive_fp).expect("failed creating archive file");
        let mut e = GzEncoder::new(&mut archive_file, flate2::Compression::default());
        io::copy(&mut tar_writer.as_slice(), &mut e)
            .expect("failed writing from tar archive to gz encoder");
        e.finish().expect("gz finish fail");

        let out_tmp =
            TempDir::new("self_update_unpack_file_tar_gzip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&archive_fp)
            .extract_file(&out_path, "inner_archive/temp.txt")
            .expect("extract fail");
        let out_file = out_path.join("inner_archive/temp.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");
    }

    #[test]
    fn unpack_zip() {
        let tmp_dir = TempDir::new("self_update_unpack_zip_src").expect("tempdir fail");
        let tmp_path = tmp_dir.path();

        let archive_path = tmp_path.join("archive.zip");
        let archive_file = File::create(&archive_path).expect("create file fail");
        let mut zip = zip::ZipWriter::new(archive_file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("zipped.txt", options)
            .expect("failed starting zip file");
        zip.write_all(b"This is a test!")
            .expect("failed writing to zip");
        zip.start_file("zipped2.txt", options)
            .expect("failed starting second zip file");
        zip.write_all(b"This is a second test!")
            .expect("failed writing to second zip");
        zip.finish().expect("failed finishing zip");

        let out_tmp = TempDir::new("self_update_unpack_zip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&archive_path)
            .extract_into(&out_path)
            .expect("extract fail");
        let out_file = out_path.join("zipped.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a test!");

        let out_file2 = out_path.join("zipped2.txt");
        assert!(out_file2.exists());
        cmp_content(&out_file2, "This is a second test!");
    }

    #[test]
    fn unpack_zip_file() {
        let tmp_dir = TempDir::new("self_update_unpack_zip_src").expect("tempdir fail");
        let tmp_path = tmp_dir.path();

        let archive_path = tmp_path.join("archive.zip");
        let archive_file = File::create(&archive_path).expect("create file fail");
        let mut zip = zip::ZipWriter::new(archive_file);
        let options =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("zipped.txt", options)
            .expect("failed starting zip file");
        zip.write_all(b"This is a test!")
            .expect("failed writing to zip");
        zip.start_file("zipped2.txt", options)
            .expect("failed starting second zip file");
        zip.write_all(b"This is a second test!")
            .expect("failed writing to second zip");
        zip.finish().expect("failed finishing zip");

        let out_tmp = TempDir::new("self_update_unpack_zip_outdir").expect("tempdir fail");
        let out_path = out_tmp.path();
        Extract::from_source(&archive_path)
            .extract_file(&out_path, "zipped2.txt")
            .expect("extract fail");
        let out_file = out_path.join("zipped2.txt");
        assert!(out_file.exists());
        cmp_content(&out_file, "This is a second test!");
    }
}
