use indicatif::ProgressStyle;
use reqwest::{self, header};
use std::env;
use std::fs;
#[cfg(not(windows))]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use crate::{confirm, errors::*, version, Download, Extract, Move, Status};

/// Release asset information
#[derive(Clone, Debug, Default)]
pub struct ReleaseAsset {
    pub download_url: String,
    pub name: String,
}

/// Update status with extended information
pub enum UpdateStatus {
    /// Crate is up to date
    UpToDate,
    /// Crate was updated to the contained release
    Updated(Release),
}

impl UpdateStatus {
    /// Turn the extended information into the crate's standard `Status` enum
    pub fn into_status(self, current_version: String) -> Status {
        match self {
            UpdateStatus::UpToDate => Status::UpToDate(current_version),
            UpdateStatus::Updated(release) => Status::Updated(release.version),
        }
    }

    /// Returns `true` if `Status::UpToDate`
    pub fn uptodate(&self) -> bool {
        match *self {
            UpdateStatus::UpToDate => true,
            _ => false,
        }
    }

    /// Returns `true` if `Status::Updated`
    pub fn updated(&self) -> bool {
        !self.uptodate()
    }
}

/// Release information
#[derive(Clone, Debug, Default)]
pub struct Release {
    pub name: String,
    pub version: String,
    pub date: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
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
            .next()
    }
}

/// Updates to a specified or latest release
pub trait ReleaseUpdate {
    /// Fetch details of the latest release from the backend
    fn get_latest_release(&self) -> Result<Release>;

    /// Fetch details of the release matching the specified version
    fn get_release_version(&self, ver: &str) -> Result<Release>;

    /// Current version of binary being updated
    fn current_version(&self) -> String;

    /// Target platform the update is being performed for
    fn target(&self) -> String;

    /// Target version optionally specified for the update
    fn target_version(&self) -> Option<String>;

    /// Name of the binary being updated
    fn bin_name(&self) -> String;

    /// Installation path for the binary being updated
    fn bin_install_path(&self) -> PathBuf;

    /// Path of the binary to be extracted from release package
    fn bin_path_in_archive(&self) -> PathBuf;

    /// Flag indicating if progress information shall be output when downloading a release
    fn show_download_progress(&self) -> bool;

    /// Flag indicating if process informative messages shall be output
    fn show_output(&self) -> bool;

    /// Flag indicating if the user shouldn't be prompted to confirm an update
    fn no_confirm(&self) -> bool;

    /// Styling for progress information if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    fn progress_style(&self) -> Option<ProgressStyle>;

    /// Authorisation token for communicating with backend
    fn auth_token(&self) -> Option<String>;

    /// Display release information and update the current binary to the latest release, pending
    /// confirmation from the user
    fn update(&self) -> Result<Status> {
        let current_version = self.current_version();
        self.update_extended()
            .map(|s| s.into_status(current_version))
    }

    /// Same as `update`, but returns `UpdateStatus`.
    fn update_extended(&self) -> Result<UpdateStatus> {
        let current_version = self.current_version();
        let target = self.target();
        let show_output = self.show_output();
        println(show_output, &format!("Checking target-arch... {}", target));
        println(
            show_output,
            &format!("Checking current version... v{}", current_version),
        );

        let release = match self.target_version() {
            None => {
                print_flush(show_output, "Checking latest released version... ")?;
                let release = self.get_latest_release()?;
                {
                    println(show_output, &format!("v{}", release.version));

                    if !version::bump_is_greater(&current_version, &release.version)? {
                        return Ok(UpdateStatus::UpToDate);
                    }

                    println(
                        show_output,
                        &format!(
                            "New release found! v{} --> v{}",
                            current_version, release.version
                        ),
                    );
                    let qualifier =
                        if version::bump_is_compatible(&current_version, &release.version)? {
                            ""
                        } else {
                            "*NOT* "
                        };
                    println(
                        show_output,
                        &format!("New release is {}compatible", qualifier),
                    );
                }
                release
            }
            Some(ref ver) => {
                println(show_output, &format!("Looking for tag: {}", ver));
                self.get_release_version(ver)?
            }
        };

        let target_asset = release.asset_for(&target).ok_or_else(|| {
            format_err!(Error::Release, "No asset found for target: `{}`", target)
        })?;

        let bin_install_path = self.bin_install_path();
        let bin_name = self.bin_name();
        let prompt_confirmation = !self.no_confirm();
        if self.show_output() || prompt_confirmation {
            println!("\n{} release status:", bin_name);
            println!("  * Current exe: {:?}", bin_install_path);
            println!("  * New exe release: {:?}", target_asset.name);
            println!("  * New exe download url: {:?}", target_asset.download_url);
            println!("\nThe new release will be downloaded/extracted and the existing binary will be replaced.");
        }
        if prompt_confirmation {
            confirm("Do you want to continue? [Y/n] ")?;
        }

        let tmp_dir_parent = if cfg!(windows) {
            env::var_os("TEMP").map(PathBuf::from)
        } else {
            bin_install_path.parent().map(PathBuf::from)
        }
        .ok_or_else(|| Error::Update("Failed to determine parent dir".into()))?;
        let tmp_dir = tempdir::TempDir::new_in(&tmp_dir_parent, &format!("{}_download", bin_name))?;
        let tmp_archive_path = tmp_dir.path().join(&target_asset.name);
        let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

        println(show_output, "Downloading...");
        let mut download = Download::from_url(&target_asset.download_url);
        let mut headers = api_headers(&self.auth_token());
        headers.insert(header::ACCEPT, "application/octet-stream".parse().unwrap());
        download.set_headers(headers);
        download.show_progress(self.show_download_progress());

        if let Some(ref progress_style) = self.progress_style() {
            download.set_progress_style(progress_style.clone());
        }

        download.download_to(&mut tmp_archive)?;

        print_flush(show_output, "Extracting archive... ")?;
        let bin_path_in_archive = self.bin_path_in_archive();
        Extract::from_source(&tmp_archive_path)
            .extract_file(&tmp_dir.path(), &bin_path_in_archive)?;
        let new_exe = tmp_dir.path().join(&bin_path_in_archive);

        // Make executable
        #[cfg(not(windows))]
        {
            let mut permissions = fs::metadata(&new_exe)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&new_exe, permissions)?;
        }

        println(show_output, "Done");

        print_flush(show_output, "Replacing binary file... ")?;
        let tmp_file = tmp_dir.path().join(&format!("__{}_backup", bin_name));
        Move::from_source(&new_exe)
            .replace_using_temp(&tmp_file)
            .to_dest(&bin_install_path)?;
        println(show_output, "Done");
        Ok(UpdateStatus::Updated(release))
    }
}

// Print out message based on provided flag and flush the output buffer
fn print_flush(show_output: bool, msg: &str) -> Result<()> {
    if show_output {
        print_flush!("{}", msg);
    }
    Ok(())
}

// Print out message based on provided flag
fn println(show_output: bool, msg: &str) {
    if show_output {
        println!("{}", msg);
    }
}

// Construct a header with an authorisation entry if an auth token is provided
fn api_headers(auth_token: &Option<String>) -> header::HeaderMap {
    let mut headers = header::HeaderMap::new();

    if auth_token.is_some() {
        headers.insert(
            header::AUTHORIZATION,
            (String::from("token ") + &auth_token.clone().unwrap())
                .parse()
                .unwrap(),
        );
    };

    headers
}
