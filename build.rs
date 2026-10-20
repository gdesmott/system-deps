pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "binary")]
    binary::build()?;
    Ok(())
}

#[cfg(feature = "binary")]
mod binary {
    use std::{fs, path::Path};

    use heck::ToShoutySnakeCase;
    use system_deps_meta::{
        binary::{merge, Paths},
        error::{BinaryError, Error},
        parse::read_metadata,
        BUILD_MANIFEST,
    };

    // Add pkg-config paths to the overrides
    pub fn build() -> Result<(), Error> {
        println!("cargo:rerun-if-env-changed=DOCS_RS");
        let docs_rs = std::env::var_os("DOCS_RS").is_some();
        let paths = if docs_rs || no_prebuilt("SYSTEM_DEPS_NO_PREBUILT") {
            Paths::default()
        } else {
            // Read metadata from the crate graph
            let metadata = read_metadata(BUILD_MANIFEST, "system-deps", merge)?;

            // Skip the binaries disabled with `SYSTEM_DEPS_$NAME_NO_PREBUILT`, and those following them
            let skipped: Vec<String> = metadata
                .keys()
                .filter(|name| {
                    no_prebuilt(&format!(
                        "SYSTEM_DEPS_{}_NO_PREBUILT",
                        name.to_shouty_snake_case()
                    ))
                })
                .cloned()
                .collect();
            let metadata = metadata.into_iter().filter(|(name, value)| {
                let follows = value.get("follows").and_then(|f| f.as_str());
                !skipped.contains(name) && !follows.is_some_and(|f| skipped.iter().any(|s| s == f))
            });

            // Download the binaries and get their pkg_config paths
            Paths::from_binaries(metadata)?
        };

        // Write the binary paths to a file for later use
        let dest = Path::new(&std::env::var_os("OUT_DIR").unwrap()).join("paths.toml");
        fs::write(&dest, paths.to_string()?).map_err(BinaryError::InvalidDirectory)?;
        println!(
            "cargo:rustc-env=SYSTEM_DEPS_BINARY_PATHS={}",
            dest.display()
        );

        // On Windows the test/run binaries need the prebuilt
        // bin directory on PATH at runtime: rustc-link-search only affects
        // linking, and Cargo doesn't set PATH for executed test binaries.
        // Surface the directories so the user can prepend them.
        #[cfg(windows)]
        {
            let dirs = paths.dll_search_dirs();
            if !dirs.is_empty() {
                let joined =
                    std::env::join_paths(&dirs).expect("DLL search dir contains invalid character");
                let joined = joined.to_string_lossy();
                println!(
                    "cargo:warning=To run tests/binaries, copy & paste (PowerShell): $env:PATH = \"{};$env:PATH\"",
                    joined
                );
                println!(
                    "cargo:warning=To run tests/binaries, copy & paste (cmd.exe):   set \"PATH={};%PATH%\"",
                    joined
                );
            }
        }

        Ok(())
    }

    fn no_prebuilt(var: &str) -> bool {
        println!("cargo:rerun-if-env-changed={}", var);
        std::env::var(var).is_ok()
    }
}
