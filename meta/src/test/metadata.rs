use std::vec;

use crate::{
    error::Error,
    parse::MetadataList,
    test::{assert_set, Package, Test},
    utils::merge_default,
};

#[test]
fn simple() -> Result<(), Error> {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            value = "simple"
        ],
    }];

    let test = Test::new("simple", pkgs)?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "simple"]);

    Ok(())
}

#[test]
fn inherit() -> Result<(), Error> {
    let mut pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: Default::default(),
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

    let test = Test::new("inherit", pkgs.clone())?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "original"]);

    pkgs[0].config = toml::toml![
        [package.metadata.system-deps.dep]
        value = "final"
    ];

    let test = Test::new("overwrite", pkgs)?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "final"]);

    Ok(())
}

#[test]
fn chain() -> Result<(), Error> {
    let names = ["final", "a", "b", "c", "d", "e", "original", ""];
    let mut pkgs = names
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

    let test = Test::new("chain", pkgs.clone())?;
    assert_eq!(test.check("original")?, &toml::toml![value = "final"]);
    assert_set(test.metadata.nodes.keys().map(|k| k.as_str()), names);

    for p in pkgs.iter_mut() {
        if !["final", "original"].contains(&p.name) {
            p.config.retain(|_, _| false);
        }
    }

    let test = Test::new("gap", pkgs)?;
    assert_eq!(test.check("original")?, &toml::toml![value = "final"]);
    assert_set(
        test.metadata.nodes.keys().map(|k| k.as_str()),
        ["final", "original", ""],
    );

    Ok(())
}

#[test]
fn merge_some() -> Result<(), Error> {
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

    let test = Test::new("merge_some", pkgs)?;

    assert_eq!(
        test.check("dep")?,
        &toml::toml! [
            text = "final"
            number = 256
            value = false
            added = "top"
            list = [ "a", "b", "c", "d" ]
            other = { same = 1, different = 3, new = 4 }
        ]
    );

    Ok(())
}

#[test]
fn incompatible_type() -> Result<(), Error> {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = 256
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "simple"
            ],
        },
    ];

    let test = Test::new("incompatible", pkgs);
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::IncompatibleMerge)));

    Ok(())
}

#[test]
fn root_workspace() -> Result<(), Error> {
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

    let test = Test::new("root_workspace", pkgs)?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "final"]);

    Ok(())
}

#[test]
fn virtual_workspace() -> Result<(), Error> {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            value = "original"
        ],
    }];

    let mut path = Test::write_manifest("virtual_workspace", pkgs);

    path.pop();
    path.pop();
    path.push("Cargo.toml");

    let manifest = toml::toml![
        [workspace]
        members = ["dep"]
        resolver = "2"

        [workspace.metadata.system-deps.dep]
        value = "final"
    ];
    std::fs::write(&path, manifest.to_string()).expect("Failed to write manifest");

    let metadata = MetadataList::new(&path, "system-deps")?;
    let table = metadata.build(merge_default)?;
    let test = Test {
        metadata,
        table,
        manifest: path,
    };
    assert_eq!(test.check("dep")?, &toml::toml![value = "final"]);

    Ok(())
}

#[test]
fn branch() -> Result<(), Error> {
    let mut pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b"],
            config: Default::default(),
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

    let test = Test::new("branch", pkgs.clone())?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "final"]);

    pkgs[2].config = toml::toml![
        [package.metadata.system-deps.dep]
        value = "different"
    ];

    let test = Test::new("branch_conflict", pkgs);
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::IncompatibleMerge)));

    Ok(())
}

#[test]
fn two_dependencies() -> Result<(), Error> {
    let mut pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b"],
            config: Default::default(),
        },
        Package {
            name: "a",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.a]
                value = "a"
            ],
        },
        Package {
            name: "b",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.b]
                value = "b"
            ],
        },
    ];

    let test = Test::new("two_dependencies", pkgs.clone())?;
    assert_eq!(test.check("a")?, &toml::toml![value = "a"]);
    assert_eq!(test.check("b")?, &toml::toml![value = "b"]);

    pkgs[1].config = toml::toml![
        [package.metadata.system-deps.a]
        value = "a"
        [package.metadata.system-deps.b]
        value = "a"
    ];

    let test = Test::new("two_dependencies_incompatible", pkgs.clone());
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::IncompatibleMerge)));

    pkgs[0].deps.pop();
    pkgs[1].deps.push("b");

    let test = Test::new("two_dependencies_nested", pkgs)?;
    assert_eq!(test.check("a")?, &toml::toml![value = "a"]);
    assert_eq!(test.check("b")?, &toml::toml![value = "a"]);

    Ok(())
}

#[test]
fn dependency_types() -> Result<(), Error> {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec![],
            config: toml::toml![
                [dependencies]
                regular = { path = "../regular" }
                [dev-dependencies]
                dev = { path = "../dev" }
                [build-dependencies]
                build = { path = "../build" }
            ],
        },
        Package {
            name: "regular",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.regular]
                value = "regular"
            ],
        },
        Package {
            name: "dev",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dev]
                value = "dev"
            ],
        },
        Package {
            name: "build",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.build]
                value = "build"
            ],
        },
    ];

    let test = Test::new("dependency_types", pkgs)?;
    assert_eq!(test.check("regular")?, &toml::toml![value = "regular"]);

    let dev = test.check("dev");
    println!("left: {:?}", dev);
    assert!(matches!(dev, Err(Error::PackageNotFound(_))));

    let build = test.check("build");
    println!("left: {:?}", build);
    assert!(matches!(build, Err(Error::PackageNotFound(_))));

    let nodes = test.metadata.nodes.keys().map(|k| k.as_str());
    assert_set(nodes, ["", "regular"]);

    Ok(())
}

#[test]
fn optional_package() -> Result<(), Error> {
    let mut pkgs = vec![
        Package {
            name: "main",
            deps: vec!["dep"],
            config: toml::toml![
                [dependencies.dep]
                optional = true
                [features]
                default = [ "dep:dep" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "simple"
            ],
        },
    ];

    let test = Test::new("optional_package", pkgs.clone())?;
    assert_eq!(test.check("dep")?, &toml::toml![value = "simple"]);

    pkgs[0].config.remove("features");
    let test = Test::new("optional_package_disabled", pkgs)?;

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::PackageNotFound(_))));

    Ok(())
}
