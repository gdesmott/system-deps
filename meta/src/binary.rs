use std::{
    collections::HashMap,
    fs,
    iter::FromIterator,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    error::{BinaryError, Error},
    parse::MetadataList,
    utils::{merge_base, merge_default},
};

/// The extension of the binary archive.
/// Support for different extensions is enabled using features.
#[derive(Debug)]
enum Extension {
    /// A `.tar.gz` archive.
    #[cfg(feature = "gz")]
    TarGz,
    /// A `.tar.xz` archive.
    #[cfg(feature = "xz")]
    TarXz,
    /// A `.zip` archive.
    #[cfg(feature = "zip")]
    Zip,
    // #[cfg(feature = "pkg")]
    // Pkg,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Binary {
    Follow(FollowBinary),
    Url(UrlBinary),
}

/// Represents one location from where to download library binaries.
#[derive(Debug, Deserialize)]
pub struct UrlBinary {
    /// The url from which to download the archived binaries. It suppports:
    ///
    /// - Web urls, in the form `http[s]://website/archive.ext`.
    ///   This must directly download an archive with a known `Extension`.
    /// - Local files, in the form `file:///path/to/archive.ext`.
    ///   Note that this is made of the url descriptor `file://`, and then an absolute path, that
    ///   starts with `/`, so three total slashes are needed.
    ///   The path can point at an archive with a known `Extension`, or to a folder containing the
    ///   uncompressed binaries.
    url: String,
    /// Optionally, a checksum of the downloaded archive. When set, it is used to correctly cache
    /// the result. If this is not specified, it will still be cached by cargo, but redownloads
    /// might happen more often. It has no effect if `url` is a local folder.
    checksum: Option<String>,
    /// A list of relative paths inside the binary archive that point to a folder containing
    /// package config files. These directories will be prepended to the `PKG_CONFIG_PATH` when
    /// compiling the affected libraries.
    paths: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct FollowBinary {
    follows: String,
}

pub trait BinaryMetadataListExt {
    fn paths(&self, package: &str) -> Result<Vec<PathBuf>, Error>;
}

impl BinaryMetadataListExt for MetadataList {
    fn paths(&self, package: &str) -> Result<Vec<PathBuf>, Error> {
        // The binaries are stored in the target dir set by `system_deps_meta`.

        // If they are specific to a dependency, they live in a subfolder.
        let binary_list: HashMap<String, Binary> = self.get(package, merge_binary)?;
        let binary = binary_list
            .get(package)
            .ok_or(Error::PackageNotFound(package.into()))?;

        // Point binaries that follow an url to the original paths
        let (binary, name) = match binary {
            Binary::Url(ref bin) => (bin, package),
            Binary::Follow(bin) => {
                let Some(Binary::Url(follows)) = binary_list.get(&bin.follows) else {
                    return Err(
                        BinaryError::InvalidFollows(package.into(), bin.follows.clone()).into(),
                    );
                };
                (follows, bin.follows.as_str())
            }
        };

        // Only download the binaries if there isn't already a valid copy
        let dst = Path::new(&crate::BUILD_TARGET_DIR).join(name);
        if !check_valid_dir(&dst, binary.checksum.as_deref())? {
            download(&binary.url, &dst)?;
        }

        Ok(binary.paths.iter().flatten().map(|p| dst.join(p)).collect())
    }
}

// Global overrides from environment
// TODO: Change this so the env set global url always is first in the list of paths
//if let Some(url) = env("SYSTEM_DEPS_BINARY_URL") {
//    let checksum = env("SYSTEM_DEPS_BINARY_CHECKSUM");
//    let pkg_paths = env("SYSTEM_DEPS_BINARY_PKG_PATHS");
//    binaries.insert(
//        ".from_env".into(),
//        Binary::Url(UrlBinary {
//            url,
//            checksum,
//            pkg_paths: pkg_paths
//                .map(|p| {
//                    std::env::split_paths(&p)
//                        .map(|v| {
//                            v.to_str()
//                                .expect("Error with global binary package path")
//                                .to_string()
//                        })
//                        .collect()
//                })
//                .unwrap_or_default(),
//            global: Some(true),
//        }),
//    );
//}

/// Checks if the target directory is valid and if binaries need to be redownloaded.
/// On an `Ok` result, if the value is true it means that the directory is correct.
fn check_valid_dir(dst: &Path, checksum: Option<&str>) -> Result<bool, BinaryError> {
    let e = BinaryError::InvalidDirectory;

    // If it doesn't exist yet the download will need to happen
    if !dst.try_exists().map_err(e)? {
        return Ok(false);
    }

    // Raise an error if it is a file
    if dst.is_file() {
        return Err(BinaryError::DirectoryIsFile(dst.display().to_string()));
    }

    // If a checksum is not specified, assume the directory is invalid
    let Some(checksum) = checksum else {
        return Ok(false);
    };

    // Check if the checksum is valid
    for f in dst.read_dir().map_err(e)? {
        let f = f.map_err(e)?;
        if f.file_name() != "checksum" {
            continue;
        }
        return Ok(checksum == fs::read_to_string(f.path()).map_err(e)?);
    }
    Ok(false)
}

/// Retrieve a binary archive from the specified `url` and decompress it in the target directory.
/// "Download" is used as an umbrella term, since this can also be a local file.
fn download(url: &str, dst: &Path) -> Result<(), BinaryError> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    let ext = match url {
        #[cfg(feature = "gz")]
        u if u.ends_with(".tar.gz") => Ok(Extension::TarGz),
        #[cfg(feature = "xz")]
        u if u.ends_with(".tar.xz") => Ok(Extension::TarXz),
        #[cfg(feature = "zip")]
        u if u.ends_with(".zip") => Ok(Extension::Zip),
        // #[cfg(feature = "pkg")]
        // u if u.ends_with(".pkg") => Ok(Extension::Pkg),
        u => Err(BinaryError::UnsupportedExtension(u.into())),
    };

    // Local file
    if let Some(file_path) = url.strip_prefix("file://") {
        let path = Path::new(file_path);
        match ext {
            Ok(ext) => {
                let file = fs::read(path).map_err(BinaryError::LocalFileError)?;
                decompress(&file, dst, ext)?;
            }
            Err(e) => {
                // If it is a folder it can be symlinked
                if !path.is_dir() {
                    return Err(e);
                }
                let _l = LOCK.get_or_init(|| Mutex::new(())).lock();
                if !dst.read_link().is_ok_and(|l| l == path) {
                    if dst.is_symlink() {
                        std::fs::remove_file(dst).map_err(BinaryError::SymlinkError)?;
                    }
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(path, dst).map_err(BinaryError::SymlinkError)?;
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(path, dst)
                        .map_err(BinaryError::SymlinkError)?;
                }
            }
        };
    }
    // Download from the web
    else {
        let ext = ext?;
        let file = reqwest::blocking::get(url)
            .and_then(|req| req.bytes())
            .map_err(BinaryError::DownloadError)?;
        decompress(&file, dst, ext)?;
    }

    Ok(())
}

/// Extract a binary archive to the target directory. The methods for unpacking are
/// different depending on the extension. Each file type is gated behind a feature to
/// avoid having too many dependencies.
#[allow(unused)]
fn decompress(file: &[u8], dst: &Path, ext: Extension) -> Result<(), BinaryError> {
    #[cfg(any(feature = "gz", feature = "xz", feature = "zip", feature = "pkg"))]
    {
        match ext {
            #[cfg(feature = "gz")]
            Extension::TarGz => {
                let reader = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(reader);
                archive.unpack(dst).map_err(BinaryError::DecompressError)?;
            }
            #[cfg(feature = "xz")]
            Extension::TarXz => {
                let reader = xz::read::XzDecoder::new(file);
                let mut archive = tar::Archive::new(reader);
                archive.unpack(dst).map_err(BinaryError::DecompressError)?;
            }
            #[cfg(feature = "zip")]
            Extension::Zip => {
                let reader = std::io::Cursor::new(file);
                let mut archive = zip::ZipArchive::new(reader)
                    .map_err(|e| BinaryError::DecompressError(e.into()))?;
                archive
                    .extract(dst)
                    .map_err(|e| BinaryError::DecompressError(e.into()))?;
            } // #[cfg(feature = "pkg")]
              // Extension::Pkg => {
              //     let reader = std::io::Cursor::new(file);
              //     let mut archive = apple_flat_package::PkgReader::new(reader).unwrap();
              //     let pkgs = archive.component_packages().unwrap();
              //     let mut cpio = pkgs.first().unwrap().payload_reader().unwrap().unwrap();
              //     while let Some(next) = cpio.next() {
              //         let entry = next.unwrap();
              //         let mut file = Vec::new();
              //         cpio.read_to_end(&mut file).unwrap();
              //         if entry.file_size() != 0 {
              //             let dst = dst.join(entry.name());
              //             fs::create_dir_all(dst.parent().unwrap())?;
              //             fs::write(&dst, file).map_err(e)?;
              //         }
              //     }
              // }
        };

        // Update the checksum
        let checksum = sha256::digest(file);
        let mut path = dst.to_path_buf();
        path.push("checksum");
        if let Err(e) = fs::write(path, checksum) {
            println!("cargo:warning=Couldn't write the binary checksum {:?}", e);
        };
    }
    Ok(())
}

pub fn merge_binary(mut rhs: Value, mut lhs: Value, overwrite: bool) -> Result<Value, Error> {
    let r = rhs.as_object_mut().ok_or(Error::IncompatibleMerge)?;
    let l = lhs.as_object_mut().ok_or(Error::IncompatibleMerge)?;

    if overwrite {
        for (pkg, value) in l.iter_mut() {
            if value.get("url").is_some() {
                r.get_mut(pkg)
                    .and_then(|p| p.as_object_mut())
                    .and_then(|p| p.remove("follows"));
            }
            if let Some((provides, value)) = resolve_follow(pkg.clone(), value) {
                for pkg in provides {
                    let e = r.entry(pkg).or_insert(Value::Null);
                    *e = merge_base(e.take(), &value, true, &merge_default)?;
                    if let Value::Object(e) = e {
                        e.remove("url");
                    };
                }
            }
        }
    }

    let res = merge_base(rhs, &lhs, overwrite, &merge_default)?;

    let r = res.as_object().ok_or(Error::IncompatibleMerge)?;
    for value in r.values() {
        if value.get("url").is_some() && value.get("follows").is_some() {
            return Err(Error::IncompatibleMerge);
        }
    }

    Ok(res)
}

fn resolve_follow(name: String, value: &mut Value) -> Option<(Vec<String>, Value)> {
    let provides = value.as_object_mut()?.remove("provides")?;
    let provides = provides
        .as_array()?
        .iter()
        .filter_map(|p| p.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>();

    let value = Map::from_iter([("follows".into(), Value::String(name))]);
    Some((provides, Value::Object(value)))
}
