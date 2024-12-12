use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

use toml::{Table, Value};

use crate::{error::Error, parse::MetadataList, utils::merge_default};

#[cfg(feature = "binary")]
mod binary;
mod conditional;
mod metadata;

macro_rules! entry {
    ($table:expr, $key:expr) => {
        $table
            .entry($key)
            .or_insert_with(|| Value::Table(Table::default()))
            .as_table_mut()
            .unwrap()
    };
}

#[derive(Clone, Debug, Default)]
struct Package {
    name: &'static str,
    deps: Vec<&'static str>,
    config: Table,
}

impl Package {
    fn write_toml(self, test_name: &str) -> io::Result<PathBuf> {
        let mut table = self.config;

        let package = entry!(table, "package");
        package.insert("name".into(), self.name.into());

        let lib = entry!(table, "lib");
        lib.insert("path".into(), "".into());

        if !self.deps.is_empty() {
            let dependencies = entry!(table, "dependencies");
            for name in self.deps {
                let dep = entry!(dependencies, name);
                dep.insert("path".into(), format!("../{}", name).into());
            }
        }

        let mut out = Path::new(env!("OUT_DIR")).join(format!("tests/{}/{}", test_name, self.name));
        let _ = fs::remove_dir_all(&out);

        fs::create_dir_all(&out)?;
        out.push("Cargo.toml");
        fs::write(&out, table.to_string())?;

        Ok(out)
    }
}

#[derive(Debug)]
struct Test {
    metadata: MetadataList,
    table: Table,
    #[allow(dead_code)]
    manifest: PathBuf,
}

impl Test {
    fn write_manifest(name: impl AsRef<str>, packages: Vec<Package>) -> PathBuf {
        assert!(!packages.is_empty());

        println!("\n# Dependencies\n");
        let mut manifest = None;
        for pkg in packages {
            let out = pkg
                .write_toml(name.as_ref())
                .expect("Error writing Cargo.toml for test package");
            println!("- {}", out.display());
            manifest.get_or_insert(out);
        }

        manifest.expect("There is no main test case")
    }

    fn new(name: impl AsRef<str>, packages: Vec<Package>) -> Result<Self, Error> {
        let manifest = Self::write_manifest(name, packages);
        let metadata = MetadataList::new(&manifest, "system-deps")?;
        let table = metadata.build(merge_default)?;

        //println!("\n# Metadata\n");
        //println!("{:#?}", metadata);

        Ok(Self {
            metadata,
            table,
            manifest,
        })
    }

    fn check(&self, key: &str) -> Result<&Table, Error> {
        self.table
            .get(key)
            .and_then(|v| v.as_table())
            .ok_or(Error::PackageNotFound(key.into()))
    }
}

fn assert_set<T: std::fmt::Debug + Eq + std::hash::Hash>(
    rhs: impl IntoIterator<Item = T>,
    lhs: impl IntoIterator<Item = T>,
) {
    let r = rhs.into_iter().collect::<HashSet<_>>();
    let l = lhs.into_iter().collect::<HashSet<_>>();
    assert_eq!(r, l);
}
