# self_update


[![Build status](https://ci.appveyor.com/api/projects/status/xlkq8rd73cla4ixw/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/self-update/branch/master)
[![Build Status](https://travis-ci.org/jaemk/self_update.svg?branch=master)](https://travis-ci.org/jaemk/self_update)
[![crates.io:clin](https://img.shields.io/crates/v/self_update.svg?label=self_update)](https://crates.io/crates/self_update)
[![docs](https://docs.rs/self_update/badge.svg)](https://docs.rs/self_update)


`self_update` provides updaters for updating rust executables in-place from various release
distribution backends.

```shell
self_update = "0.5"
```

## Usage

Update (replace) the current executable with the latest release downloaded
from `https://api.github.com/repos/jaemk/self_update/releases/latest`.
Note, the [`trust`](https://github.com/japaric/trust) project provides a nice setup for
producing release-builds via CI (travis/appveyor).


```rust
#[macro_use] extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .bin_name("self_update_example")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        // For private repos, you will need to provide a GitHub auth token
        // Make sure not to bake the token into your app; it is recommended
        // you obtain it via another mechanism, such as environment variables
        // or prompting the user for input
        //.auth_token(env!("ACCESS_TOKEN"))
        .build()?
        .update()?;
    println!("Update status: `{}`!", status.version());
    Ok(())
}
```

Run the above example to see `self_update` in action: `cargo run --example github`

Separate utilities are also exposed:

```rust
extern crate self_update;

fn update() -> Result<(), Box<::std::error::Error>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("jaemk")
        .repo_name("self_update")
        .build()?
        .fetch()?;
    println!("found releases:");
    println!("{:#?}\n", releases);

    // get the first available release
    let asset = releases[0]
        .asset_for(&self_update::get_target()).unwrap();

    let tmp_dir = self_update::TempDir::new_in(::std::env::current_dir()?, "self_update")?;
    let tmp_tarball_path = tmp_dir.path().join(&asset.name);
    let tmp_tarball = ::std::fs::File::open(&tmp_tarball_path)?;

    self_update::Download::from_url(&asset.download_url)
        .download_to(&tmp_tarball)?;

    let bin_name = std::path::PathBuf::from("self_update_bin");
    self_update::Extract::from_source(&tmp_tarball_path)
        .archive(self_update::ArchiveKind::Tar(Some(self_update::Compression::Gz)))
        .extract_file(&tmp_dir.path(), &bin_name)?;

    let tmp_file = tmp_dir.path().join("replacement_tmp");
    let bin_path = tmp_dir.path().join(bin_name);
    self_update::Move::from_source(&bin_path)
        .replace_using_temp(&tmp_file)
        .to_dest(&::std::env::current_exe()?)?;

    Ok(())
}
```


License: MIT
