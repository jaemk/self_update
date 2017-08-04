/*!
Example updating an executable to the latest version released via GitHub
*/

// For the `cargo_crate_version!` macro
#[macro_use] extern crate self_update;


fn run() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    let status = self_update::backends::github::Update::configure()?
        .repo_owner("jaemk")
        .repo_name("self_update")
        .target(&target)
        .bin_name("github")
        .show_download_progress(true)
        //.show_output(false)
        //.no_confirm(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `v{}`!", status.version());
    Ok(())
}

pub fn main() {
    if let Err(e) = run() {
        println!("[ERROR] {}", e);
        ::std::process::exit(1);
    }
}
