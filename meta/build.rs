use std::{
    env,
    path::{Path, PathBuf},
};

/// Environment variable to override the top level `Cargo.toml`.
const MANIFEST_VAR: &str = "SYSTEM_DEPS_BUILD_MANIFEST";

/// Environment variable to override the directory where `system-deps`
/// will store build products such as binary outputs.
const TARGET_VAR: &str = "SYSTEM_DEPS_TARGET_DIR";

/// Try to find the project root using locate-project
fn find_with_cargo(dir: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new(env!("CARGO"))
        .current_dir(dir)
        .arg("locate-project")
        .arg("--workspace")
        .arg("--message-format=plain")
        .output()
        .ok()?
        .stdout;
    if out.is_empty() {
        return None;
    }
    Some(PathBuf::from(std::str::from_utf8(&out).ok()?.trim()))
}

/// Get the manifest from the project directory. This is **not** the directory
/// where `system-deps` is cloned, it should point to the top level `Cargo.toml`
/// file. This is needed to obtain metadata from all of dependencies, including
/// those downstream of the package being compiled.
///
/// If the target directory is not a subfolder of the project it will not be
/// possible to detect it automatically. In this case, the user will be asked
/// to specify the `SYSTEM_DEPS_MANIFEST` variable to point to it.
///
/// See https://github.com/rust-lang/cargo/issues/3946 for updates on first
/// class support for finding the workspace root.
fn manifest() -> PathBuf {
    println!("cargo:rerun-if-env-changed={}", MANIFEST_VAR);
    if let Ok(root) = env::var(MANIFEST_VAR) {
        return PathBuf::from(&root);
    }

    // When build scripts are invoked, they have one argument pointing to the
    // build path of the crate in the target directory. This is different than
    // the `OUT_DIR` environment variable, that can point to a target directory
    // where the checkout of the dependency is.
    let mut dir = PathBuf::from(
        std::env::args()
            .next()
            .expect("There should be cargo arguments for determining the root"),
    );
    dir.pop();

    // Try to find the project with cargo
    find_with_cargo(&dir).expect(
        "Error determining the cargo root manifest.\n\
         Please set `SYSTEM_DEPS_MANIFEST` to the path of your project's Cargo.toml",
    )
}

/// Set compile time values for the manifest and target paths, and the compile target.
/// Calculating this in a build script is necessary so that they are only calculated
/// once and every invocation of `system-deps` references the same metadata.
pub fn main() {
    let manifest = manifest();
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-env=BUILD_MANIFEST={}", manifest.display());

    let target_dir = env::var(TARGET_VAR).or(env::var("OUT_DIR")).unwrap();
    println!("cargo:rerun-if-env-changed={}", TARGET_VAR);
    println!("cargo:rustc-env=BUILD_TARGET_DIR={}", target_dir);

    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
