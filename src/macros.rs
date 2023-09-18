/// Allows you to pull the version from your Cargo.toml at compile time as
/// `MAJOR.MINOR.PATCH_PKGVERSION_PRE`
#[macro_export]
macro_rules! cargo_crate_version {
    // -- Pulled from clap.rs src/macros.rs
    () => {
        env!("CARGO_PKG_VERSION")
    };
}

/// Helper to `print!` and immediately `flush` `stdout`
macro_rules! print_flush {
    ($literal:expr) => {
        print!($literal);
        ::std::io::Write::flush(&mut ::std::io::stdout())?;
    };
    ($literal:expr, $($arg:expr),*) => {
        print!($literal, $($arg),*);
        ::std::io::Write::flush(&mut ::std::io::stdout())?;
    }
}

/// Helper for formatting `errors::Error`s
macro_rules! format_err {
    ($e_type:expr, $literal:expr) => {
        $e_type(format!($literal))
    };
    ($e_type:expr, $literal:expr, $($arg:expr),*) => {
        $e_type(format!($literal, $($arg),*))
    };
}

/// Helper for formatting `errors::Error`s and returning early
macro_rules! bail {
    ($e_type:expr, $literal:expr) => {
        return Err(format_err!($e_type, $literal))
    };
    ($e_type:expr, $literal:expr, $($arg:expr),*) => {
        return Err(format_err!($e_type, $literal, $($arg),*))
    };
}
