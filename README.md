# self_update

[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Currently only GitHub releases are supported.

```shell
self_update = "0.1"
```

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`

```rust
#[macro_use] extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    let status = self_update::backends::github::Updater::configure()?
        .repo_owner("jaemk")
        .repo_name("self_update")
        .target(&target)
        .bin_name("self_update_example")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()?;
    println!("Update status: `v{}`!", status.version());
    Ok(())
}
```


Run the above example to see `self_update` in action: `cargo run --example github`


License: MIT
