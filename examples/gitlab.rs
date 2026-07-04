/*!
Example updating an executable to the latest version released via Gitlab
*/

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let releases = self_update::backends::gitlab::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let status = self_update::backends::gitlab::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("gitlab")
        .show_download_progress(true)
        //.release_tag("v9.9.10")
        //.show_output(false)
        //.no_confirm(true)
        //
        // Defaults to https://gitlab.com; for a self-hosted instance, set the base URL:
        //.host("https://gitlab.mycorp.com")
        //
        // For private repos, you will need to provide an auth token
        // **Make sure not to bake the token into your app**; it is recommended
        // you obtain it via another mechanism, such as environment variables
        // or prompting the user for input
        //.auth_token(&std::env::var("DOWNLOAD_AUTH_TOKEN")?)
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
