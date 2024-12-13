use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    iter,
    path::Path,
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use serde::Serialize;
use toml::Table;

use crate::{error::Error, utils::reduce};

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
    let data = MetadataCommand::new()
        .manifest_path(manifest.as_ref())
        .exec()
        .unwrap();

    // Create the root node from the workspace metadata
    let value = data.workspace_metadata.get(section).cloned();
    let root_node = MetadataNode {
        table: reduce(
            value
                .and_then(|v| Table::try_from(v).ok())
                .unwrap_or_default(),
        )?,
        ..Default::default()
    };

    // Use the root package or all the workspace packages as a starting point
    let mut packages: VecDeque<_> = if let Some(root) = data.root_package() {
        [(root, "")].into()
    } else {
        data.workspace_packages()
            .into_iter()
            .zip(iter::repeat(""))
            .collect()
    };

    let mut nodes = HashMap::from([("", root_node)]);

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
            .starts_with(manifest.as_ref().parent().unwrap())
        {
            println!("cargo:rerun-if-changed={}", pkg.manifest_path);
        };

        // Get `package.metadata.section` and add it to the metadata graph
        let node = match (nodes.get_mut(name), pkg.metadata.get(section).cloned()) {
            (None, Some(s)) => {
                let node = MetadataNode {
                    table: reduce(Table::try_from(s)?)?,
                    ..Default::default()
                };
                nodes.insert(name, node);
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
            if let Some(dep_pkg) = data.packages.iter().find(|p| p.name == dep.name) {
                packages.push_back((dep_pkg, next_parent));
            };
        }
    }

    // Now that the tree is built, apply the reducing rules
    let mut res = Table::new();
    let mut curr = Table::new();

    // Initialize the queue from the leaves
    // NOTE: Use `extract_if` when it is available https://github.com/rust-lang/rust/issues/43244
    let mut queue = VecDeque::new();
    let mut nodes: HashMap<&str, MetadataNode> = nodes
        .into_iter()
        .filter_map(|(k, v)| {
            if v.children == 0 {
                queue.push_back(v);
                None
            } else {
                Some((k, v))
            }
        })
        .collect();

    while let Some(node) = queue.pop_front() {
        // Push the parents to the queue, avoid unnecessary clones
        for p in node.parents.iter().rev() {
            let Some(parent) = nodes.get_mut(p.as_str()) else {
                return Err(Error::PackageNotFound(p.into()));
            };
            let next = if parent.children.checked_sub(1).is_some() {
                parent.clone()
            } else {
                nodes.remove(p.as_str()).expect("Already checked")
            };
            queue.push_front(next);
        }

        let reduced = reduce(node.table)?;
        merge(&mut curr, reduced, true)?;

        if node.parents.is_empty() {
            merge(&mut res, curr, false)?;
            curr = Table::new();
        }
    }

    Ok(res)
}
