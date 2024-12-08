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

#[derive(Debug, Default)]
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
    manifest: PathBuf,
}

impl Test {
    fn new(name: impl AsRef<str>, packages: Vec<Package>) -> Self {
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

        let manifest = manifest.expect("There is no main test case");
        let metadata = MetadataList::from(&manifest, "system-deps");

        Self { metadata, manifest }
    }

    fn check(&self, key: &str) -> Result<Table, Error> {
        let resolved = self
            .metadata
            .get::<Table>(key, merge_default)?
            .remove(key)
            .ok_or(Error::PackageNotFound(key.into()))?;

        println!("\n# Final\n");
        println!("{:#?}", resolved);

        match resolved {
            Value::Table(v) => Ok(v),
            _ => unreachable!(),
        }
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
