/*!
Error type, conversions, and macros

*/
use std;
use serde_json;
use reqwest;
use semver;

pub type Result<T> = std::result::Result<T, Error>;


#[derive(Debug)]
pub enum Error {
    Update(String),
    Config(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Reqwest(reqwest::Error),
    SemVer(semver::SemVerError),
}


impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        match *self {
            Update(ref s)   => write!(f, "UpdateError: {}", s),
            Config(ref s)   => write!(f, "ConfigError: {}", s),
            Io(ref e)       => write!(f, "IoError: {}", e),
            Json(ref e)     => write!(f, "JsonError: {}", e),
            Reqwest(ref e)  => write!(f, "ReqwestError: {}", e),
            SemVer(ref e)   => write!(f, "SemVerError: {}", e),
        }
    }
}


impl std::error::Error for Error {
    fn description(&self) -> &str {
        "Self Update Error"
    }

    fn cause(&self) -> Option<&std::error::Error> {
        use Error::*;
        Some(match *self {
            Io(ref e)           => e,
            Json(ref e)         => e,
            Reqwest(ref e)      => e,
            SemVer(ref e)       => e,
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

