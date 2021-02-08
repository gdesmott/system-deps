// Parse system-deps metadata from Cargo.toml

use std::{fs, io::Read, path::Path};

use anyhow::{anyhow, bail, Error};
use toml::{map::Map, Value};

#[derive(Debug, PartialEq)]
pub(crate) struct MetaData {
    pub(crate) deps: Vec<Dependency>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Dependency {
    pub(crate) key: String,
    pub(crate) version: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) feature: Option<String>,
    pub(crate) optional: bool,
    pub(crate) version_overrides: Vec<VersionOverride>,
}

impl Dependency {
    fn new(name: &str) -> Self {
        Self {
            key: name.to_string(),
            version: None,
            name: None,
            feature: None,
            optional: false,
            version_overrides: Vec::new(),
        }
    }

    pub(crate) fn lib_name(&self) -> String {
        self.name.as_ref().unwrap_or(&self.key).to_string()
    }
}

#[derive(Debug, PartialEq)]
pub(crate) struct VersionOverride {
    pub(crate) key: String,
    pub(crate) version: String,
    pub(crate) name: Option<String>,
    pub(crate) optional: Option<bool>,
}

struct VersionOverrideBuilder {
    version_id: String,
    version: Option<String>,
    full_name: Option<String>,
    optional: Option<bool>,
}

impl VersionOverrideBuilder {
    fn new(version_id: &str) -> Self {
        Self {
            version_id: version_id.to_string(),
            version: None,
            full_name: None,
            optional: None,
        }
    }

    fn build(self) -> Result<VersionOverride, Error> {
        let version = self
            .version
            .ok_or_else(|| anyhow!("missing version field"))?;

        Ok(VersionOverride {
            key: self.version_id,
            version,
            name: self.full_name,
            optional: self.optional,
        })
    }
}

impl MetaData {
    pub(crate) fn from_file(path: &Path) -> Result<Self, crate::Error> {
        let mut manifest = fs::File::open(&path).map_err(|e| {
            crate::Error::FailToRead(format!("error opening {}", path.display()), e)
        })?;

        let mut manifest_str = String::new();
        manifest.read_to_string(&mut manifest_str).map_err(|e| {
            crate::Error::FailToRead(format!("error reading {}", path.display()), e)
        })?;

        Self::from_str(manifest_str)
            .map_err(|e| crate::Error::InvalidMetadata(format!("{}: {}", path.display(), e)))
    }

    fn from_str(manifest_str: String) -> Result<Self, Error> {
        let toml = manifest_str
            .parse::<toml::Value>()
            .map_err(|e| anyhow!("error parsing TOML: {:?}", e))?;

        let key = "package.metadata.system-deps";
        let meta = toml
            .get("package")
            .and_then(|v| v.get("metadata"))
            .and_then(|v| v.get("system-deps"))
            .ok_or_else(|| anyhow!("no {}", key))?;

        let table = meta
            .as_table()
            .ok_or_else(|| anyhow!("{} not a table", key))?;

        let mut deps = Vec::new();

        for (name, value) in table {
            let dep = Self::parse_dep(name, value)
                .map_err(|e| anyhow!("metadata.system-deps.{}: {}", name, e))?;
            deps.push(dep);
        }

        Ok(MetaData { deps })
    }

    fn parse_dep(name: &str, value: &Value) -> Result<Dependency, Error> {
        let mut dep = Dependency::new(name);

        match value {
            // somelib = "1.0"
            toml::Value::String(ref s) => {
                dep.version = Some(s.clone());
            }
            toml::Value::Table(ref t) => {
                Self::parse_dep_table(&mut dep, t)?;
            }
            _ => {
                bail!("not a string or table");
            }
        }

        Ok(dep)
    }

    fn parse_dep_table(dep: &mut Dependency, t: &Map<String, Value>) -> Result<(), Error> {
        for (key, value) in t {
            match (key.as_str(), value) {
                ("feature", &toml::Value::String(ref s)) => {
                    dep.feature = Some(s.clone());
                }
                ("version", &toml::Value::String(ref s)) => {
                    dep.version = Some(s.clone());
                }
                ("name", &toml::Value::String(ref s)) => {
                    dep.name = Some(s.clone());
                }
                ("optional", &toml::Value::Boolean(optional)) => {
                    dep.optional = optional;
                }
                (version_feature, &toml::Value::Table(ref version_settings))
                    if version_feature.starts_with('v') =>
                {
                    let mut builder = VersionOverrideBuilder::new(version_feature);

                    for (k, v) in version_settings {
                        match (k.as_str(), v) {
                            ("version", &toml::Value::String(ref feat_vers)) => {
                                builder.version = Some(feat_vers.into());
                            }
                            ("name", &toml::Value::String(ref feat_name)) => {
                                builder.full_name = Some(feat_name.into());
                            }
                            ("optional", &toml::Value::Boolean(optional)) => {
                                builder.optional = Some(optional);
                            }
                            _ => {
                                bail!(
                                    "unexpected version settings key: {} type: {}",
                                    k,
                                    v.type_str()
                                )
                            }
                        }
                    }

                    dep.version_overrides.push(builder.build()?);
                }
                _ => {
                    bail!("unexpected key {} type {}", key, value.type_str());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use std::path::PathBuf;

    fn parse_file(dir: &str) -> Result<MetaData, crate::Error> {
        let mut p = PathBuf::new();
        p.push("src");
        p.push("tests");
        p.push(dir);
        p.push("Cargo.toml");
        assert!(p.exists());

        MetaData::from_file(&p)
    }

    #[test]
    fn parse_good() {
        let m = parse_file("toml-good").unwrap();

        assert_eq!(
            m,
            MetaData {
                deps: vec![
                    Dependency {
                        key: "testdata".into(),
                        version: Some("4".into()),
                        name: None,
                        feature: None,
                        optional: false,
                        version_overrides: vec![],
                    },
                    Dependency {
                        key: "testlib".into(),
                        version: Some("1".into()),
                        name: None,
                        feature: Some("test-feature".into()),
                        optional: false,
                        version_overrides: vec![],
                    },
                    Dependency {
                        key: "testmore".into(),
                        version: Some("2".into()),
                        name: None,
                        feature: Some("another-test-feature".into()),
                        optional: false,
                        version_overrides: vec![],
                    }
                ]
            }
        )
    }

    #[test]
    fn parse_feature_not_string() {
        assert_matches!(
            parse_file("toml-feature-not-string"),
            Err(crate::Error::InvalidMetadata(_))
        );
    }

    #[test]
    fn parse_override_name() {
        let m = parse_file("toml-override-name").unwrap();

        assert_eq!(
            m,
            MetaData {
                deps: vec![Dependency {
                    key: "test_lib".into(),
                    version: Some("1.0".into()),
                    name: Some("testlib".into()),
                    feature: None,
                    optional: false,
                    version_overrides: vec![VersionOverride {
                        key: "v1_2".into(),
                        version: "1.2".into(),
                        name: None,
                        optional: None,
                    }],
                },]
            }
        )
    }

    #[test]
    fn parse_feature_versions() {
        let m = parse_file("toml-feature-versions").unwrap();

        assert_eq!(
            m,
            MetaData {
                deps: vec![Dependency {
                    key: "testdata".into(),
                    version: Some("4".into()),
                    name: None,
                    feature: None,
                    optional: false,
                    version_overrides: vec![
                        VersionOverride {
                            key: "v5".into(),
                            version: "5".into(),
                            name: None,
                            optional: None,
                        },
                        VersionOverride {
                            key: "v6".into(),
                            version: "6".into(),
                            name: None,
                            optional: None,
                        },
                    ],
                },]
            }
        )
    }

    #[test]
    fn parse_optional() {
        let m = parse_file("toml-optional").unwrap();

        assert_eq!(
            m,
            MetaData {
                deps: vec![
                    Dependency {
                        key: "testbadger".into(),
                        version: Some("1".into()),
                        name: None,
                        feature: None,
                        optional: true,
                        version_overrides: vec![],
                    },
                    Dependency {
                        key: "testlib".into(),
                        version: Some("1.0".into()),
                        name: None,
                        feature: None,
                        optional: true,
                        version_overrides: vec![VersionOverride {
                            key: "v5".into(),
                            version: "5.0".into(),
                            name: Some("testlib-5.0".into()),
                            optional: Some(false),
                        },],
                    },
                    Dependency {
                        key: "testmore".into(),
                        version: Some("2".into()),
                        name: None,
                        feature: None,
                        optional: false,
                        version_overrides: vec![VersionOverride {
                            key: "v3".into(),
                            version: "3.0".into(),
                            name: None,
                            optional: Some(true),
                        },],
                    },
                ]
            }
        )
    }
}
