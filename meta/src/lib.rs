use std::{fmt, io};

mod parse;
pub use parse::*;

#[cfg(feature = "binary")]
mod binary;
#[cfg(feature = "binary")]
pub use binary::*;

#[cfg(test)]
mod test;

/// Path to the top level Cargo.toml.
pub const BUILD_MANIFEST: &str = env!("BUILD_MANIFEST");

/// Directory where `system-deps` related build products will be stored.
pub const BUILD_TARGET_DIR: &str = env!("BUILD_TARGET_DIR");

/// Metadata related errors.
#[derive(Debug)]
pub enum Error {
    // Meta
    PackageNotFound(String),
    IncompatibleMerge,
    SerializationError(serde_json::Error),
    // Binary
    DecompressError(io::Error),
    DownloadError(reqwest::Error),
    DirectoryIsFile(String),
    InvalidDirectory(io::Error),
    InvalidExtension(String),
    LocalFileError(io::Error),
    SymlinkError(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PackageNotFound(pkg) => write!(f, "Package not found {}", pkg),
            Error::IncompatibleMerge => write!(f, "Can't merge metadata"),
            Error::SerializationError(e) => write!(f, "Error while parsing: {:?}", e),
            Error::DecompressError(e) => {
                write!(f, "Error while decompressing the binaries: {:?}", e)
            }
            Error::DirectoryIsFile(dir) => write!(f, "The target directory is a file {}", dir),
            Error::InvalidDirectory(e) => write!(f, "The binary directory is not valid: {:?}", e),
            Error::InvalidExtension(url) => {
                write!(f, "Unsuppported binary extension for {}", url)
            }
            Error::LocalFileError(e) => {
                write!(f, "Error reading the local binary file: {:?}", e)
            }
            Error::SymlinkError(e) => {
                write!(f, "Error creating symlink to local binary folder: {:?}", e)
            }
            Error::DownloadError(e) => write!(f, "Error while downloading: {:?}", e),
        }
    }
}

/// Looks up an environment variable and adds it to the rerun flags.
fn env(name: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={}", name);
    std::env::var(name).ok()
}
