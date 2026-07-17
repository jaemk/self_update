/*!
Example updating an executable to the latest version described by a static JSON release manifest.

`cargo run --example manifest --features "manifest archive-tar compression-tar-gz"`

Point `manifest_url` at a JSON document served over HTTP(S) that lists your releases and their
assets (the release artifacts live alongside it on the same static host, or at absolute URLs). No
server-side API is needed — regenerate the manifest at release time. See the
`self_update::backends::manifest` module docs for the schema.
*/

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let mut builder = self_update::backends::manifest::Update::configure();
    builder
        .manifest_url("https://example.net/releases/manifest.json")
        .bin_name("myapp")
        .show_download_progress(true)
        //.release_tag("v9.9.10")
        //.no_confirm(true)
        .current_version(cargo_crate_version!());

    let status = builder.build()?.update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}

pub fn main() {
    if let Err(e) = run() {
        println!("[ERROR] {}", e);
        ::std::process::exit(1);
    }
}
