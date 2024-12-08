use std::{
    convert::TryInto,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use tiny_http::{Header, Response, Server, StatusCode};
use toml::{Table, Value};

use crate::{
    binary::{merge_binary, Extension, Paths},
    error::Error,
    test::{Package, Test},
    BUILD_MANIFEST,
};

use super::assert_set;

trait BinaryTestExt {
    fn new_bin(name: &str, packages: Vec<Package>) -> Result<Test, Error>;
    fn get_paths(&self, key: &str) -> Result<Vec<PathBuf>, Error>;
}

impl BinaryTestExt for Test {
    fn new_bin(name: &str, mut packages: Vec<Package>) -> Result<Test, Error> {
        let name = format!("bin_{}", name);
        for p in packages.iter_mut() {
            replace_paths(&name, &mut p.config);
        }
        Self::new(name, packages)
    }

    fn get_paths(&self, key: &str) -> Result<Vec<PathBuf>, Error> {
        let paths: Paths = self.metadata.build(merge_binary)?.into_iter().collect();
        paths.get(key).cloned()
    }
}

fn replace_paths(name: &str, table: &mut Table) {
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    for (k, v) in table.iter_mut() {
        match v {
            Value::String(v) if k == "url" && v == "$TEST" => {
                let folder = COUNT.fetch_add(1, Ordering::Relaxed);
                let dir = format!("{}/paths/{}/{}", env!("OUT_DIR"), name, folder);
                fs::create_dir_all(&dir).expect("Failed to create test paths");
                *v = format!("file://{}", dir);
            }
            Value::Table(v) => replace_paths(name, v),
            _ => (),
        }
    }
}

fn assert_paths(paths: &[PathBuf], expected: &[&str]) {
    assert_set(
        paths,
        &expected
            .iter()
            .map(|p| Path::new(env!("OUT_DIR")).join(p))
            .collect::<Vec<_>>(),
    );
}

fn get_archives(web: Option<&str>) -> (PathBuf, Vec<(Table, &str, String, &str)>) {
    let base_path = Path::new(BUILD_MANIFEST)
        .parent()
        .unwrap()
        .join("meta/src/test/files");

    let mut archives = vec![
        #[cfg(feature = "gz")]
        (
            if web.is_some() { "web_gz" } else { "gz" },
            "test.tar.gz",
            "5135f7d6b869ae8802228aed4328f2aecf8f38ba597da89ced03b30d6afc3a35",
        ),
        #[cfg(feature = "xz")]
        (
            if web.is_some() { "web_xz" } else { "xz" },
            "test.tar.xz",
            "9b15167891c06d78995781683e9f5db58091a1678b9cedc9f2e04a53b549166e",
        ),
        #[cfg(feature = "zip")]
        (
            if web.is_some() { "web_zip" } else { "zip" },
            "test.zip",
            "cc4f4303d8673b3265ed92c7fbdbbe840b6f96f1e24d6bb92b3990f0c2238b9d",
        ),
    ];

    if web.is_none() {
        archives.push(("folder", "", ""));
    }

    let archives = archives
        .into_iter()
        .map(|(name, url, checksum)| {
            let url = if let Some(ref server) = web {
                format!("http://{}/{}", server, url)
            } else {
                format!("file://{}", base_path.join(url).display())
            };

            let manifest = format!(
                r#"
                    [package.metadata.system-deps.{}]
                    name = "{}"
                    url = "{}"
                    checksum = "{}"
                    paths = [ "lib/pkgconfig" ]"#,
                name, name, url, checksum,
            );

            (
                toml::from_str(&manifest).unwrap(),
                name,
                url.strip_prefix("file://").unwrap_or_default().to_string(),
                checksum,
            )
        })
        .collect();

    (base_path, archives)
}

// TODO: Version test
// TODO: These tests should be moved to system_deps base, check pkgconfig
// TODO: Change unwraps for specific errors
// TODO: Find a way of printing progress

#[test]
fn simple() -> Result<(), Error> {
    let pkgs = vec![Package {
        name: "dep",
        deps: vec![],
        config: toml::toml![
            [package.metadata.system-deps.dep]
            url = "$TEST"
            paths = [ "lib/pkgconfig" ]
        ],
    }];

    let test = Test::new_bin("simple", pkgs)?;
    let paths = test.get_paths("dep")?;
    assert_paths(&paths, &["dep/lib/pkgconfig"]);

    Ok(())
}

#[test]
fn overrides() -> Result<(), Error> {
    let pkgs = vec![
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "$TEST"
                paths = [ "new" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "$TEST"
                paths = [ "old" ]
            ],
        },
    ];

    let test = Test::new_bin("overrides", pkgs)?;
    let paths = test.get_paths("dep")?;
    assert_paths(&paths, &["dep/new", "dep/old"]);

    Ok(())
}

#[test]
fn provides() -> Result<(), Error> {
    let pkgs = vec![
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.pkg]
                name = "pkg"
                url = "$TEST"
                paths = [ "lib/pkgconfig" ]
                provides = [ "dep" ]
            ],
        },
        Package {
            name: "dep",
            deps: vec![],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "$TEST"
                name = "dep"
            ],
        },
    ];

    let test = Test::new_bin("provides", pkgs)?;
    let pkg = test.get_paths("pkg")?;
    assert_paths(&pkg, &["pkg/lib/pkgconfig"]);
    let dep = test.get_paths("dep")?;
    assert_paths(&dep, &["pkg/lib/pkgconfig"]);

    Ok(())
}

#[test]
fn provides_override() -> Result<(), Error> {
    let pkgs = vec![
        Package {
            name: "main",
            deps: vec!["pkg"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "$TEST"
                paths = [ "lib/pkgconfig" ]
            ],
        },
        Package {
            name: "pkg",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.pkg]
                name = "pkg"
                url = "$TEST"
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

    let test = Test::new_bin("provides_override", pkgs)?;
    let pkg = test.get_paths("pkg")?;
    assert_paths(&pkg, &["pkg/lib/pkgconfig"]);
    let dep = test.get_paths("dep")?;
    assert_paths(&dep, &["dep/lib/pkgconfig"]);

    Ok(())
}

#[test]
fn provides_conflict() -> Result<(), Error> {
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
                url = "$TEST"
                paths = [ "lib/pkgconfig" ]
                provides = [ "dep" ]
            ],
        },
        Package {
            name: "b",
            deps: vec!["dep"],
            config: toml::toml![
                [package.metadata.system-deps.dep]
                url = "$TEST"
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

    let test = Test::new_bin("provides_conflict", pkgs)?;
    let res = test.get_paths("pkg");
    println!("left: {:#?}", res);
    assert!(matches!(res, Err(Error::IncompatibleMerge)));

    Ok(())
}

#[test]
fn file_types() -> Result<(), Error> {
    let (_, archives) = get_archives(None);

    for (config, name, url, checksum) in archives {
        let pkgs = vec![Package {
            name,
            deps: vec![],
            config,
        }];

        let test = Test::new_bin(name, pkgs)?;
        let mut paths = test.get_paths(name)?;
        let mut p = paths.pop().unwrap();
        assert!(paths.is_empty());

        println!("{:?}", p);
        assert!(p.join("test.pc").is_file());
        p.pop();
        assert!(p.join("libtest.a").is_file());
        p.pop();

        // TODO: Folder
        if name == "folder" {
            println!("URL {:?}", url);
            assert_eq!(p.read_link().unwrap(), Path::new(&url));
        } else {
            p.push("checksum");
            assert!(p.is_file());
            assert_eq!(checksum.to_string(), fs::read_to_string(p).unwrap());
        }
    }

    Ok(())
}

#[test]
//#[cfg(feature = "gz")]
fn download() -> Result<(), Error> {
    let server_url = "127.0.0.1:8000";
    let (base_path, archives) = get_archives(Some(server_url));

    let mut path_list = thread::scope(|s| -> Result<Vec<(PathBuf, String)>, Error> {
        let handle = Arc::new(Server::http(server_url).unwrap());

        let server = handle.clone();

        s.spawn(move || loop {
            let Ok(Some(req)) = server.recv_timeout(Duration::new(3, 0)) else {
                break;
            };

            let url = base_path.join(&req.url()[1..]);

            let content_type = match url.as_path().try_into().unwrap() {
                #[cfg(feature = "gz")]
                Extension::TarGz => "application/gzip",
                #[cfg(feature = "xz")]
                Extension::TarXz => "application/zlib",
                #[cfg(feature = "zip")]
                Extension::Zip => "application/zip",
                Extension::Folder => "text/plain",
            };
            let header = Header {
                field: "Content-Type".parse().unwrap(),
                value: FromStr::from_str(content_type).unwrap(),
            };

            match fs::File::open(url) {
                Ok(file) => {
                    let res = Response::from_file(file).with_header(header);
                    let _ = req.respond(res);
                }
                Err(_) => {
                    let res = Response::new_empty(StatusCode(404));
                    let _ = req.respond(res);
                }
            };
        });

        let mut path_list = Vec::new();
        for (config, name, _, checksum) in archives {
            let pkgs = vec![Package {
                name,
                deps: vec![],
                config,
            }];

            let test = Test::new_bin(name, pkgs)?;
            let mut paths = test.get_paths(name)?;
            assert_eq!(paths.len(), 1);

            path_list.push((paths.pop().unwrap(), checksum.to_string()));
        }

        handle.unblock();
        Ok(path_list)
    })?;

    for (p, ch) in path_list.iter_mut() {
        assert!(p.join("test.pc").is_file());
        p.pop();
        assert!(p.join("libtest.a").is_file());
        p.pop();

        p.push("checksum");
        assert!(p.is_file());
        assert_eq!(*ch, fs::read_to_string(p).unwrap());
    }
    Ok(())
}
