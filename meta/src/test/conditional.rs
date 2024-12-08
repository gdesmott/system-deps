use crate::{
    error::Error,
    test::{Package, Test},
};

#[test]
fn conditional() -> Result<(), Error> {
    let manifest = r#"
            [package.metadata.system-deps.dep]
            value = "default"
            other = 32

            [package.metadata.system-deps.'cfg(all())'.dep]
            value = "final"
        "#;

    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(manifest)?,
    }];
    let test = Test::new("conditional_true", pkgs)?;
    assert_eq!(
        test.check("dep")?,
        &toml::toml![
            value = "final"
            other = 32
        ]
    );

    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(&manifest.replace("all", "any"))?,
    }];
    let test = Test::new("conditional_false", pkgs)?;
    assert_eq!(
        test.check("dep")?,
        &toml::toml![
            value = "default"
            other = 32
        ]
    );

    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn conditional_conflict() -> Result<(), Error> {
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
        )?,
    }];

    let test = Test::new("conditional_conflict", pkgs);
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::IncompatibleMerge)));

    Ok(())
}

#[test]
fn conditional_not_map() -> Result<(), Error> {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(
            r#"
            [package.metadata.system-deps.'cfg(all())']
            dep = 1234
        "#,
        )?,
    }];

    let test = Test::new("conditional_not_map", pkgs);
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::CfgNotObject(_))));

    Ok(())
}

#[test]
fn conditional_unsupported() -> Result<(), Error> {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::from_str(
            r#"
            [package.metadata.system-deps.'cfg(feature = "a")'.dep]
            value = "a"
        "#,
        )?,
    }];

    let test = Test::new("conditional_unsupported", pkgs);
    println!("left: {:?}", test);
    assert!(matches!(test, Err(Error::UnsupportedCfg(_))));

    Ok(())
}
