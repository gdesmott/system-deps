use crate::{
    error::{CfgError, Error},
    test::{Package, Test},
};

#[test]
fn conditional() {
    let manifest = r#"
            [package.metadata.system-deps.dep]
            value = "default"
            other = true

            [package.metadata.system-deps.'cfg(all())'.dep]
            value = "final"
        "#;

    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(manifest).unwrap(),
    }];
    let test = Test::new("conditional", pkgs);
    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml![
            value = "final"
            other = true
        ]
    );

    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(&manifest.replace("all", "any")).unwrap(),
    }];
    let test = Test::new("conditional_alt", pkgs);
    assert_eq!(
        test.check("dep").unwrap(),
        toml::toml![
            value = "default"
            other = true
        ]
    );
}

#[test]
#[cfg(target_os = "linux")]
fn conditional_conflict() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(
            r#"
            [package.metadata.system-deps.'cfg(target_os = "linux")'.dep]
            value = "linux"

            [package.metadata.system-deps.'cfg(unix)'.dep]
            value = "unix"
        "#,
        )
        .unwrap(),
    }];

    let test = Test::new("conditional_conflict", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));
}

#[test]
fn conditional_not_map() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(
            r#"
            [package.metadata.system-deps.'cfg(all())']
            dep = 1234
        "#,
        )
        .unwrap(),
    }];

    let test = Test::new("conditional_not_map", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(res, Err(Error::CfgError(CfgError::NotObject))));
}

#[test]
fn conditional_unsupported() {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(
            r#"
            [package.metadata.system-deps.'cfg(feature = "a")'.dep]
            value = "a"
        "#,
        )
        .unwrap(),
    }];

    let test = Test::new("conditional_unsupported", pkgs);

    let res = test.check("dep");
    println!("left: {:?}", res);
    assert!(matches!(
        res,
        Err(Error::CfgError(CfgError::Unsupported(_)))
    ));
}
