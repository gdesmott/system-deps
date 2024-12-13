//#![warn(missing_docs)]

pub mod error;
pub mod parse;
pub mod utils;

#[cfg(feature = "binary")]
pub mod binary;

#[cfg(any(test, feature = "test"))]
pub mod test;

/// Path to the top level Cargo.toml.
pub const BUILD_MANIFEST: &str = env!("BUILD_MANIFEST");

/// Directory where `system-deps` related build products will be stored.
pub const BUILD_TARGET_DIR: &str = env!("BUILD_TARGET_DIR");
