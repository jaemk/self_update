/*!
Demonstrates the named compile-time constant pattern for embedding a signature verifying key.

Rather than inlining `[*include_bytes!(...)]` directly in the builder call (as shown in
`examples/github.rs`), this example binds the key to a named `const` first:

```rust
const VERIFYING_KEY: self_update::VerifyingKey = *include_bytes!("github-public.key");
```

This makes the key visible as a named symbol, which is useful when the same key is passed in
multiple places or when you want the compiler to enforce the expected key length at the
definition site rather than at the call site.

Run with:
`cargo run --example embedded_key --features "github signatures archive-tar compression-tar-gz"`
*/

#[cfg(feature = "signatures")]
const VERIFYING_KEY: self_update::VerifyingKey = *include_bytes!("github-public.key");

use self_update::cargo_crate_version;

fn run() -> Result<(), Box<dyn ::std::error::Error>> {
    let mut rel_builder = self_update::backends::github::ReleaseList::configure();

    #[cfg(feature = "signatures")]
    rel_builder.repo_owner("Kijewski");

    let releases = rel_builder.repo_name("self_update").build()?.fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    let mut status_builder = self_update::backends::github::Update::configure();

    #[cfg(feature = "signatures")]
    status_builder
        .repo_owner("Kijewski")
        .verify_keys([VERIFYING_KEY]);

    let status = status_builder
        .repo_name("self_update")
        .bin_name("embedded_key")
        .show_download_progress(true)
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
