use std::{
    fs, io,
    path::{Path, PathBuf},
};

use toml::{Table, Value};

use crate::*;

macro_rules! entry {
    ($table:expr, $key:expr) => {
        $table
            .entry($key)
            .or_insert_with(|| Value::Table(Table::default()))
            .as_table_mut()
            .unwrap()
    };
}

#[derive(Debug)]
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

        if !self.deps.is_empty() {
            let dependencies = entry!(table, "dependencies");
            for name in self.deps {
                let dep = entry!(dependencies, name);
                dep.insert("path".into(), format!("../{}", name).into());
            }
        }

        let mut out = Path::new(env!("OUT_DIR")).join(format!("tests/{}/{}", test_name, self.name));
        let _ = fs::remove_dir_all(&out);

        out.push("src");
        fs::create_dir_all(&out)?;
        fs::write(out.join("lib.rs"), "")?;
        out.pop();

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
    fn new(name: &str, packages: Vec<Package>) -> Self {
        assert!(!packages.is_empty());

        println!("\n# Dependencies\n");
        let mut manifest = None;
        for pkg in packages {
            let out = pkg
                .write_toml(name)
                .expect("Error writing Cargo.toml for test package");
            println!("- {}", out.display());
            manifest.get_or_insert(out);
        }
        let manifest = manifest.expect("There is no main test case");

        let metadata = MetadataList::from(&manifest, "system-deps");
        //println!("\n# Metadata\n");
        //println!("{:#?}", metadata.nodes());

        Self { metadata, manifest }
    }

    fn check(&self, key: &str) -> Result<Table, Error> {
        let resolved = self.metadata.get::<Table>(key, merge_default)?;
        println!("\n# Final\n");
        println!("{}", toml::to_string_pretty(&resolved).unwrap());
        Ok(resolved)
    }
}

// Metadata tests

#[test]
fn simple() {
    let pkgs = vec![Package {
        name: "simple",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.simple]
            value = "simple"
        ],
    }];

    let test = Test::new("simple", pkgs);

    assert_eq!(
        test.check("simple").unwrap(),
        toml::toml! [
            [simple]
            value = "simple"
        ]
    );
}

#[test]
fn inherit() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: Table::default(),
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "original"
            ],
        },
    ];

    let test = Test::new("inherit", pkgs);

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            value = "original"
        ]
    );
}

#[test]
fn overwrite() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "final"
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "original"
            ],
        },
    ];

    let test = Test::new("overwrite", pkgs);

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            value = "final"
        ]
    );
}

#[test]
fn chain() {
    let pkgs = ["final", "a", "b", "c", "d", "e", "original", ""]
        .windows(2)
        .map(|p| {
            let manifest = format!(
                r#"
                [package.metadata.system-deps.original]
                value = "{}""#,
                p[0]
            );
            let mut deps = Vec::new();
            if !p[1].is_empty() {
                deps.push(p[1]);
            }
            Package {
                name: p[0],
                deps,
                config: toml::from_str(&manifest).unwrap(),
            }
        })
        .collect::<Vec<_>>();

    let test = Test::new("chain", pkgs);

    assert_eq!(
        test.check("original").unwrap(),
        toml::toml! [
            [original]
            value = "final"
        ]
    );
}

#[test]
fn only_some() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                text = "final"
                added = "top"
                value = false
                list = [ "c", "d" ]
                other = { different = 3, new = 4 }
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                text = "original"
                value = true
                number = 256
                list = [ "a", "b" ]
                other = { same = 1, different = 2 }
            ],
        },
    ];

    let test = Test::new("only_some", pkgs);

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            text = "final"
            number = 256
            value = false
            added = "top"
            list = [ "a", "b", "c", "d" ]
            other = { same = 1, different = 3, new = 4 }
        ]
    );
}

#[test]
fn root_workspace() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [workspace.metadata.system-deps.dep]
            value = "final"

            [package.metadata.system-deps.dep]
            value = "original"
        ],
    }];

    let test = Test::new("root_workspace", pkgs);

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            value = "final"
        ]
    );
}

#[test]
fn virtual_workspace() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            value = "original"
        ],
    }];

    let mut test = Test::new("virtual_workspace", pkgs);

    test.manifest.pop();
    test.manifest.pop();
    test.manifest.push("Cargo.toml");

    let manifest = toml::toml![
        [workspace]
        members = ["dep"]
        resolver = "2"

        [workspace.metadata.system-deps.dep]
        value = "final"
    ];
    fs::write(&test.manifest, manifest.to_string()).expect("Failed to write manifest");
    test.metadata = MetadataList::from(&test.manifest, "system-deps");

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            value = "final"
        ]
    );
}

#[test]
fn conflict() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b"],
            config: Table::default(),
        },
        Package {
            name: "a",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "a"
            ],
        },
        Package {
            name: "b",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "b"
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "original"
            ],
        },
    ];

    let test = Test::new("conflict", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));
}

#[test]
fn no_conflict() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b"],
            config: Table::default(),
        },
        Package {
            name: "a",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "final"
            ],
        },
        Package {
            name: "b",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "final"
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "original"
            ],
        },
    ];

    let test = Test::new("no_conflict", pkgs);

    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml! [
            [dep]
            value = "final"
        ]
    );
}

//let paths = BinaryPaths::from(binaries.drain());
//println!("paths: {:?}\n", paths);
