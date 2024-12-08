pub fn main() {
    #[cfg(feature = "binary")]
    binary::build().unwrap_or_else(|e| panic!("{}", e));
}

#[cfg(feature = "binary")]
mod binary {
    use std::path::Path;

    use system_deps_meta::{
        binary::{merge_binary, Paths},
        error::{BinaryError, Error},
        parse::MetadataList,
        BUILD_MANIFEST, BUILD_TARGET_DIR,
    };

    // Add pkg-config paths to the overrides
    pub fn build() -> Result<(), Error> {
        // Read metadata from the crate graph
        let metadata = MetadataList::new(BUILD_MANIFEST, "system-deps")?;

        // Download the binaries and get their pkg_config paths
        let paths: Paths = metadata.build(merge_binary)?.into_iter().collect();

        // Write the binary paths to a file for later use
        let dest_path = Path::new(BUILD_TARGET_DIR).join("binary_config.rs");
        println!("cargo:rustc-env=BINARY_CONFIG={}", dest_path.display());
        paths
            .build(dest_path)
            .map_err(|e| BinaryError::InvalidDirectory(e).into())
    }
}
