use crate::{
    error::Error,
    parse::MetadataList,
    test::{assert_set, Package, Test},
};

#[test]
fn simple() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            value = "simple"
        ],
    }];

    let test = Test::new("simple", pkgs);

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "simple"]);
}

#[test]
fn inherit() {
    let pkgs = vec![
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

    let test = Test::new("inherit", pkgs);

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "original"]);
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

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "final"]);
}

#[test]
fn chain() {
    let names = ["final", "a", "b", "c", "d", "e", "original", ""];
    let pkgs = names
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
        toml::toml![value = "final"]
    );

    let nodes = test.metadata.nodes.keys().map(|k| k.as_str());
    assert_set(nodes, names);
}

#[test]
fn gap() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "final"
            ],
        },
        Package {
            name: "a",
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

    let test = Test::new("gap", pkgs);

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "final"]);

    let nodes = test.metadata.nodes.keys().map(|k| k.as_str());
    assert_set(nodes, ["", "main", "dep"]);
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
fn incompatible() {
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

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));
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

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "final"]);
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
    std::fs::write(&test.manifest, manifest.to_string()).expect("Failed to write manifest");
    test.metadata = MetadataList::from(&test.manifest, "system-deps");

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "final"]);
}

#[test]
fn branch_no_conflict() {
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

    assert_eq!(test.check("dep").unwrap(), toml::toml![value = "final"]);
}

#[test]
fn branch_conflict() {
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
fn branch_mixed() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a", "b", "c"],
            config: Default::default(),
        },
        Package {
            name: "a",
            deps: vec!["b", "c", "dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "a"
            ],
        },
        Package {
            name: "b",
            deps: vec!["c", "dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "b"
            ],
        },
        Package {
            name: "c",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                value = "c"
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

    let test = Test::new("branch_mixed", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));
}

#[test]
fn two_dependencies() {
    let pkgs = vec![
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

    let test = Test::new("two_dependencies", pkgs);

    assert_eq!(test.check("a").unwrap(), toml::toml![value = "a"]);
    assert_eq!(test.check("b").unwrap(), toml::toml![value = "b"]);
}

#[test]
fn two_dependencies_nested() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["a"],
            config: Default::default(),
        },
        Package {
            name: "a",
            deps: vec!["b"],
            config: toml::toml![
                [package.metadata.system-deps.a]
                value = "a"

                [package.metadata.system-deps.b]
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

    let test = Test::new("two_dependencies_nested", pkgs);

    assert_eq!(test.check("a").unwrap(), toml::toml![value = "a"]);
    assert_eq!(test.check("b").unwrap(), toml::toml![value = "a"]);
}

#[test]
fn dev_dependencies() {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec![],
            config: toml::toml![
                [dev-dependencies]
                dep = { path = "../dep" }

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

    let test = Test::new("dev_dependencies", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::PackageNotFound(_))));

    let nodes = test.metadata.nodes.keys().map(|k| k.as_str());
    assert_set(nodes, ["", "main"]);
}
