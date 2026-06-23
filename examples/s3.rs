/*!
Example updating an executable to the latest version released via an S3-compatible bucket

`cargo run --example s3 --features "archive-tar archive-zip compression-tar-gz compression-zip-deflate"`

Works with Amazon S3, Google GCS, DigitalOcean Spaces, or any S3-compatible endpoint. Releases
are matched by filename using the convention
`[directory/]<asset name>-<semver>-<platform/target>.<extension>`.

To authenticate against a private bucket, enable the `s3-auth` feature and set credentials with
`.access_key((access_key_id, secret_access_key))` (shown below, gated on the feature).
*/

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let releases = self_update::backends::s3::ReleaseList::configure()
        // .endpoint(self_update::backends::s3::Endpoint::GCS)
        // .endpoint("https://s3.example.com")
        .bucket_name("my-releases")
        .asset_prefix("myapp")
        .region("us-east-1")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let mut builder = self_update::backends::s3::Update::configure();
    builder
        // .endpoint(self_update::backends::s3::Endpoint::GCS)
        // .endpoint("https://s3.example.com")
        .bucket_name("my-releases")
        .asset_prefix("myapp")
        .region("us-east-1")
        .bin_name("myapp")
        .show_download_progress(true)
        //.release_tag("v9.9.10")
        //.no_confirm(true)
        .current_version(cargo_crate_version!());

    // Private buckets: sign requests with AWS SigV4 (requires the `s3-auth` feature).
    // **Make sure not to bake credentials into your app** — read them at runtime, e.g. from
    // the environment, as below (`access_key` accepts an `(id, secret)` pair).
    #[cfg(feature = "s3-auth")]
    {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID")?;
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY")?;
        builder.access_key((access_key_id, secret_access_key));
    }

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
