# self_update

`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

Currently only GitHub releases are supported.

```shell
self_update = "0.1"
```

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`

```rust,ignore
#[macro_use] extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let target = self_update::get_target()?;
    self_update::backends::github::Updater::configure()?
        .repo_owner("jaemk")
        .repo_name("self_update")
        .target(&target)
        .bin_name("self_update_example")
        .show_progress(true)
        .current_version(crate_version!())
        .build()?
        .update()?;
}
```rust

Run the above example to see `self_update` in action: `cargo run --example github`


License: MIT
