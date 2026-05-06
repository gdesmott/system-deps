use std::fmt;

/// Metadata parsing errors.
#[derive(Debug)]
pub enum Error {
    /// Nested error for binary specific features.
    #[cfg(feature = "binary")]
    Binary(BinaryError),
    /// The toml object guarded by the cfg() expression is too shallow.
    CfgNotObject(String),
    /// Error while deserializing metadata.
    DeserializeError(toml::de::Error),
    /// Merging two incompatible branches.
    IncompatibleMerge,
    /// Error while parsing the cfg() expression.
    InvalidCfg(cfg_expr::ParseError),
    /// Tried to find the package but it is not in the metadata tree.
    PackageNotFound(String),
    /// Error while deserializing metadata.
    SerializeError(toml::ser::Error),
    /// The cfg() expression is valid, but not currently supported.
    UnsupportedCfg(String),
}

impl std::error::Error for Error {}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Self::DeserializeError(e)
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Self::SerializeError(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CfgNotObject(expr) => {
                write!(f, "The expression '{}' is not guarding a package", expr)
            }
            Self::DeserializeError(e) => write!(f, "Error while parsing: {}", e),
            Self::IncompatibleMerge => write!(f, "Can't merge metadata"),
            Self::PackageNotFound(pkg) => write!(f, "Package not found: {}", pkg),
            Self::SerializeError(e) => write!(f, "Error while parsing: {}", e),
            Self::InvalidCfg(e) => write!(f, "Invalid cfg() expression: {}", e),
            Self::UnsupportedCfg(expr) => {
                write!(f, "Unsupported cfg() expression: {}", expr)
            }
            #[cfg(feature = "binary")]
            Self::Binary(e) => e.fmt(f),
        }
    }
}

#[cfg(feature = "binary")]
pub use binary::BinaryError;
#[cfg(feature = "binary")]
mod binary {
    use std::{fmt, io};

    /// Binary related errors.
    #[derive(Debug)]
    pub enum BinaryError {
        /// Error while decompressing the packaged files.
        DecompressError(io::Error),
        /// The directory where the binaries should be saved already exists and is a file.
        DirectoryIsFile(String),
        /// Error while downloading from the specified URL.
        DownloadError(attohttpc::Error),
        /// The checksum for a package is incorrect.
        InvalidChecksum(String, String, String),
        /// Error in the directory where the binaries should be saved.
        InvalidDirectory(io::Error),
        /// The followed package does not exist.
        InvalidFollows(String, String),
        /// Error when using a local folder as the binary source.
        LocalFileError(io::Error),
        /// Error when creating the symlinks to the local folder.
        SymlinkError(io::Error),
        /// The download server requires browser verification (anti-bot).
        AntiBot(String),
        /// The binary archive extension is not currently supported.
        UnsupportedExtension(String),
    }

    impl From<BinaryError> for super::Error {
        fn from(e: BinaryError) -> Self {
            Self::Binary(e)
        }
    }

    impl From<attohttpc::Error> for BinaryError {
        fn from(e: attohttpc::Error) -> Self {
            Self::DownloadError(e)
        }
    }

    impl fmt::Display for BinaryError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::DecompressError(e) => {
                    write!(f, "Failed to decompress the binary archive: {}", e)
                }
                Self::DirectoryIsFile(path) => {
                    write!(f, "The binary target directory is a file: {}", path)
                }
                Self::DownloadError(e) => write!(f, "Failed to download binary archive: {}", e),
                Self::InvalidChecksum(pkg, specified, calculated) => {
                    write!(
                        f,
                        "Mismatch in the checksum of {}:\n\
                        - Specified: {}\n\
                        - Calculated: {}",
                        pkg, specified, calculated
                    )
                }
                Self::InvalidDirectory(e) => {
                    write!(f, "The binary target directory is not valid: {}", e)
                }
                Self::InvalidFollows(pkg, follows) => {
                    write!(f, "The package {} follows {}, which doesn't exist", pkg, follows)
                }
                Self::LocalFileError(e) => {
                    write!(f, "The requested local folder could not be read: {}", e)
                }
                Self::SymlinkError(e) => {
                    write!(f, "Couldn't create symlink to local binary folder: {}", e)
                }
                Self::AntiBot(url) => {
                    write!(
                        f,
                        "The server requires browser verification to download:\n\
                         \n\
                         Please download manually and use a file:// URL instead:\n\
                         \n\
                         1. Open in a browser: {url}\n\
                         2. Save the file locally\n\
                         3. Update your Cargo.toml url to: file:///path/to/downloaded/file"
                    )
                }
                Self::UnsupportedExtension(ext) => {
                    write!(f, "Unsupported binary extension: {}", ext)
                }
            }
        }
    }
}
