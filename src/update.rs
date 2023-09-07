use reqwest::{self, header};
use std::fs;
use std::path::PathBuf;

use crate::{confirm, errors::*, version, Download, Extract, Status};

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
        matches!(*self, UpdateStatus::UpToDate)
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
    /// contains the specified `target` and possibly `identifier`
    pub fn asset_for(&self, target: &str, identifier: Option<&str>) -> Option<ReleaseAsset> {
        self.assets
            .iter()
            .find(|asset| {
                asset.name.contains(target)
                    && if let Some(i) = identifier {
                        asset.name.contains(i)
                    } else {
                        true
                    }
            })
            .cloned()
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

    /// Optional identifier of determining the asset among multiple matches
    fn identifier(&self) -> Option<String> {
        None
    }

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

    // message template to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    fn progress_template(&self) -> String;

    // progress_chars to use if `show_download_progress` is set (see `indicatif::ProgressStyle`)
    fn progress_chars(&self) -> String;

    /// Authorisation token for communicating with backend
    fn auth_token(&self) -> Option<String>;

    #[cfg(feature = "signatures")]
    fn verifying_keys(&self) -> &[[u8; ed25519_dalek::PUBLIC_KEY_LENGTH]];

    /// Construct a header with an authorisation entry if an auth token is provided
    fn api_headers(&self, auth_token: &Option<String>) -> Result<header::HeaderMap> {
        let mut headers = header::HeaderMap::new();

        if auth_token.is_some() {
            headers.insert(
                header::AUTHORIZATION,
                (String::from("token ") + &auth_token.clone().unwrap())
                    .parse()
                    .unwrap(),
            );
        };

        Ok(headers)
    }

    /// Display release information and update the current binary to the latest release, pending
    /// confirmation from the user
    fn update(&self) -> Result<Status> {
        let current_version = self.current_version();
        self.update_extended()
            .map(|s| s.into_status(current_version))
    }

    /// Same as `update`, but returns `UpdateStatus`.
    fn update_extended(&self) -> Result<UpdateStatus> {
        let bin_install_path = self.bin_install_path();
        let bin_name = self.bin_name();

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

        let target_asset = release
            .asset_for(&target, self.identifier().as_deref())
            .ok_or_else(|| {
                format_err!(Error::Release, "No asset found for target: `{}`", target)
            })?;

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

        let tmp_archive_dir = tempfile::TempDir::new()?;
        let tmp_archive_path = tmp_archive_dir.path().join(&target_asset.name);
        let mut tmp_archive = fs::File::create(&tmp_archive_path)?;

        println(show_output, "Downloading...");
        let mut download = Download::from_url(&target_asset.download_url);
        let mut headers = self.api_headers(&self.auth_token())?;
        headers.insert(header::ACCEPT, "application/octet-stream".parse().unwrap());
        download.set_headers(headers);
        download.show_progress(self.show_download_progress());

        download.progress_template = self.progress_template();
        download.progress_chars = self.progress_chars();

        download.download_to(&mut tmp_archive)?;

        print_flush(show_output, "Extracting archive... ")?;
        let bin_path_in_archive = self.bin_path_in_archive();
        Extract::from_source(&tmp_archive_path)
            .extract_file(tmp_archive_dir.path(), &bin_path_in_archive)?;
        let new_exe = tmp_archive_dir.path().join(&bin_path_in_archive);

        #[cfg(feature = "signatures")]
        {
            use std::io::Read;

            let verifying_keys = self.verifying_keys();
            if !verifying_keys.is_empty() {
                // TODO: FIXME: this only works for signed .zip files, not .tar
                let mut signature = [0; ed25519_dalek::SIGNATURE_LENGTH];
                fs::File::open(&tmp_archive_path)?.read_exact(&mut signature)?;
                let signature = ed25519_dalek::Signature::from_bytes(&signature);

                let exe = fs::File::open(&new_exe)?;
                let exe = unsafe { memmap2::Mmap::map(&exe)? };

                let mut valid_signature = false;
                for (idx, bytes) in verifying_keys.into_iter().enumerate() {
                    let key = match ed25519_dalek::VerifyingKey::from_bytes(&bytes) {
                        Ok(key) => key,
                        Err(_) => panic!("Key #{} is invalid", idx),
                    };
                    if key.verify_strict(&exe, &signature).is_ok() {
                        valid_signature = true;
                        break;
                    }
                }
                if !valid_signature {
                    return Err(Error::NoValidSignature);
                }
            }
        }

        println(show_output, "Done");

        print_flush(show_output, "Replacing binary file... ")?;
        self_replace::self_replace(new_exe)?;
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
