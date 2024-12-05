use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

use cargo_metadata::{DependencyKind, MetadataCommand};
use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{from_value, Value};

use crate::Error;

/// Stores a section of metadata found in one package.
/// `next` indexes the  to packages downstream from this.
#[derive(Debug, Default, Serialize)]
pub struct MetadataNode {
    value: Value,
    parents: HashSet<String>,
}

/// Graph like structure that stores the package nodes that have a metadata entry.
#[derive(Debug, Serialize)]
pub struct MetadataList {
    nodes: HashMap<String, MetadataNode>,
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
            VecDeque::from([(root, None)])
        } else {
            data.workspace_packages()
                .into_iter()
                .map(|p| (p, None))
                .collect()
        };

        let mut res = Self {
            nodes: HashMap::from([("".into(), root_node)]),
        };

        // Iterate through the dependency tree to visit all packages
        let mut visited: HashSet<&str> = packages.iter().map(|(p, _)| p.name.as_str()).collect();
        while let Some((pkg, parent)) = packages.pop_front() {
            // TODO: Optional packages

            // Keep track of the local manifests to see if they change
            if pkg
                .manifest_path
                .starts_with(manifest.as_ref().parent().unwrap())
            {
                println!("cargo:rerun-if-changed={}", pkg.manifest_path);
            };

            // Get `package.metadata.section` and add it to the metadata graph
            let section = pkg.metadata.as_object().and_then(|meta| meta.get(section));
            res.insert(
                pkg.name.clone(),
                parent.unwrap_or_default(),
                section.cloned().unwrap_or(Value::Null),
            );
            let next_parent = section
                .and(Some(pkg.name.as_str()))
                .or(Some(parent.unwrap_or_default()));

            // Add dependencies to the queue
            for dep in &pkg.dependencies {
                match dep.kind {
                    DependencyKind::Normal | DependencyKind::Build => {}
                    _ => continue,
                }

                // If visited, don't keep going, but add dependencies to graph
                if !visited.insert(&dep.name) {
                    res.insert(pkg.name.clone(), parent.unwrap_or_default(), Value::Null);
                    continue;
                }

                if let Some(dep_pkg) = data.packages.iter().find(|p| p.name == dep.name) {
                    packages.push_back((dep_pkg, next_parent));
                };
            }
        }

        res
    }

    fn insert(&mut self, name: String, parent: &str, value: Value) {
        // If no value is provided only add the node to the parent if it already exists
        if value.is_null() && !self.nodes.contains_key(&name) {
            return;
        }

        self.nodes
            .entry(name.clone())
            .or_insert_with(|| MetadataNode {
                value,
                ..Default::default()
            })
            .parents
            .insert(parent.into());
    }

    /// Applies the reducing rules to the tree and returns the final value transformed to the desired type
    /// Starts on a node and goes downstream to its parents
    pub fn get<T: DeserializeOwned>(
        &self,
        package: &str,
        merge: impl Fn(Value, &Value) -> Result<Value, Error>,
    ) -> Result<T, Error> {
        let base = self
            .nodes
            .get(package)
            .ok_or(Error::PackageNotFound(package.into()))?;

        let mut nodes = VecDeque::from([base]);
        let mut res = Value::Null;

        // TODO: Backtrack and handle conflicts
        // TODO: cfg

        while let Some(node) = nodes.pop_front() {
            for p in &node.parents {
                nodes.push_front(self.nodes.get(p).ok_or(Error::PackageNotFound(p.into()))?);
            }
            res = merge(res, &node.value)?;
        }

        from_value::<T>(res).map_err(Error::SerializationError)
    }

    /// Uses `cfg_expr` to evaluate a conditional expression in a toml key.
    /// At the moment it only supports target expressions.
    ///
    /// ```toml
    /// [package.metadata.'cfg(target = "unix")']
    /// value = ...
    /// ```
    fn check_cfg(lit: &str) -> Option<bool> {
        let cfg = Expression::parse(lit).ok()?;
        let triple = std::env::var("TARGET").ok()?;
        let target = get_builtin_target_by_triple(&triple)?;
        cfg.eval(|pred| match pred {
            Predicate::Target(tp) => Some(tp.matches(target)),
            _ => None,
        })
    }

    #[cfg(test)]
    pub fn nodes(&self) -> &HashMap<String, MetadataNode> {
        &self.nodes
    }
}

pub fn merge_default(rhs: Value, lhs: &Value) -> Result<Value, Error> {
    if rhs == *lhs {
        return Ok(rhs);
    }
    if let Value::Null = lhs {
        return Ok(rhs);
    }
    if let Value::Null = rhs {
        return Ok(lhs.clone());
    }

    if std::mem::discriminant(&rhs) != std::mem::discriminant(lhs) {
        return Err(Error::IncompatibleMerge);
    }

    match rhs {
        Value::Array(mut r) => {
            for v in lhs.as_array().unwrap() {
                if !r.contains(v) {
                    r.push(v.clone());
                }
            }
            Ok(Value::Array(r))
        }
        Value::Object(mut r) => {
            for (k, v) in lhs.as_object().unwrap() {
                let merged = merge_default(r.get(k).cloned().unwrap_or(Value::Null), v)?;
                r.insert(k.into(), merged);
            }
            Ok(Value::Object(r))
        }
        _ => Ok(lhs.clone()),
    }
}
