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
    Reqwest(reqwest::Error),
    SemVer(semver::SemVerError),
    ArchiveNotEnabled(String),
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
            Reqwest(ref e) => write!(f, "ReqwestError: {}", e),
            SemVer(ref e) => write!(f, "SemVerError: {}", e),
            #[cfg(feature = "archive-zip")]
            Zip(ref e) => write!(f, "ZipError: {}", e),
            ArchiveNotEnabled(ref s) => write!(f, "ArchiveNotEnabled: Archive extension '{}' not supported, please enable 'archive-{}' feature!", s, s),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        "Self Update Error"
    }

    fn cause(&self) -> Option<&dyn std::error::Error> {
        use Error::*;
        Some(match *self {
            Io(ref e) => e,
            Json(ref e) => e,
            Reqwest(ref e) => e,
            SemVer(ref e) => e,
            _ => return None,
        })
    }

    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use Error::*;
        Some(match *self {
            Io(ref e) => e,
            Json(ref e) => e,
            Reqwest(ref e) => e,
            SemVer(ref e) => e,
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

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Error {
        Error::Reqwest(e)
    }
}

impl From<semver::SemVerError> for Error {
    fn from(e: semver::SemVerError) -> Error {
        Error::SemVer(e)
    }
}

#[cfg(feature = "archive-zip")]
impl From<ZipError> for Error {
    fn from(e: ZipError) -> Error {
        Error::Zip(e)
    }
}
