use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    path::Path,
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{from_value, Value};

use crate::{error::Error, utils::merge_base};

/// Stores a section of metadata found in one package.
#[derive(Debug, Default, Serialize)]
pub struct MetadataNode {
    value: Value,
    parents: BTreeSet<String>,
}

/// Graph like structure that stores the package nodes that have a metadata entry.
#[derive(Debug, Serialize)]
pub struct MetadataList {
    pub nodes: HashMap<String, MetadataNode>,
}

impl MetadataList {
    /// Recursively read dependency manifests to find metadata matching a key using cargo_metadata.
    ///
    /// ```toml
    /// [package.metadata.section]
    /// some_value = ...
    /// other_value = ...
    /// ```
    pub fn from(manifest: impl AsRef<Path>, section: &str) -> Self {
        let data = MetadataCommand::new()
            .manifest_path(manifest.as_ref())
            .exec()
            .unwrap();

        // Create the root node from the workspace metadata
        let value = data
            .workspace_metadata
            .as_object()
            .and_then(|m| m.get(section))
            .cloned()
            .unwrap_or(Value::Null);
        let root_node = MetadataNode {
            value,
            ..Default::default()
        };

        // Use the root package or all the workspace packages as a starting point
        let mut packages = if let Some(root) = data.root_package() {
            VecDeque::from([(root, "")])
        } else {
            data.workspace_packages()
                .into_iter()
                .map(|p| (p, ""))
                .collect()
        };

        let mut res = Self {
            nodes: HashMap::from([("".into(), root_node)]),
        };

        // Iterate through the dependency tree to visit all packages
        let mut visited = HashSet::new();
        while let Some((pkg, parent)) = packages.pop_front() {
            // TODO: Optional packages

            // If we already handled this node, update parents and keep going
            if !visited.insert(&pkg.name) {
                res.nodes
                    .get_mut(&pkg.name)
                    .map(|n| n.parents.insert(parent.into()));
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
            let node = if let Some(section) =
                pkg.metadata.as_object().and_then(|meta| meta.get(section))
            {
                Some(
                    res.nodes
                        .entry(pkg.name.clone())
                        .or_insert_with(|| MetadataNode {
                            value: section.clone(),
                            ..Default::default()
                        }),
                )
            } else {
                res.nodes.get_mut(&pkg.name)
            };

            let next_parent = if node.is_some() {
                pkg.name.as_str()
            } else {
                parent
            };

            node.map(|p| p.parents.insert(parent.into()));

            // Add dependencies to the queue
            for dep in &pkg.dependencies {
                match dep.kind {
                    DependencyKind::Normal | DependencyKind::Build => {}
                    _ => continue,
                }

                if let Some(dep_pkg) = data.packages.iter().find(|p| p.name == dep.name) {
                    packages.push_back((dep_pkg, next_parent));
                };
            }
        }

        res
    }

    /// Applies reducing rules to the tree and returns the final value transformed to the desired type.
    /// It first searchs one branch from the package to the project root. Then, it backtracks through
    /// the rest of the paths. If the final values from each path are incompatible, a merge error
    /// is returned.
    ///
    /// `merge` is a function that takes the current value, the new value that should be applied to it,
    /// and whether it should allow the second value to overwrite the first. When traveling up the tree
    /// this is true since we want dependent crates to have priority, but when comparing horizontally
    /// it is false to avoid conflicts.
    pub fn get<T: DeserializeOwned>(
        &self,
        package: &str,
        merge: impl Fn(Value, Value, bool) -> Result<Value, Error>,
    ) -> Result<T, Error> {
        let base = self
            .nodes
            .get(package)
            .ok_or(Error::PackageNotFound(package.into()))?;

        let mut nodes = VecDeque::from([base]);
        let mut res = Value::Null;
        let mut curr = Value::Null;

        while let Some(node) = nodes.pop_front() {
            for p in node.parents.iter().rev() {
                let next = self.nodes.get(p).ok_or(Error::PackageNotFound(p.into()))?;
                nodes.push_front(next);
            }
            curr = merge_base(curr, &node.value, true, &merge)?;
            if node.parents.is_empty() {
                res = merge_base(res, &curr, false, &merge)?;
                curr = Value::Null;
            }
        }

        from_value::<T>(res).map_err(Error::SerializationError)
    }
}
