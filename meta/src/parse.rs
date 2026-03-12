use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    iter,
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
        let mut table = Table::new();
        let mut cond = Table::new();

        for (key, value) in Table::try_from(value)? {
            // If the key is a `cfg()` expression, check if it applies and merge the inner part.
            if let Some(pred) = key.strip_prefix("cfg(").and_then(|s| s.strip_suffix(")")) {
                let target = get_builtin_target_by_triple(env!("TARGET"))
                    .expect("The target set by the build script should be valid");
                let expr = Expression::parse(pred).map_err(Error::InvalidCfg)?;
                let res = expr.eval(|pred| match pred {
                    Predicate::Target(p) => Some(p.matches(target)),
                    _ => None,
                });
                if !res.ok_or(Error::UnsupportedCfg(pred.into()))? {
                    continue;
                };
                let Value::Table(value) = value else {
                    return Err(Error::CfgNotObject(pred.into()));
                };
                merge(&mut cond, value, false)?;
                continue;
            }
            // Regular case
            table.insert(key, value);
        }

        // The values in `cfg()` expressions override the default counterparts.
        merge(&mut table, cond, true)?;

        Ok(Self {
            table,
            ..Default::default()
        })
    }
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
                nodes.insert(name, MetadataNode::new(s)?);
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
