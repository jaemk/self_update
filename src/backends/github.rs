/*!
GitHub releases
*/
use std::env;
use std::fs;
use std::path::PathBuf;

use hyper_old_types::header::{LinkValue, RelationType};
use reqwest;
use serde_json;
use tempdir;

use super::super::Download;
use super::super::Extract;
use super::super::Move;
use super::super::Status;

use super::super::confirm;
use super::super::errors::*;
use super::super::version;

/// GitHub release-asset information
#[derive(Clone, Debug)]
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
        let download_url = asset["browser_download_url"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `browser_download_url`"))?;
        let name = asset["name"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Asset missing `name`"))?;
        Ok(ReleaseAsset {
            download_url: download_url.to_owned(),
            name: name.to_owned(),
        })
    }
}

/// Update status with extended information from Github
pub enum GitHubUpdateStatus {
    /// Crate is up to date
    UpToDate,
    /// Crate was updated to the contained release
    Updated(Release),
}

impl GitHubUpdateStatus {
    /// Turn the extended information into the crate's standard `Status` enum
    pub fn into_status(self, current_version: String) -> Status {
        match self {
            GitHubUpdateStatus::UpToDate => Status::UpToDate(current_version),
            GitHubUpdateStatus::Updated(release) => Status::Updated(release.version().into()),
        }
    }

    /// Returns `true` if `Status::UpToDate`
    pub fn uptodate(&self) -> bool {
        match *self {
            GitHubUpdateStatus::UpToDate => true,
            _ => false,
        }
    }

    /// Returns `true` if `Status::Updated`
    pub fn updated(&self) -> bool {
        !self.uptodate()
    }
}

/// GitHub release information
#[derive(Clone, Debug)]
pub struct Release {
    pub name: String,
    pub body: String,
    pub tag: String,
    pub date_created: String,
    pub assets: Vec<ReleaseAsset>,
}
impl Release {
    fn from_release(release: &serde_json::Value) -> Result<Release> {
        let tag = release["tag_name"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `tag_name`"))?;
        let date_created = release["created_at"]
            .as_str()
            .ok_or_else(|| format_err!(Error::Release, "Release missing `created_at`"))?;
        let name = release["name"].as_str().unwrap_or(tag);
        let body = release["body"].as_str().unwrap_or("");
        let assets = release["assets"]
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No assets found"))?;
        let assets = assets
            .iter()
            .map(ReleaseAsset::from_asset)
            .collect::<Result<Vec<ReleaseAsset>>>()?;
        Ok(Release {
            name: name.to_owned(),
            body: body.to_owned(),
            tag: tag.to_owned(),
            date_created: date_created.to_owned(),
            assets: assets,
        })
    }

    /// Check if release has an asset who's name contains the specified `target`
    pub fn has_target_asset(&self, target: &str) -> bool {
        self.assets.iter().any(|asset| asset.name.contains(target))
    }

    /// Return the first `ReleaseAsset` for the current release who's name
    /// contains the specified `target`
    pub fn asset_for(&self, target: &str) -> Option<ReleaseAsset> {
        self.assets
            .iter()
            .filter(|asset| asset.name.contains(target))
            .cloned()
            .nth(0)
    }

    pub fn version(&self) -> &str {
        self.tag.trim_left_matches('v')
    }
}

/// `ReleaseList` Builder
#[derive(Clone, Debug)]
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
        Ok(ReleaseList {
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                bail!(Error::Config, "`repo_owner` required")
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`repo_name` required")
            },
            target: self.target.clone(),
        })
    }
}

/// `ReleaseList` provides a builder api for querying a GitHub repo,
/// returning a `Vec` of available `Release`s
#[derive(Clone, Debug)]
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
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases",
            self.repo_owner, self.repo_name
        );
        let releases = Self::fetch_releases(&api_url)?;
        let releases = match self.target {
            None => releases,
            Some(ref target) => releases
                .into_iter()
                .filter(|r| r.has_target_asset(target))
                .collect::<Vec<_>>(),
        };
        Ok(releases)
    }

    fn fetch_releases(url: &str) -> Result<Vec<Release>> {
        let mut resp = reqwest::get(url)?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                url
            )
        }
        let releases = resp.json::<serde_json::Value>()?;
        let releases = releases
            .as_array()
            .ok_or_else(|| format_err!(Error::Release, "No releases found"))?;
        let mut releases = releases
            .iter()
            .map(Release::from_release)
            .collect::<Result<Vec<Release>>>()?;

        // handle paged responses containing `Link` header:
        // `Link: <https://api.github.com/resource?page=2>; rel="next"`
        let headers = resp.headers();
        let links = headers.get_all(reqwest::header::LINK);

        let next_link = links
            .iter()
            .filter_map(|link| {
                if let Ok(link) = link.to_str() {
                    let lv = LinkValue::new(link.to_owned());
                    if let Some(rels) = lv.rel() {
                        if rels.contains(&RelationType::Next) {
                            return Some(link);
                        }
                    }
                    None
                } else {
                    None
                }
            })
            .nth(0);

        Ok(match next_link {
            None => releases,
            Some(link) => {
                releases.extend(Self::fetch_releases(link)?);
                releases
            }
        })
    }
}

/// `github::Update` builder
///
/// Configure download and installation from
/// `https://api.github.com/repos/<repo_owner>/<repo_name>/releases/latest`
#[derive(Debug)]
pub struct UpdateBuilder {
    repo_owner: Option<String>,
    repo_name: Option<String>,
    target: Option<String>,
    bin_name: Option<String>,
    bin_install_path: Option<PathBuf>,
    bin_path_in_archive: Option<PathBuf>,
    show_download_progress: bool,
    show_output: bool,
    no_confirm: bool,
    current_version: Option<String>,
    target_version: Option<String>,
}
impl UpdateBuilder {
    /// Initialize a new builder, defaulting the `bin_install_path` to the current
    /// executable's path
    ///
    /// * Errors:
    ///     * Io - Determining current exe path
    pub fn new() -> Result<Self> {
        Ok(Self {
            repo_owner: None,
            repo_name: None,
            target: None,
            bin_name: None,
            bin_install_path: Some(env::current_exe()?),
            bin_path_in_archive: None,
            show_download_progress: false,
            show_output: true,
            no_confirm: false,
            current_version: None,
            target_version: None,
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

    /// Set the target version tag to update to. This will be used to search for a release
    /// by tag name:
    /// `/repos/:owner/:repo/releases/tags/:tag`
    ///
    /// If not specified, the latest available release is used.
    pub fn target_version_tag(&mut self, ver: &str) -> &mut Self {
        self.target_version = Some(ver.to_owned());
        self
    }

    /// Set the target triple that will be downloaded, e.g. `x86_64-unknown-linux-gnu`.
    /// The `get_target` function can cover use cases for most mainstream arches
    pub fn target(&mut self, target: &str) -> &mut Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Set the exe's name. Also sets `bin_path_in_archive` if it hasn't already been set.
    pub fn bin_name(&mut self, name: &str) -> &mut Self {
        self.bin_name = Some(name.to_owned());
        if self.bin_path_in_archive.is_none() {
            self.bin_path_in_archive = Some(PathBuf::from(name));
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
    /// # use self_update::backends::github::Update;
    /// # fn run() -> Result<(), Box<::std::error::Error>> {
    /// Update::configure()?
    ///     .bin_path_in_archive("bin/myapp")
    /// #   .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn bin_path_in_archive(&mut self, bin_path: &str) -> &mut Self {
        self.bin_path_in_archive = Some(PathBuf::from(bin_path));
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

    /// Confirm config and create a ready-to-use `Update`
    ///
    /// * Errors:
    ///     * Config - Invalid `Update` configuration
    pub fn build(&self) -> Result<Update> {
        Ok(Update {
            repo_owner: if let Some(ref owner) = self.repo_owner {
                owner.to_owned()
            } else {
                bail!(Error::Config, "`repo_owner` required")
            },
            repo_name: if let Some(ref name) = self.repo_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`repo_name` required")
            },
            target: if let Some(ref target) = self.target {
                target.to_owned()
            } else {
                bail!(Error::Config, "`target` required")
            },
            bin_name: if let Some(ref name) = self.bin_name {
                name.to_owned()
            } else {
                bail!(Error::Config, "`bin_name` required")
            },
            bin_install_path: if let Some(ref path) = self.bin_install_path {
                path.to_owned()
            } else {
                bail!(Error::Config, "`bin_install_path` required")
            },
            bin_path_in_archive: if let Some(ref path) = self.bin_path_in_archive {
                path.to_owned()
            } else {
                bail!(Error::Config, "`bin_path_in_archive` required")
            },
            current_version: if let Some(ref ver) = self.current_version {
                ver.to_owned()
            } else {
                bail!(Error::Config, "`current_version` required")
            },
            target_version: self.target_version.as_ref().map(|v| v.to_owned()),
            show_download_progress: self.show_download_progress,
            show_output: self.show_output,
            no_confirm: self.no_confirm,
        })
    }
}

/// Updates to a specified or latest release distributed via GitHub
#[derive(Debug)]
pub struct Update {
    repo_owner: String,
    repo_name: String,
    target: String,
    current_version: String,
    target_version: Option<String>,
    bin_name: String,
    bin_install_path: PathBuf,
    bin_path_in_archive: PathBuf,
    show_download_progress: bool,
    show_output: bool,
    no_confirm: bool,
}
impl Update {
    /// Initialize a new `Update` builder
    pub fn configure() -> Result<UpdateBuilder> {
        UpdateBuilder::new()
    }

    fn get_latest_release(repo_owner: &str, repo_name: &str) -> Result<Release> {
        set_ssl_vars!();
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            repo_owner, repo_name
        );
        let mut resp = reqwest::get(&api_url)?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                api_url
            )
        }
        let json = resp.json::<serde_json::Value>()?;
        Ok(Release::from_release(&json)?)
    }

    fn get_release_version(repo_owner: &str, repo_name: &str, ver: &str) -> Result<Release> {
        set_ssl_vars!();
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            repo_owner, repo_name, ver
        );
        let mut resp = reqwest::get(&api_url)?;
        if !resp.status().is_success() {
            bail!(
                Error::Network,
                "api request failed with status: {:?} - for: {:?}",
                resp.status(),
                api_url
            )
        }
        let json = resp.json::<serde_json::Value>()?;
        Ok(Release::from_release(&json)?)
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
        let current_version = self.current_version.clone();
        self.update_extended()
            .map(|s| s.into_status(current_version))
    }

    /// Same as `update`, but returns `GitHubUpdateStatus`.
    pub fn update_extended(self) -> Result<GitHubUpdateStatus> {
        self.println(&format!("Checking target-arch... {}", self.target));
        self.println(&format!(
            "Checking current version... v{}",
            self.current_version
        ));

        let release = match self.target_version {
            None => {
                self.print_flush("Checking latest released version... ")?;
                let release = Self::get_latest_release(&self.repo_owner, &self.repo_name)?;
                {
                    let release_tag = release.version();
                    self.println(&format!("v{}", release_tag));

                    if !version::bump_is_greater(&self.current_version, &release_tag)? {
                        return Ok(GitHubUpdateStatus::UpToDate);
                    }

                    self.println(&format!(
                        "New release found! v{} --> v{}",
                        &self.current_version, release_tag
                    ));
                    let qualifier =
                        if version::bump_is_compatible(&self.current_version, &release_tag)? {
                            ""
                        } else {
                            "*NOT* "
                        };
                    self.println(&format!("New release is {}compatible", qualifier));
                }
                release
            }
            Some(ref ver) => {
                self.println(&format!("Looking for tag: {}", ver));
                Self::get_release_version(&self.repo_owner, &self.repo_name, ver)?
            }
        };

        let target_asset = release.asset_for(&self.target).ok_or_else(|| {
            format_err!(
                Error::Release,
                "No asset found for target: `{}`",
                self.target
            )
        })?;

        if self.show_output || !self.no_confirm {
            println!("\n{} release status:", self.bin_name);
            println!("  * Current exe: {:?}", self.bin_install_path);
            println!("  * New exe release: {:?}", target_asset.name);
            println!("  * New exe download url: {:?}", target_asset.download_url);
            println!("\nThe new release will be downloaded/extracted and the existing binary will be replaced.");
        }
        if !self.no_confirm {
            confirm("Do you want to continue? [Y/n] ")?;
        }

        let tmp_dir_parent = self.bin_install_path.parent().expect(&format!(
            "Failed to determine parent dir of `bin_install_path`: {:?}",
            self.bin_install_path
        ));
        let tmp_dir =
            tempdir::TempDir::new_in(&tmp_dir_parent, &format!("{}_download", self.bin_name))?;
        let tmp_archive_path = tmp_dir.path().join(&target_asset.name);
        let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

        self.println("Downloading...");
        Download::from_url(&target_asset.download_url)
            .show_progress(self.show_download_progress)
            .download_to(&mut tmp_archive)?;

        self.print_flush("Extracting archive... ")?;
        Extract::from_source(&tmp_archive_path)
            .extract_file(&tmp_dir.path(), &self.bin_path_in_archive)?;
        let new_exe = tmp_dir.path().join(&self.bin_path_in_archive);
        self.println("Done");

        self.print_flush("Replacing binary file... ")?;
        let tmp_file = tmp_dir.path().join(&format!("__{}_backup", self.bin_name));
        Move::from_source(&new_exe)
            .replace_using_temp(&tmp_file)
            .to_dest(&self.bin_install_path)?;
        self.println("Done");
        Ok(GitHubUpdateStatus::Updated(release))
    }
}
