/*!
Error type, conversions, and macros

*/
#[cfg(feature = "archive-zip")]
use zip::result::ZipError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Update(String),
    Network(String),
    Release(String),
    Config(String),
    Io(std::io::Error),
    #[cfg(feature = "archive-zip")]
    Zip(ZipError),
    Json(serde_json::Error),
    SemVer(semver::Error),
    ArchiveNotEnabled(String),
    Ureq(ureq::Error),
    NativeTls(native_tls::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        match *self {
            Update(ref s) => write!(f, "UpdateError: {}", s),
            Network(ref s) => write!(f, "NetworkError: {}", s),
            Release(ref s) => write!(f, "ReleaseError: {}", s),
            Config(ref s) => write!(f, "ConfigError: {}", s),
            Io(ref e) => write!(f, "IoError: {}", e),
            Json(ref e) => write!(f, "JsonError: {}", e),
            SemVer(ref e) => write!(f, "SemVerError: {}", e),
            #[cfg(feature = "archive-zip")]
            Zip(ref e) => write!(f, "ZipError: {}", e),
            ArchiveNotEnabled(ref s) => write!(f, "ArchiveNotEnabled: Archive extension '{}' not supported, please enable 'archive-{}' feature!", s, s),
            Ureq(ref e) => write!(f, "HTTP error: {}", e),
            NativeTls(ref e) => write!(f, "Native TLS: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        Some(match *self {
            Io(ref e) => e,
            Json(ref e) => e,
            SemVer(ref e) => e,
            Ureq(ref e) => e,
            NativeTls(ref e) => e,
            _ => return None,
        })
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Json(e)
    }
}

impl From<semver::Error> for Error {
    fn from(e: semver::Error) -> Error {
        Error::SemVer(e)
    }
}

#[cfg(feature = "archive-zip")]
impl From<ZipError> for Error {
    fn from(e: ZipError) -> Error {
        Error::Zip(e)
    }
}

impl From<ureq::Error> for Error {
    fn from(value: ureq::Error) -> Self {
        Self::Ureq(value)
    }
}

impl From<native_tls::Error> for Error {
    fn from(value: native_tls::Error) -> Self {
        Self::NativeTls(value)
    }
}
