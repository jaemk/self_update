/*!
Example updating an executable to the latest version released via GitHub
*/

// For the `cargo_crate_version!` macro
#[macro_use]
extern crate self_update;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("Kijewski")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let status = self_update::backends::github::Update::configure()
        .repo_owner("Kijewski")
        .repo_name("self_update")
        .bin_name("github")
        .show_download_progress(true)
        //.target_version_tag("v9.9.10")
        //.show_output(false)
        //.no_confirm(true)
        //
        // For private repos, you will need to provide a GitHub auth token
        // **Make sure not to bake the token into your app**; it is recommended
        // you obtain it via another mechanism, such as environment variables
        // or prompting the user for input
        //.auth_token(env!("DOWNLOAD_AUTH_TOKEN"))
        .current_version(cargo_crate_version!())
        .verifying_keys([*include_bytes!("github-public.key")])
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
