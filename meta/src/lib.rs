use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::OnceLock,
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use serde_json::{Map, Value};

pub use cargo_metadata::Metadata;
pub use serde_json::from_value;
pub type Values = Map<String, Value>;

/// Path to the top level Cargo.toml.
pub const BUILD_MANIFEST: &str = env!("BUILD_MANIFEST");

/// Directory where `system-deps` related build products will be stored.
pub const BUILD_TARGET_DIR: &str = env!("BUILD_TARGET_DIR");

/// Get metadata from every crate in the project.
fn metadata() -> &'static Metadata {
    static CACHED: OnceLock<Metadata> = OnceLock::new();
    CACHED.get_or_init(|| {
        MetadataCommand::new()
            .manifest_path(BUILD_MANIFEST)
            .exec()
            .unwrap()
    })
}

fn check_cfg(lit: &str) -> Option<bool> {
    let cfg = Expression::parse(lit).ok()?;
    let target = get_builtin_target_by_triple(&std::env::var("TARGET").ok()?)?;
    cfg.eval(|pred| match pred {
        Predicate::Target(tp) => Some(tp.matches(target)),
        _ => None,
    })
}

/// Inserts values from b into a only if they don't already exist.
/// TODO: This function could be a lot cleaner and it needs better error handling.
fn merge(a: &mut Value, b: Value) {
    match (a, b) {
        (a @ &mut Value::Object(_), Value::Object(b)) => {
            for (k, v) in b {
                // Check the cfg expressions on the tree to see if they apply
                if k.starts_with("cfg(") {
                    if check_cfg(&k).unwrap_or_default() {
                        merge(a, v);
                    }
                    continue;
                }
                let a = a.as_object_mut().unwrap();
                if let Some(e) = a.get_mut(&k) {
                    if e.is_object() {
                        merge(e, v);
                    }
                } else {
                    a.insert(k, v);
                }
            }
        }
        (a, b) => *a = b,
    }
}

/// Recursively read dependency manifests to find metadata matching a key.
/// The matching metadata is aggregated in a list, with upstream crates having priority
/// for overwriting values. It will only read from the metadata sections matching the
/// provided key.
///
/// ```toml
/// [package.metadata.key]
/// some_value = ...
/// other_value = ...
/// ```
pub fn read_metadata(key: &str) -> Values {
    let metadata = metadata();
    let project_root = PathBuf::from(BUILD_MANIFEST);
    let project_root = project_root.parent().unwrap();

    // Depending on if we are on a workspace or not, use the root package or all the
    // workspace packages as a starting point
    let mut packages = if let Some(root) = metadata.root_package() {
        VecDeque::from([root])
    } else {
        metadata.workspace_packages().into()
    };

    // Add the workspace metadata (if it exists) first
    let mut res = metadata
        .workspace_metadata
        .as_object()
        .and_then(|meta| meta.get(key))
        .cloned()
        .unwrap_or(Value::Object(Map::new()));

    // Iterate through the dependency tree to visit all packages
    let mut visited: HashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    while let Some(pkg) = packages.pop_front() {
        for dep in &pkg.dependencies {
            match dep.kind {
                DependencyKind::Normal | DependencyKind::Build => {}
                _ => continue,
            }
            if !visited.insert(&dep.name) {
                continue;
            }
            if let Some(dep_pkg) = metadata.packages.iter().find(|p| p.name == dep.name) {
                packages.push_back(dep_pkg);
            };
        }

        // Keep track of the local manifests to see if they change
        if pkg.manifest_path.starts_with(project_root) {
            println!("cargo:rerun-if-changed={}", pkg.manifest_path);
        };

        // Get the `package.metadata.key` and merge it
        let Some(meta) = pkg.metadata.as_object().and_then(|meta| meta.get(key)) else {
            continue;
        };
        merge(&mut res, meta.clone());
    }

    res.as_object().cloned().unwrap_or_default()
}
