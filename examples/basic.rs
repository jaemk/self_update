/*!
Example updating an executable to the latest version released via GitHub
*/

// For the `crate_version!` macro
#[macro_use] extern crate self_update;


fn run() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    self_update::backends::github::Updater::configure()?
        .repo_owner("jaemk")
        .repo_name("clin")
        .target(&target)
        .bin_name("clin")
        .show_progress(true)
        .current_version(crate_version!())
        .build()?
        .update()?;
    Ok(())
}

pub fn main() {
    if let Err(e) = run() {
        eprintln!("[ERROR] {}", e);
        ::std::process::exit(1);
    }
}
