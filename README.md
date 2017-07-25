# self_update

```shell
self_update = "0.1"
```

## Usage

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


License: MIT
