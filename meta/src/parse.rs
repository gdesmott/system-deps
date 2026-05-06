use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::Path,
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use serde::Serialize;
use toml::{Table, Value};

use crate::error::Error;

/// Stores a section of metadata found in one package.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MetadataNode {
    /// Deserialized metadata.
    table: Table,
    /// The parents of this package.
    parents: BTreeSet<String>,
    /// The number of children.
    children: usize,
}

impl MetadataNode {
    /// Use the parsed metadata values to create a new node. Apply some checks.
    fn new(value: impl Serialize) -> Result<Self, Error> {
        let mut table = Table::try_from(value)?;
        filter_cfg(&mut table)?;

        Ok(Self {
            table,
            ..Default::default()
        })
    }
}

/// Recursively evaluate `cfg()` keys in a table and merge matching ones.
fn filter_cfg(table: &mut Table) -> Result<(), Error> {
    let target = get_builtin_target_by_triple(env!("SYSTEM_DEPS_TARGET"))
        .expect("The target set by the build script should be valid");
    filter_cfg_inner(table, target)
}

fn filter_cfg_inner(
    table: &mut Table,
    target: &cfg_expr::targets::TargetInfo,
) -> Result<(), Error> {
    let cfg_keys: Vec<String> = table
        .keys()
        .filter(|k| k.starts_with("cfg(") && k.ends_with(")"))
        .cloned()
        .collect();

    let mut cond = Table::new();
    for key in cfg_keys {
        let value = table.remove(&key).unwrap();
        let pred = key
            .strip_prefix("cfg(")
            .and_then(|s| s.strip_suffix(")"))
            .unwrap();
        let expr = Expression::parse(pred).map_err(Error::InvalidCfg)?;
        let res = expr.eval(|pred| match pred {
            Predicate::Target(p) => Some(p.matches(target)),
            _ => None,
        });
        if !res.ok_or(Error::UnsupportedCfg(pred.into()))? {
            continue;
        }
        let Value::Table(value) = value else {
            return Err(Error::CfgNotObject(pred.into()));
        };
        merge(&mut cond, value, false)?;
    }

    // The values in `cfg()` expressions override the default counterparts.
    merge(table, cond, true)?;

    // Recurse into sub-tables
    let keys: Vec<String> = table.keys().cloned().collect();
    for key in keys {
        if let Some(Value::Table(sub)) = table.get_mut(&key) {
            filter_cfg_inner(sub, target)?;
        }
    }

    Ok(())
}

/// Recursively read dependency manifests to find metadata matching a key using cargo_metadata.
///
/// ```toml
/// [package.metadata.section]
/// some_value = ...
/// other_value = ...
/// ```
pub fn read_metadata(
    manifest: impl AsRef<Path>,
    section: &str,
    merge: impl Fn(&mut Table, Table, bool) -> Result<(), Error>,
) -> Result<Table, Error> {
    let manifest = manifest.as_ref();
    let nodes = collect_nodes(manifest, section)?;
    reduce_nodes(nodes, &merge)
}

/// Collect metadata nodes from the dependency tree using `cargo metadata`.
fn collect_nodes(
    manifest: &Path,
    section: &str,
) -> Result<HashMap<String, MetadataNode>, Error> {
    use std::iter;

    let data = run_cargo_metadata(manifest)?;

    // Create the root node from the workspace metadata
    let value = data
        .workspace_metadata
        .get(section)
        .cloned()
        .unwrap_or_default();
    let root_node = MetadataNode::new(value).unwrap_or_default();

    // Use the root package or all the workspace packages as a starting point
    let mut packages: VecDeque<_> = if let Some(root) = data.root_package() {
        [(root, "")].into()
    } else {
        data.workspace_packages()
            .into_iter()
            .zip(iter::repeat(""))
            .collect()
    };

    let mut nodes = HashMap::from([("".to_string(), root_node)]);

    // Iterate through the dependency tree to visit all packages
    let mut visited = HashSet::new();
    while let Some((pkg, parent)) = packages.pop_front() {
        let name = pkg.name.as_str();

        // If we already handled this node, update parents and keep going
        if !visited.insert(name) {
            if let Some(node) = nodes.get_mut(name) {
                if node.parents.insert(parent.into()) {
                    if let Some(p) = nodes.get_mut(parent) {
                        p.children += 1
                    }
                }
            }
            continue;
        }

        // Keep track of the local manifests to see if they change
        if pkg
            .manifest_path
            .starts_with(manifest.parent().unwrap())
        {
            println!("cargo:rerun-if-changed={}", pkg.manifest_path);
        };

        // Get `package.metadata.section` and add it to the metadata graph
        let node = match (nodes.get_mut(name), pkg.metadata.get(section).cloned()) {
            (None, Some(s)) => {
                nodes.insert(name.into(), MetadataNode::new(s)?);
                nodes.get_mut(name)
            }
            (n, _) => n,
        };

        // Update parents
        let next_parent = if let Some(node) = node {
            if node.parents.insert(parent.into()) {
                if let Some(p) = nodes.get_mut(parent) {
                    p.children += 1
                }
            }
            name
        } else {
            parent
        };

        // Add dependencies to the queue
        for dep in &pkg.dependencies {
            if !matches!(dep.kind, DependencyKind::Normal) {
                continue;
            }
            if let Some(dep_pkg) = data
                .packages
                .iter()
                .find(|p| p.name.as_str() == dep.name.as_str())
            {
                packages.push_back((dep_pkg, next_parent));
            };
        }
    }

    Ok(nodes)
}

/// Run `cargo metadata` to obtain the full resolved dependency graph.
///
/// On Windows the previous behaviour was to walk `Cargo.toml` files manually
/// because invoking `cargo metadata` from a build script deadlocked: cargo
/// holds mandatory file locks on the build's `target` dir and a child cargo
/// blocks trying to acquire them. Running `cargo metadata` with a separate
/// `CARGO_TARGET_DIR` sidesteps that — and the manual parser was unable to
/// follow git or crates.io dependencies, so anything pulled in via `git = ...`
/// or `version = "..."` was silently dropped from the graph (the immediate
/// cause of `paths.toml` ending up empty for downstream consumers like
/// `gst-example-autodeploy` on Windows).
fn run_cargo_metadata(manifest: &Path) -> Result<cargo_metadata::Metadata, Error> {
    #[cfg(not(windows))]
    {
        MetadataCommand::new()
            .manifest_path(manifest)
            .exec()
            .map_err(|e| Error::PackageNotFound(format!("cargo metadata: {e}")))
    }

    #[cfg(windows)]
    {
        let mut cmd = MetadataCommand::new();
        cmd.manifest_path(manifest);
        let mut raw = cmd.cargo_command();
        let tmp_target = std::env::temp_dir().join("system-deps-meta-cargo-metadata");
        let _ = std::fs::create_dir_all(&tmp_target);
        raw.env("CARGO_TARGET_DIR", &tmp_target);
        let output = raw
            .output()
            .map_err(|e| Error::PackageNotFound(format!("cargo metadata: {e}")))?;
        if !output.status.success() {
            return Err(Error::PackageNotFound(format!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|e| Error::PackageNotFound(format!("cargo metadata utf8: {e}")))?;
        MetadataCommand::parse(stdout)
            .map_err(|e| Error::PackageNotFound(format!("parse cargo metadata: {e}")))
    }
}

/// Reduce the collected metadata nodes into a single table by walking
/// from leaves to root, applying merge rules along each path.
fn reduce_nodes(
    nodes: HashMap<String, MetadataNode>,
    merge: &impl Fn(&mut Table, Table, bool) -> Result<(), Error>,
) -> Result<Table, Error> {
    let mut res = Table::new();
    let mut curr = Table::new();

    // Initialize the queue from the leaves
    // TODO: Use extract_if when MSRV is bumped to 1.87
    let mut queue = VecDeque::new();
    let nodes: HashMap<&str, MetadataNode> = nodes
        .iter()
        .filter_map(|(k, v)| {
            if v.children == 0 {
                queue.push_back(v.clone());
                None
            } else {
                Some((k.as_str(), v.clone()))
            }
        })
        .collect();

    while let Some(node) = queue.pop_front() {
        // Push clones of all parents to the front of the queue so that each
        // leaf-to-root path is processed as a contiguous sequence. Nodes are
        // never removed from `nodes` because the same parent may appear in
        // multiple paths (diamond dependencies).
        for p in node.parents.iter().rev() {
            let Some(parent) = nodes.get(p.as_str()) else {
                return Err(Error::PackageNotFound(p.into()));
            };
            queue.push_front(parent.clone());
        }

        merge(&mut curr, node.table, true)?;

        if node.parents.is_empty() {
            merge(&mut res, curr, false)?;
            curr = Table::new();
        }
    }

    Ok(res)
}

/// Base merge function to use with `read_metadata`.
/// It will join values based on some assignment rules.
pub fn merge(rhs: &mut Table, lhs: Table, force: bool) -> Result<(), Error> {
    for (key, lhs) in lhs {
        // 1. None = * will always return the new value.
        let Some(rhs) = rhs.get_mut(&key) else {
            rhs.insert(key, lhs);
            continue;
        };

        // 2. If they are the same, we can stop early
        if *rhs == lhs {
            continue;
        }

        // 3. Assignment from two different types is incompatible.
        if std::mem::discriminant(rhs) != std::mem::discriminant(&lhs) {
            return Err(Error::IncompatibleMerge);
        }

        match (rhs, lhs) {
            // 4. Arrays return a combined deduplicated list.
            (Value::Array(rhs), Value::Array(lhs)) => {
                for value in lhs {
                    if !rhs.contains(&value) {
                        rhs.push(value);
                    }
                }
            }
            // 5. Tables combine keys from both following the previous rules.
            (Value::Table(rhs), Value::Table(lhs)) => {
                merge(rhs, lhs, force)?;
            }
            // 6. For simple types (Booleans, Numbers and Strings):
            //   6.1. If `force` is true, the new value will be returned.
            //   6.2. Otherwise, if the value is not the same there will be an error.
            (r, l) => {
                if !force {
                    return Err(Error::IncompatibleMerge);
                }
                *r = l;
            }
        }
    }
    Ok(())
}
