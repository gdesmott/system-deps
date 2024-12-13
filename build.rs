pub fn main() {
    #[cfg(feature = "binary")]
    binary::build().unwrap_or_else(|e| panic!("{}", e));
}

#[cfg(feature = "binary")]
mod binary {
    use std::{fs, path::Path};

    use system_deps_meta::{
        binary::{merge_binary, Paths},
        error::{BinaryError, Error},
        parse::read_metadata,
        BUILD_MANIFEST, BUILD_TARGET_DIR,
    };

    // Add pkg-config paths to the overrides
    pub fn build() -> Result<(), Error> {
        // Read metadata from the crate graph
        let metadata = read_metadata(BUILD_MANIFEST, "system-deps", merge_binary)?;

        // Download the binaries and get their pkg_config paths
        let paths: Paths = metadata.into_iter().collect();

        // Write the binary paths to a file for later use
        let dest = Path::new(BUILD_TARGET_DIR).join("paths.toml");
        fs::write(&dest, paths.to_string()?).map_err(BinaryError::InvalidDirectory)?;
        println!("cargo:rustc-env=BINARY_PATHS={}", dest.display());

        Ok(())
    }
}
