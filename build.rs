pub fn main() {
    #[cfg(feature = "binary")]
    binary::build();
}

#[cfg(feature = "binary")]
mod binary {
    use std::{collections::HashMap, fs, path::Path};

    use system_deps_meta::{read_metadata, Binary, BinaryPaths, BUILD_TARGET_DIR};

    pub fn build() {
        // Add pkg-config paths to the overrides
        // TODO: This should probably follow some deterministic ordering to avoid issues

        let dest_path = Path::new(BUILD_TARGET_DIR).join("binary_config.rs");
        println!("cargo:rustc-env=BINARY_CONFIG={}", dest_path.display());

        // Read metadata from the crate graph
        let metadata = read_metadata(system_deps_meta::BUILD_MANIFEST, "system-deps");

        // Download the binaries and get their pkg_config paths
        let mut binaries = metadata
            .get::<HashMap<String, Binary>>(|| true)
            .unwrap_or_default();
        println!("cargo:warning=BINARIES {:?}", binaries);
        let paths = BinaryPaths::from(binaries.drain());
        println!("cargo:warning=PATHS {:?}", paths);

        fs::write(dest_path, paths.build()).expect("Error when writing binary config");
    }
}
