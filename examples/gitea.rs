/*!
Example updating an executable to the latest version released via Gitea

`cargo run --example gitea --features "archive-tar archive-zip compression-flate2 compression-zip-deflate"`

Unlike GitHub/GitLab, Gitea has no canonical public host, so the instance URL is required —
set it with `.url(..)`.
*/

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let releases = self_update::backends::gitea::ReleaseList::configure()
        .url("https://gitea.example.com")
        .repo_owner("myuser")
        .repo_name("myproject")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let status = self_update::backends::gitea::Update::configure()
        .url("https://gitea.example.com")
        .repo_owner("myuser")
        .repo_name("myproject")
        .bin_name("gitea")
        .show_download_progress(true)
        //.release_tag("v9.9.10")
        //.show_output(false)
        //.no_confirm(true)
        //
        // For private repos, provide an auth token.
        // **Make sure not to bake the token into your app**; obtain it via another mechanism,
        // such as environment variables or prompting the user for input.
        //.auth_token(&std::env::var("DOWNLOAD_AUTH_TOKEN")?)
        //
        // An optional `asset_identifier` narrows an asset match for a target / OS-arch combination.
        //.asset_identifier("gitea-bin")
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
