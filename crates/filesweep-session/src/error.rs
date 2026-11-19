//! An error crosses to the interface as a code it can act on, never as text
//! to be parsed.

use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum StartError {
    /// The running scan is left exactly as it was.
    Busy,
    NoRoots,
    TooManyRoots {
        max: usize,
    },
    /// A root has to be absolute: a desktop application has no working folder
    /// its user would know about.
    RelativeRoot {
        path: String,
    },
    Spawn {
        message: String,
    },
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => f.write_str("a scan is already running"),
            Self::NoRoots => f.write_str("no folder was given to scan"),
            Self::TooManyRoots { max } => write!(f, "more than {max} folders were given to scan"),
            Self::RelativeRoot { path } => write!(f, "the folder path is not absolute: {path}"),
            Self::Spawn { message } => {
                write!(f, "the scan thread could not be started: {message}")
            }
        }
    }
}

impl std::error::Error for StartError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum QueryError {
    StaleScan,
    NoSuchFile,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StaleScan => "the query names a scan the session no longer holds",
            Self::NoSuchFile => "the scan has no such group or file",
        })
    }
}

impl std::error::Error for QueryError {}
