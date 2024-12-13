use std::fmt;

/// Metadata parsing errors.
#[derive(Debug)]
pub enum Error {
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
            Self::CfgNotObject(s) => {
                write!(f, "The expression '{}' is not guarding a package", s)
            }
            Self::DeserializeError(e) => write!(f, "Error while parsing: {}", e),
            Self::IncompatibleMerge => write!(f, "Can't merge metadata"),
            Self::PackageNotFound(s) => write!(f, "Package not found: {}", s),
            Self::SerializeError(e) => write!(f, "Error while parsing: {}", e),
            Self::UnsupportedCfg(s) => {
                write!(f, "Unsupported cfg() expression: {}", s)
            }
            e => e.fmt(f),
        }
    }
}
