use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::*;

#[derive(Debug, Default)]
struct TestDep<'a> {
    name: &'a str,
    dependencies: Vec<&'a str>,
    content: &'a str,
}

impl TestDep<'_> {
    fn write_toml(&self) -> io::Result<PathBuf> {
        let mut out = Path::new(env!("OUT_DIR")).join(format!("tests/{}", self.name));
        let _ = fs::remove_dir_all(&out);

        out.push("src");
        fs::create_dir_all(&out)?;
        fs::write(out.join("lib.rs"), "")?;
        out.pop();

        out.push("Cargo.toml");
        let f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&out)?;

        writeln!(
            &f,
            r#"
[package]
name = "{}""#,
            self.name,
        )
        .unwrap();

        if !self.dependencies.is_empty() {
            writeln!(&f, "[dependencies]")?;
        }
        for name in &self.dependencies {
            writeln!(&f, r#"{} = {{ path = "{}" }}"#, name, name)?;
        }

        write!(&f, "{}", self.content)?;

        Ok(out)
    }
}

fn setup_test(deps: Vec<TestDep>) -> MetadataList {
    assert!(!deps.is_empty());

    println!("\n[Dependencies]\n");
    let mut manifest = None;
    for dep in deps {
        let out = dep.write_toml().expect("Error writing test Cargo.toml");
        println!("- {}", out.display());
        manifest.get_or_insert(out);
    }
    let manifest = manifest.expect("There is no main test case");

    read_metadata(&manifest, "system-deps")
}

fn assert_nodes<'a>(
    rhs: impl IntoIterator<Item = &'a str>,
    lhs: impl IntoIterator<Item = &'a str>,
) {
    let rhs = HashSet::<&'a str>::from_iter(rhs);
    let lhs = HashSet::from_iter(lhs);
    assert_eq!(rhs, lhs)
}

// Metadata tests

#[test]
fn simple() {
    let deps = vec![TestDep {
        name: "simple",
        content: r#"
[package.metadata.system-deps.simple]
name = "simple"
version = "1.0"
        "#,
        ..Default::default()
    }];

    let metadata = setup_test(deps);
    println!("\n[Results]\n");
    println!("Values: {:#?}\n", metadata);

    assert_nodes(["", "simple"], metadata.nodes().keys().cloned());

    let resolved = metadata.get::<Value>(|| true).unwrap_or_default();
    println!("Final: {:#?}\n", resolved);

    let expected = r#"
[simple]
name = "simple"
version = "1.0"
"#;
    assert_eq!(resolved, toml::from_str::<Value>(expected).unwrap());
}

//let paths = BinaryPaths::from(binaries.drain());
//println!("paths: {:?}\n", paths);
