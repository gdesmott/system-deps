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
#[derive(Debug, Default, Serialize)]
pub struct MetadataNode {
    /// Deserialized metadata.
    table: Table,
    /// The parents of this package.
    parents: BTreeSet<String>,
}

/// Graph like structure that stores the package nodes that have a metadata entry.
#[derive(Debug, Serialize)]
pub struct MetadataList {
    /// Stores the metadata of one package.
    pub nodes: HashMap<String, MetadataNode>,
    /// Packages without dependencies.
    pub leaves: HashSet<String>,
}

impl MetadataList {
    /// Recursively read dependency manifests to find metadata matching a key using cargo_metadata.
    ///
    /// ```toml
    /// [package.metadata.section]
    /// some_value = ...
    /// other_value = ...
    /// ```
    pub fn new(manifest: impl AsRef<Path>, section: &str) -> Result<Self, Error> {
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

        let mut res = Self {
            nodes: HashMap::from([("".into(), root_node)]),
            leaves: HashSet::from(["".into()]),
        };

        // Iterate through the dependency tree to visit all packages
        let mut visited = HashSet::new();
        while let Some((pkg, parent)) = packages.pop_front() {
            // If we already handled this node, update parents and keep going
            if !visited.insert(&pkg.name) {
                if let Some(node) = res.nodes.get_mut(&pkg.name) {
                    node.parents.insert(parent.into());
                    res.leaves.remove(parent);
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
            let node = match (
                res.nodes.get_mut(&pkg.name),
                pkg.metadata.get(section).cloned(),
            ) {
                (None, Some(s)) => {
                    let node = MetadataNode {
                        table: reduce(Table::try_from(s)?)?,
                        ..Default::default()
                    };
                    res.leaves.insert(pkg.name.clone());
                    res.nodes.insert(pkg.name.clone(), node);
                    res.nodes.get_mut(&pkg.name)
                }
                (n, _) => n,
            };

            // Update parents
            let next_parent = if let Some(node) = node {
                node.parents.insert(parent.into());
                res.leaves.remove(parent);
                pkg.name.as_str()
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

        Ok(res)
    }

    /// Applies reducing rules to the tree and returns the final value. It first searchs one branch from the
    /// package to the project root. Then, it backtracks through the rest of the paths. If the final values
    /// from each path are incompatible, a merge error is returned.
    ///
    /// `merge` is a function that takes the current value, the new value that should be applied to it, and
    /// whether it should allow the second value to overwrite the first. When traveling up the tree this is
    /// true since we want dependent crates to have priority, but when comparing horizontally it is false to
    /// avoid conflicts.
    pub fn build(
        &self,
        merge: impl Fn(&mut Table, Table, bool) -> Result<(), Error>,
    ) -> Result<Table, Error> {
        let mut res = Table::new();

        for node in &self.leaves {
            let value = self.get(&merge, node)?;
            merge(&mut res, value, false)?;
        }

        Ok(res)
    }

    /// Helper for `build` that gets a single package branch.
    fn get(
        &self,
        merge: impl Fn(&mut Table, Table, bool) -> Result<(), Error>,
        package: &str,
    ) -> Result<Table, Error> {
        let base = self
            .nodes
            .get(package)
            .ok_or(Error::PackageNotFound(package.into()))?;

        let mut nodes = VecDeque::from([base]);
        let mut res = Table::new();
        let mut curr = Table::new();

        while let Some(node) = nodes.pop_front() {
            for p in node.parents.iter().rev() {
                let next = self.nodes.get(p).ok_or(Error::PackageNotFound(p.into()))?;
                nodes.push_front(next);
            }
            let value = reduce(node.table.clone())?;
            merge(&mut curr, value, true)?;
            if node.parents.is_empty() {
                merge(&mut res, curr, false)?;
                curr = Table::new();
            }
        }

        Ok(res)
    }
}
