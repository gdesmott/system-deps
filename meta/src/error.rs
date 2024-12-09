use std::fmt;

/// Metadata related errors
#[derive(Debug)]
pub enum Error {
    #[cfg(feature = "binary")]
    BinaryError(BinaryError),
    CfgError(CfgError),
    IncompatibleMerge,
    MergeBase(serde_json::Value),
    PackageNotFound(String),
    SerializationError(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleMerge => write!(f, "Can't merge metadata"),
            Self::PackageNotFound(pkg) => write!(f, "Package not found {}", pkg),
            Self::SerializationError(e) => write!(f, "Error while parsing: {:?}", e),
            e => e.fmt(f),
        }
    }
}

/// Conditional expression errors
#[derive(Debug)]
pub enum CfgError {
    Invalid(cfg_expr::ParseError),
    NotObject,
    Unsupported(String),
}

impl From<CfgError> for Error {
    fn from(e: CfgError) -> Self {
        Self::CfgError(e)
    }
}

impl fmt::Display for CfgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "Invalid conditional directive: {:?}", e),
            Self::NotObject => write!(f, "The conditional expression is not an object"),
            Self::Unsupported(cfg) => {
                write!(f, "Unsupported conditional expression '{}'", cfg)
            }
        }
    }
}

#[cfg(feature = "binary")]
pub use binary::BinaryError;
#[cfg(feature = "binary")]
mod binary {
    use std::{fmt, io};

    /// Binary related errors
    #[derive(Debug)]
    pub enum BinaryError {
        DecompressError(io::Error),
        DownloadError(reqwest::Error),
        DirectoryIsFile(String),
        InvalidDirectory(io::Error),
        InvalidFollows(String, String),
        LocalFileError(io::Error),
        SymlinkError(io::Error),
        UnsupportedExtension(String),
    }

    impl From<BinaryError> for super::Error {
        fn from(e: BinaryError) -> Self {
            Self::BinaryError(e)
        }
    }

    impl fmt::Display for BinaryError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::DecompressError(e) => {
                    write!(f, "Error while decompressing the binaries: {:?}", e)
                }
                Self::DirectoryIsFile(dir) => write!(f, "The target directory is a file {}", dir),
                Self::InvalidDirectory(e) => {
                    write!(f, "The binary directory is not valid: {:?}", e)
                }
                Self::InvalidFollows(from, to) => {
                    write!(f, "The package {} follows {} which is invalid", from, to)
                }
                Self::LocalFileError(e) => {
                    write!(f, "Error reading the local binary file: {:?}", e)
                }
                Self::SymlinkError(e) => {
                    write!(f, "Error creating symlink to local binary folder: {:?}", e)
                }
                Self::DownloadError(e) => write!(f, "Error while downloading: {:?}", e),
                Self::UnsupportedExtension(url) => {
                    write!(f, "Unsupported binary extension for {}", url)
                }
            }
        }
    }
}
