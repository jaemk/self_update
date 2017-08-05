/*!
GitHub releases
*/
use std::env;
use std::path::PathBuf;
use std::fs;

use serde_json;
use reqwest;
use tempdir;

use super::super::Status;
use super::super::Download;
use super::super::Extract;
use super::super::ArchiveKind;
use super::super::EncodingKind;
use super::super::Move;

use super::super::prompt_ok;
use super::super::should_update;
use super::super::errors::*;


/// GitHub release-asset information
#[derive(Debug)]
pub struct ReleaseAsset {
    pub download_url: String,
    pub name: String,
}
impl ReleaseAsset {
    /// Parse a release-asset json object
    ///
    /// Errors:
    ///     * Missing required name & download-url keys
    fn from_asset(asset: &serde_json::Value) -> Result<ReleaseAsset> {
        let download_url = asset["browser_download_url"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `browser_download_url`"))?;
        let name = asset["name"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `name`"))?;
        Ok(ReleaseAsset {
            download_url: download_url.to_owned(),
            name: name.to_owned(),
        })
    }
}


/// GitHub release information
#[derive(Debug)]
pub struct Release {
    pub name: String,
    pub body: String,
    pub tag: String,
    pub date_created: String,
    pub assets: Vec<ReleaseAsset>,
}
impl Release {
    fn from_release(release: &serde_json::Value) -> Result<Release> {
        let name = release["name"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `name`"))?;
        let body = release["body"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `body`"))?;
        let tag = release["tag_name"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `tag_name`"))?;
        let date_created = release["created_at"].as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `created_at`"))?;
        let assets = release["assets"].as_array().ok_or_else(|| format_err!(Error::Release, "No assets found"))?;
        let assets = assets.iter().map(ReleaseAsset::from_asset).collect::<Result<Vec<ReleaseAsset>>>()?;
        Ok(Release {
            name: name.to_owned(),
            body: body.to_owned(),
            tag: tag.to_owned(),
            date_created: date_created.to_owned(),
            assets: assets,
        })
    }
    fn has_target_asset(&self, target: &str) -> bool {
        self.assets.iter().any(|asset| asset.name.contains(target))
    }
}


/// `ReleaseList` Builder
#[derive(Debug)]
pub struct ReleaseListBuilder {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
}
impl ReleaseListBuilder {
    /// Set the repo owner, used to build a github api url
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    /// Set the optional arch `target` name, used to filter available releases
    pub fn with_target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Verify builder args, returning a `ReleaseList`
    pub fn build(&self) -> Result<ReleaseList> {
        Ok(ReleaseList{
            repo_owner: if let Some(ref owner) = self.repo_owner { owner.to_owned() } else { bail!(Error::Config, "`repo_owner` required") },
            repo_name: if let Some(ref name) = self.repo_name { name.to_owned() } else { bail!(Error::Config, "`repo_name` required") },
            target: self.target.clone(),
        })
    }
}


/// `ReleaseList` provides a builder api for querying a GitHub repo,
/// returning a `Vec` of available `Release`s
#[derive(Debug)]
pub struct ReleaseList {
    repo_owner: String,
    repo_name: String,
    target: Option<String>,
}
impl ReleaseList {
    /// Initialize a ReleaseListBuilder
    pub fn configure() -> ReleaseListBuilder {
        ReleaseListBuilder {
            repo_owner: None,
            repo_name: None,
            target: None,
        }
    }

    /// Retrieve a list of `Release`s.
    /// If specified, filter for those containing a specified `target`
    pub fn fetch(self) -> Result<Vec<Release>> {
        set_ssl_vars!();
        let api_url = format!("https://api.github.com/repos/{}/{}/releases", self.repo_owner, self.repo_name);
        let mut resp = reqwest::get(&api_url)?;
        if !resp.status().is_success() { bail!(Error::Network, "api request failed with status: {:?}", resp.status()) }
        let releases = resp.json::<serde_json::Value>()?;
        let releases = releases.as_array().ok_or_else(|| format_err!(Error::Release, "No releases found"))?;
        let releases = releases.iter().map(Release::from_release).collect::<Result<Vec<Release>>>()?;
        let releases = match self.target {
            None => releases,
            Some(ref target) => releases.into_iter().filter(|r| r.has_target_asset(target)).collect::<Vec<_>>(),
        };
        Ok(releases)
    }
}


/// `github::UpdateLatest` builder
///
/// Configure download and installation from
/// `https://api.github.com/repos/<repo_owner>/<repo_name>/releases/latest`
#[derive(Debug)]
pub struct UpdateLatestBuilder {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    bin_name: Option<String>,
    bin_install_path: Option<PathBuf>,
    bin_path_in_tarball: Option<PathBuf>,
    show_download_progress: bool,
    show_output: bool,
    no_confirm: bool,
    current_version: Option<String>,
}
impl UpdateLatestBuilder {
    /// Initialize a new builder, defaulting the `bin_install_path` to the current
    /// executable's path
    ///
    /// * Errors:
    ///     * Io - Determining current exe path
    pub fn new() -> Result<Self> {
        Ok(Self {
            repo_owner: None, repo_name: None,
            target: None, bin_name: None,
            bin_install_path: Some(env::current_exe()?),
            bin_path_in_tarball: None,
            show_download_progress: false,
            show_output: true,
            no_confirm: false,
            current_version: None,
        })
    }

    /// Set the repo owner, used to build a github api url
    pub fn repo_owner(&mut self, owner: &str) -> &mut Self {
        self.repo_owner = Some(owner.to_owned());
        self
    }

    /// Set the repo name, used to build a github api url
    pub fn repo_name(&mut self, name: &str) -> &mut Self {
        self.repo_name = Some(name.to_owned());
        self
    }

    /// Set the current app version, used to compare against the latest available version.
    /// The `cargo_crate_version!` macro can be used to pull the version from your `Cargo.toml`
    pub fn current_version(&mut self, ver: &str) -> &mut Self {
        self.current_version = Some(ver.to_owned());
        self
    }

    /// Set the target triple that will be downloaded, e.g. `x86_64-unknown-linux-gnu`.
    /// The `get_target` function can cover use cases for most mainstream arches
    pub fn target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Set the exe's name. Also sets `bin_path_in_tarball` if it hasn't already been set.
    pub fn bin_name(&mut self, name: &str) -> &mut Self {
        self.bin_name = Some(name.to_owned());
        if self.bin_path_in_tarball.is_none() {
            self.bin_path_in_tarball = Some(PathBuf::from(name));
        }
        self
    }

    /// Set the installation path for the new exe, defaults to the current
    /// executable's path
    pub fn bin_install_path(&mut self, bin_install_path: &str) -> &mut Self {
        self.bin_install_path = Some(PathBuf::from(bin_install_path));
        self
    }

    /// Set the path of the exe inside the release tarball. This is the location
    /// of the executable relative to the base of the tar'd directory and is the
    /// path that will be copied to the `bin_install_path`. If not specified, this
    /// will default to the value of `bin_name`. This only needs to be specified if
    /// the path to the binary (from the root of the tarball) is not equal to just
    /// the `bin_name`.
    ///
    /// # Example
    ///
    /// For a tarball `myapp.tar.gz` with the contents:
    ///
    /// ```shell
    /// myapp.tar/
    ///  |------- bin/
    ///  |         |--- myapp  # <-- executable
    /// ```
    ///
    /// The path provided should be:
    ///
    /// ```
    /// # use self_update::backends::github::UpdateLatest;
    /// # fn run() -> Result<(), Box<::std::error::Error>> {
    /// UpdateLatest::configure()?
    ///     .bin_path_in_tarball("bin/myapp")
    /// #   .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn bin_path_in_tarball(&mut self, bin_path: &str) -> &mut Self {
        self.bin_path_in_tarball = Some(PathBuf::from(bin_path));
        self
    }

    /// Toggle download progress bar, defaults to `off`.
    pub fn show_download_progress(&mut self, show: bool) -> &mut Self {
        self.show_download_progress = show;
        self
    }

    /// Toggle update output information, defaults to `true`.
    pub fn show_output(&mut self, show: bool) -> &mut Self {
        self.show_output = show;
        self
    }

    /// Toggle download confirmation. Defaults to `false`.
    pub fn no_confirm(&mut self, no_confirm: bool) -> &mut Self {
        self.no_confirm = no_confirm;
        self
    }

    /// Confirm config and create a ready-to-use `UpdateLatest`
    ///
    /// * Errors:
    ///     * Config - Invalid `UpdateLatest` configuration
    pub fn build(&self) -> Result<UpdateLatest> {
        Ok(UpdateLatest {
            repo_owner: if let Some(ref owner) = self.repo_owner { owner.to_owned() } else { bail!(Error::Config, "`repo_owner` required")},
            repo_name: if let Some(ref name) = self.repo_name { name.to_owned() } else { bail!(Error::Config, "`repo_name` required")},
            target: if let Some(ref target) = self.target { target.to_owned() } else { bail!(Error::Config, "`target` required")},
            bin_name: if let Some(ref name) = self.bin_name { name.to_owned() } else { bail!(Error::Config, "`bin_name` required")},
            bin_install_path: if let Some(ref path) = self.bin_install_path { path.to_owned() } else { bail!(Error::Config, "`bin_install_path` required")},
            bin_path_in_tarball: if let Some(ref path) = self.bin_path_in_tarball { path.to_owned() } else { bail!(Error::Config, "`bin_path_in_tarball` required")},
            current_version: if let Some(ref ver) = self.current_version { ver.to_owned() } else { bail!(Error::Config, "`current_version` required")},
            show_download_progress: self.show_download_progress,
            show_output: self.show_output,
            no_confirm: self.no_confirm,
        })
    }
}


/// Updates to the latest releases distributed via GitHub
#[derive(Debug)]
pub struct UpdateLatest {
    repo_owner: String,
    repo_name: String,
    target: String,
    current_version: String,
    bin_name: String,
    bin_install_path: PathBuf,
    bin_path_in_tarball: PathBuf,
    show_download_progress: bool,
    show_output: bool,
    no_confirm: bool,
}
impl UpdateLatest {
    /// Initialize a new `UpdateLatest` builder
    pub fn configure() -> Result<UpdateLatestBuilder> {
        UpdateLatestBuilder::new()
    }

    fn get_latest_release(repo_owner: &str, repo_name: &str) -> Result<serde_json::Value> {
        set_ssl_vars!();
        let api_url = format!("https://api.github.com/repos/{}/{}/releases/latest", repo_owner, repo_name);
        let mut resp = reqwest::get(&api_url)?;
        if !resp.status().is_success() { bail!(Error::Network, "api request failed with status: {:?}", resp.status()) }
        Ok(resp.json::<serde_json::Value>()?)
    }

    fn get_target_asset(assets: &serde_json::Value, target: &str) -> Result<ReleaseAsset> {
        let latest_assets = assets.as_array().ok_or_else(|| format_err!(Error::Release, "No release assets found!"))?;
        let target_asset = latest_assets.iter().map(ReleaseAsset::from_asset).collect::<Result<Vec<ReleaseAsset>>>();
        let target_asset = target_asset?.into_iter()
            .filter(|ra| ra.name.contains(target))
            .nth(0)
            .ok_or_else(|| format_err!(Error::Update, "No release asset found for current target: `{}`", target))?;
        Ok(target_asset)
    }

    fn print_flush(&self, msg: &str) -> Result<()> {
        if self.show_output {
            print_flush!("{}", msg);
        }
        Ok(())
    }

    fn println(&self, msg: &str) {
        if self.show_output {
            println!("{}", msg);
        }
    }

    /// Display release information and update the current binary to the latest release, pending
    /// confirmation from the user
    pub fn update(self) -> Result<Status> {
        self.println(&format!("Checking target-arch... {}", self.target));
        self.println(&format!("Checking current version... v{}", self.current_version));

        self.print_flush("Checking latest released version... ")?;
        let latest = Self::get_latest_release(&self.repo_owner, &self.repo_name)?;
        let latest_tag = latest["tag_name"].as_str()
            .ok_or_else(|| format_err!(Error::Update, "No tag_name found for latest release"))?
            .trim_left_matches("v");
        self.println(&format!("v{}", latest_tag));

        if !should_update(&self.current_version, &latest_tag)? {
            return Ok(Status::UpToDate(self.current_version.to_owned()))
        }

        self.println(&format!("New release found! v{} --> v{}", self.current_version, latest_tag));
        let target_asset = Self::get_target_asset(&latest["assets"], &self.target)?;

        if self.show_output || !self.no_confirm {
            println!("\n{} release status:", self.bin_name);
            println!("  * Current exe: {:?}", self.bin_install_path);
            println!("  * New exe tarball: {:?}", target_asset.name);
            println!("  * New exe download url: {:?}", target_asset.download_url);
            println!("\nThe new release will be downloaded/extracted and the existing binary will be replaced.");
        }
        if !self.no_confirm {
            prompt_ok("Do you want to continue? [y/n] ")?;
        }

        let tmp_dir_parent = self.bin_install_path.parent()
            .expect(&format!("Failed to determine parent dir of `bin_install_path`: {:?}", self.bin_install_path));
        let tmp_dir = tempdir::TempDir::new_in(&tmp_dir_parent, &format!("{}_download", self.bin_name))?;
        let tmp_tarball_path = tmp_dir.path().join(&target_asset.name);
        let mut tmp_tarball = fs::File::create(&tmp_tarball_path)?;

        self.println("Downloading...");
        Download::from_url(&target_asset.download_url)
            .show_progress(self.show_download_progress)
            .download_to(&mut tmp_tarball)?;

        self.print_flush("Extracting tarball... ")?;
        Extract::from_source(&tmp_tarball_path)
            .encoding(EncodingKind::Gz)
            .archive(ArchiveKind::Tar)
            .extract_into(&tmp_dir.path())?;
        let new_exe = tmp_dir.path().join(&self.bin_path_in_tarball);
        self.println("Done");

        self.print_flush("Replacing binary file... ")?;
        let tmp_file = tmp_dir.path().join(&format!("__{}_backup", self.bin_name));
        Move::from_source(&new_exe)
            .replace_using_temp(&tmp_file)
            .to_dest(&self.bin_install_path)?;
        self.println("Done");
        Ok(Status::Updated(latest_tag.to_owned()))
    }
}

