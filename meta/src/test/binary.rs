use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use toml::{Table, Value};

use crate::{
    binary::BinaryMetadataListExt,
    error::Error,
    test::{Package, Test},
};

use super::assert_set;

trait BinaryTestExt {
    fn new_bin(name: &str, packages: Vec<Package>) -> Self;
    fn check_paths(&self, key: &str, expected: &[&str]) -> Result<(), Error>;
}

impl BinaryTestExt for Test {
    fn new_bin(name: &str, mut packages: Vec<Package>) -> Self {
        let name = format!("bin_{}", name);
        for p in packages.iter_mut() {
            replace_paths(&name, &mut p.config);
        }
        Self::new(name, packages)
    }

    fn check_paths(&self, key: &str, expected: &[&str]) -> Result<(), Error> {
        let resolved = self.metadata.paths(key)?;

        println!("\n# Final\n");
        println!("{:#?}", resolved);

        assert_set(
            resolved,
            expected
                .iter()
                .map(|p| PathBuf::from(env!("OUT_DIR")).join(p)),
        );
        Ok(())
    }
}

fn replace_paths(name: &str, table: &mut Table) {
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    for e in table.iter_mut() {
        match e {
            (k, Value::String(v)) if k == "url" => {
                let folder = COUNT.fetch_add(1, Ordering::Relaxed);
                let dir = PathBuf::from(format!("{}/paths/{}/{}", env!("OUT_DIR"), name, folder));
                *v = v.replace("$TEST", dir.to_str().unwrap());
                if let Some(path) = v.strip_prefix("file://") {
                    fs::create_dir_all(path).expect("Failed to create test paths");
                }
            }
            (_, Value::Table(v)) => replace_paths(name, v),
            _ => (),
        };
    }
}

// TODO: Unarchive test
// TODO: Download test
// TODO: Checksum test

#[test]
fn simple() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            url = "file://$TEST"
            paths = [ "lib/pkgconfig" ]
        ],
    }];

    let test = Test::new_bin("simple", pkgs);

    test.check_paths("dep", &["dep/lib/pkgconfig"]).unwrap();
}

#[test]
fn overrides() {
    let pkgs = vec![
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "file://$TEST"
                paths = [ "new" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "file://$TEST"
                paths = [ "old" ]
            ],
        },
    ];

    let test = Test::new_bin("overrides", pkgs);

    test.check_paths("dep", &["dep/new", "dep/old"]).unwrap();
}

#[test]
fn provides() {
    let pkgs = vec![
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.pkg]
                name = "pkg"
                url = "file://$TEST"
                paths = [ "lib/pkgconfig" ]
                provides = [ "dep" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "file://$TEST"
                name = "dep"
            ],
        },
    ];

    let test = Test::new_bin("provides", pkgs);

    test.check_paths("pkg", &["pkg/lib/pkgconfig"]).unwrap();
    test.check_paths("dep", &["pkg/lib/pkgconfig"]).unwrap();
}

#[test]
fn provides_override() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["pkg"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "file://$TEST"
                paths = [ "lib/pkgconfig" ]
            ],
        },
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.pkg]
                name = "pkg"
                url = "file://$TEST"
                paths = [ "lib/pkgconfig" ]
                provides = [ "dep" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                name = "dep"
            ],
        },
    ];

    let test = Test::new_bin("provides_override", pkgs);

    test.check_paths("pkg", &["pkg/lib/pkgconfig"]).unwrap();
    test.check_paths("dep", &["dep/lib/pkgconfig"]).unwrap();
}

#[test]
fn provides_conflict() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b"],
            config: Default::default(),
        },
        Package {
            name: "a",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.a]
                name = "a"
                url = "file://$TEST"
                paths = [ "lib/pkgconfig" ]
                provides = [ "dep" ]
            ],
        },
        Package {
            name: "b",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "file://$TEST"
                paths = [ "lib/pkgconfig" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                name = "dep"
            ],
        },
    ];

    let test = Test::new_bin("provides_conflict", pkgs);
    println!("{:#?}", test.metadata);

    let res = test.check_paths("dep", &[]);
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));
}
