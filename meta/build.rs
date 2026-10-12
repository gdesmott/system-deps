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
#[cfg(not(windows))]
fn find_workspace_root(dir: &Path) -> Option<PathBuf> {
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

/// Try to find the workspace root by walking up from the given directory.
/// The build script binary lives inside `target/debug/build/<pkg>-<hash>/`,
/// so walking up will eventually reach the workspace root where `Cargo.toml`
/// resides. This is used on Windows where `cargo locate-project` deadlocks
/// due to mandatory file locks held by the parent cargo process.
#[cfg(windows)]
fn find_workspace_root(dir: &Path) -> Option<PathBuf> {
    let mut dir = dir.to_path_buf();
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            return Some(manifest);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Get the manifest from the project directory. This is **not** the directory
/// where `system-deps` is cloned, it should point to the top level `Cargo.toml`
/// file. This is needed to obtain metadata from all of dependencies, including
/// those downstream of the package being compiled.
///
/// If Cargo's build-dir (the target directory by default) is not a subfolder
/// of the project it will not be possible to detect it automatically. In this
/// case, the user will be asked to specify the `SYSTEM_DEPS_BUILD_MANIFEST`
/// variable to point to it.
///
/// When `DOCS_RS` is set, this crate's own manifest is returned instead, since
/// the metadata is not read in that case and docs.rs builds may have the
/// target directory outside the project.
///
/// See https://github.com/rust-lang/cargo/issues/3946 for updates on first
/// class support for finding the workspace root.
fn manifest() -> PathBuf {
    println!("cargo:rerun-if-env-changed={}", MANIFEST_VAR);
    if let Ok(root) = env::var(MANIFEST_VAR) {
        return PathBuf::from(&root);
    }

    println!("cargo:rerun-if-env-changed=DOCS_RS");
    if env::var_os("DOCS_RS").is_some() {
        return Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
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

    find_workspace_root(&dir).unwrap_or_else(|| {
        panic!(
            "Error determining the cargo root manifest from {}.\n\
             If Cargo's build-dir (the target directory by default) is outside the project, set \
             `SYSTEM_DEPS_BUILD_MANIFEST` to the absolute path of your project's Cargo.toml",
            dir.display()
        )
    })
}

/// Set compile time values for the manifest and target paths, and the compile target.
/// This step needs to happen in a build script so that the paths are only calculated
/// once and every invocation of `system-deps` references the same metadata.
pub fn main() {
    let manifest = manifest();
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-env={}={}", MANIFEST_VAR, manifest.display());

    // TODO: Use a user cache dir, similar to .cargo?
    let target_dir = env::var(TARGET_VAR).or(env::var("OUT_DIR")).unwrap();
    println!("cargo:rerun-if-env-changed={}", TARGET_VAR);
    println!("cargo:rustc-env={}={}", TARGET_VAR, target_dir);

    println!(
        "cargo:rustc-env=SYSTEM_DEPS_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
}
