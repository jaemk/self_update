/*!
Example updating an executable to the latest version released via GitHub
*/

// For the `cargo_crate_version!` macro
#[macro_use]
extern crate self_update;

fn run() -> Result<(), Box<::std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("github")
        .show_download_progress(true)
        //.target_version_tag("v9.9.9")
        //.show_output(false)
        //.no_confirm(true)
        //.auth_token("0123456789abcdef0123456789abcdef01234567")
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}

pub fn main() {
    if let Err(e) = run() {
        println!("[ERROR] {}", e);
        ::std::process::exit(1);
    }
}
