/*!
Example updating an executable to the latest version released via Gitee

`cargo run --example gitee --features "gitee archive-tar archive-zip compression-tar-gz compression-zip-deflate"`

Unlike Gitea, Gitee has a canonical public host (gitee.com), so the instance URL is optional and
defaults to `https://gitee.com`. Set `.host(..)` only for a self-hosted Gitee Enterprise instance.
*/

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let releases = self_update::backends::gitee::ReleaseList::configure()
        .repo_owner("myuser")
        .repo_name("myproject")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let status = self_update::backends::gitee::Update::configure()
        .repo_owner("myuser")
        .repo_name("myproject")
        .bin_name("gitee")
        .show_download_progress(true)
        //.release_tag("v9.9.10")
        //.show_output(false)
        //.no_confirm(true)
        //
        // For a self-hosted Gitee Enterprise instance, set the host.
        //.host("https://gitee.example.com")
        //
        // For private repos, provide an auth token.
        // **Make sure not to bake the token into your app**; obtain it via another mechanism,
        // such as environment variables or prompting the user for input.
        //.auth_token(&std::env::var("DOWNLOAD_AUTH_TOKEN")?)
        //
        // An optional `asset_identifier` narrows an asset match for a target / OS-arch combination.
        //.asset_identifier("gitee-bin")
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
