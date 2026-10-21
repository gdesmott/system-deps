//#![warn(missing_docs)]

pub mod error;
pub mod parse;

#[cfg(feature = "binary")]
pub mod binary;

#[cfg(any(test, feature = "test"))]
pub mod test;

/// Path to the top level Cargo.toml.
///
/// When `DOCS_RS` is set and `SYSTEM_DEPS_BUILD_MANIFEST` is not, it points to this crate's
/// own manifest instead, since the top level one may not be found; build scripts should not
/// read metadata from it in that case.
pub const BUILD_MANIFEST: &str = env!("SYSTEM_DEPS_BUILD_MANIFEST");

/// Directory where `system-deps` related build products will be stored.
pub const TARGET_DIR: &str = env!("SYSTEM_DEPS_TARGET_DIR");
