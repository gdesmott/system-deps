use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    sync::OnceLock,
};

use cargo_metadata::{DependencyKind, Metadata, MetadataCommand};
use cfg_expr::{targets::get_builtin_target_by_triple, Expression, Predicate};
use serde::de::DeserializeOwned;
use serde_json::{from_value, Map, Value};

/// Stores a section of metadata found in one package.
/// `next` indexes the  to packages downstream from this.
#[derive(Debug, Default)]
pub struct MetadataNode {
    value: Option<Value>,
    next: HashSet<String>,
}

/// Graph like structure that stores the package nodes that have a metadata entry.
#[derive(Debug)]
pub struct MetadataList {
    nodes: HashMap<&'static str, MetadataNode>,
}

impl MetadataList {
    fn new() -> Self {
        Self {
            nodes: HashMap::from([("", MetadataNode::default())]),
        }
    }

    fn insert(&mut self, name: &'static str, parent: &str, values: Option<Value>) {
        match values {
            Some(v) => {
                // Create a new node if it doesn't exist
                self.nodes.entry(name).or_insert_with(|| MetadataNode {
                    value: Some(v),
                    ..Default::default()
                });
            }
            None => {
                if !self.nodes.contains_key(name) {
                    // If no value is provided only add the node to the parent if it already exists
                    return;
                }
            }
        };

        // Add child to the parent node
        let parent_node = self
            .nodes
            .get_mut(parent)
            .expect("Error creating metadata graph");
        parent_node.next.insert(name.into());
    }

    /// Applies the reducing rules to the tree and returns the final value transformed to the desired type
    pub fn get<T: DeserializeOwned>(&self, f: impl Fn() -> bool) -> Option<T> {
        let base = self.nodes.get("").unwrap();
        let mut stack = VecDeque::from([base]);

        let mut value = Value::Null;
        while let Some(node) = stack.pop_front() {
            stack.extend(node.next.iter().filter_map(|n| self.nodes.get(n.as_str())));
            if let Some(v) = &node.value {
                value = v.clone();
            };
        }

        from_value::<T>(value).ok()
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
    pub fn nodes(&self) -> &HashMap<&'static str, MetadataNode> {
        &self.nodes
    }
}

/// Recursively read dependency manifests to find metadata matching a key.
/// The matching metadata is aggregated in a list, with downstream crates having priority
/// for overwriting values. It will only read from the metadata sections matching the
/// provided section key.
///
/// ```toml
/// [package.metadata.section]
/// some_value = ...
/// other_value = ...
/// ```
pub fn read_metadata(manifest: &Path, section: &str) -> MetadataList {
    static CACHED: OnceLock<Metadata> = OnceLock::new();
    let metadata = CACHED.get_or_init(|| {
        MetadataCommand::new()
            .manifest_path(manifest)
            .exec()
            .unwrap()
    });

    let project_root = manifest.parent().unwrap();

    // Depending on if we are on a workspace or not, use the root package or all the
    // workspace packages as a starting point
    let mut packages = if let Some(root) = metadata.root_package() {
        VecDeque::from([(root, None)])
    } else {
        metadata
            .workspace_packages()
            .into_iter()
            .map(|p| (p, None))
            .collect()
    };

    // Add the workspace metadata (if it exists) first
    //let mut res = metadata
    //    .workspace_metadata
    //    .as_object()
    //    .and_then(|meta| meta.get(key))
    //    .cloned()
    //    .unwrap_or(Value::Object(Map::new()));

    let mut res = MetadataList::new();

    // Iterate through the dependency tree to visit all packages
    let mut visited: HashSet<&str> = packages.iter().map(|(p, _)| p.name.as_str()).collect();
    while let Some((pkg, parent)) = packages.pop_front() {
        // TODO: Optional packages

        // Keep track of the local manifests to see if they change
        if pkg.manifest_path.starts_with(project_root) {
            println!("cargo:rerun-if-changed={}", pkg.manifest_path);
        };

        // Get `package.metadata.section` and add it to the metadata graph
        let section = pkg.metadata.as_object().and_then(|meta| meta.get(section));
        res.insert(
            pkg.name.as_str(),
            parent.unwrap_or_default(),
            section.cloned(),
        );

        // TODO: If this is the last element, don't keep going

        // Add dependencies to the queue
        for dep in &pkg.dependencies {
            match dep.kind {
                DependencyKind::Normal | DependencyKind::Build => {}
                _ => continue,
            }

            // If visited, don't keep going, but add dependencies to graph
            if !visited.insert(&dep.name) {
                res.insert(pkg.name.as_str(), parent.unwrap_or_default(), None);
                continue;
            }

            if let Some(dep_pkg) = metadata.packages.iter().find(|p| p.name == dep.name) {
                packages.push_back((dep_pkg, Some(pkg.name.as_str())));
            };
        }
    }

    res
}
