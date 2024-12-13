use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    fs,
    iter::FromIterator,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::{
    error::{BinaryError, Error},
    utils::merge_default,
};

/// The extension of the binary archive.
/// Support for different extensions is enabled using features.
#[derive(Debug)]
pub enum Extension {
    /// A `.tar.gz` archive.
    #[cfg(feature = "gz")]
    TarGz,
    /// A `.tar.xz` archive.
    #[cfg(feature = "xz")]
    TarXz,
    /// A `.zip` archive.
    #[cfg(feature = "zip")]
    Zip,
    Folder,
}

impl TryFrom<&str> for Extension {
    type Error = BinaryError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Path::new(value).try_into()
    }
}

impl TryFrom<&Path> for Extension {
    type Error = BinaryError;
    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        if path.is_dir() {
            return Ok(Self::Folder);
        };
        let Some(ext) = path.extension() else {
            return Err(BinaryError::UnsupportedExtension("<error>".into()));
        };
        match ext {
            #[cfg(feature = "gz")]
            e if e == "gz" || e == "tgz" => Ok(Extension::TarGz),
            #[cfg(feature = "xz")]
            e if e == "xz" => Ok(Extension::TarXz),
            #[cfg(feature = "zip")]
            e if e == "zip" => Ok(Extension::Zip),
            e => Err(BinaryError::UnsupportedExtension(
                e.to_str().unwrap().into(),
            )),
        }
    }
}

/// Binary locations can be specified either by describing its metadata or by refering to another
/// package. This helper enum allows deserializing both as valid versions.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Binary {
    Follow(FollowBinary),
    Url(UrlBinary),
}

impl TryFrom<Value> for Binary {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self, Self::Error> {
        Ok(value.try_into()?)
    }
}

/// A package that doesn't point to a binary itself but instead uses the metadata from another one.
/// While the `follows` field can be specified, it is usually more convenient to use `provides` in
/// the package that defines the binary url. Both of these are equivalent:
///
/// ```toml
/// [package.metadata.system-deps.a]
/// url = "..."
/// provides = [ "b" ]
///
/// [package.metadata.system-deps.b]
/// follows = "a"
/// ```
///
/// Specifying both an url and a followed package is incompatible and it will cause an error.
#[derive(Debug, Deserialize)]
pub struct FollowBinary {
    /// The package name to get the metadata from.
    follows: String,
}

/// Represents one location from where to download prebuilt binaries.
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

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Paths {
    paths: HashMap<String, Vec<PathBuf>>,
    follows: HashMap<String, String>,
    wildcards: HashMap<String, String>,
}

impl<T> FromIterator<(String, T)> for Paths
where
    Binary: TryFrom<T>,
{
    /// Uses the metadata from the cargo manifests and the environment to build a list of urls
    /// from where to download binaries for dependencies and adds them to their `PKG_CONFIG_PATH`.
    ///
    /// This function may panic, but only on unrecoverable results such as downloading or
    /// decompressing errors. While it would possible to pass these values to the caller, in this
    /// particular instance it would be hard to use this trait and it complicates error management.
    fn from_iter<I: IntoIterator<Item = (String, T)>>(binaries: I) -> Self {
        let mut res = Self::default();

        let (url_binaries, follow_binaries): (Vec<_>, Vec<_>) = binaries
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.try_into().ok()?)))
            .partition(|(_, bin)| matches!(bin, Binary::Url(_)));

        // Binaries with its own url
        thread::scope(|s| {
            for (name, bin) in url_binaries {
                let Binary::Url(bin) = bin else {
                    unreachable!();
                };

                let dst = Path::new(&crate::BUILD_TARGET_DIR).join(&name);
                res.paths.insert(
                    name,
                    bin.paths.iter().flatten().map(|p| dst.join(p)).collect(),
                );

                // Only refresh the binaries if there isn't already a valid copy
                let valid = check_valid_dir(&dst, bin.checksum.as_deref())
                    .unwrap_or_else(|e| panic!("{}", e));

                // Allow multiple downloads at the same time
                if !valid {
                    s.spawn(move || make_available(bin, &dst).map_err(|e| panic!("{}", e)));
                }
            }
        });

        // Check if the package provided extra configuration
        for (name, list) in res.paths.iter_mut() {
            let dst = Path::new(&crate::BUILD_TARGET_DIR).join(name);
            let Ok(info) = fs::read_to_string(dst.join("info.toml")) else {
                continue;
            };
            let Ok(table) = toml::from_str::<Table>(&info) else {
                continue;
            };
            if let Some(Value::Array(paths)) = table.get("paths") {
                for p in paths.iter().filter_map(|p| p.as_str()) {
                    let p = dst.join(p);
                    if !list.contains(&p) {
                        list.push(p);
                    }
                }
            }
        }

        // Binaries that follow others
        for (name, bin) in follow_binaries {
            let Binary::Follow(bin) = bin else {
                unreachable!();
            };
            if !res.paths.contains_key(&bin.follows) {
                panic!("{}", BinaryError::InvalidFollows(name, bin.follows));
            };
            match name.strip_suffix("*") {
                Some(wildcard) => res.wildcards.insert(wildcard.into(), bin.follows),
                None => res.follows.insert(name, bin.follows),
            };
        }

        res
    }
}

impl Paths {
    /// Returns the list of paths for a certain package. Matches wildcards but they never have
    /// priority over explicit urls or follows, even if they are defined higher in the hierarchy.
    pub fn get(&self, key: &str) -> Option<&Vec<PathBuf>> {
        if let Some(paths) = self.paths.get(key) {
            return Some(paths);
        };

        if let Some(follows) = self.follows.get(key) {
            return self.paths.get(follows);
        };

        self.wildcards.iter().find_map(|(k, v)| {
            key.starts_with(k)
                .then_some(v)
                .and_then(|v| self.paths.get(v))
        })
    }

    /// Serializes the path list.
    pub fn to_string(&self) -> Result<String, Error> {
        Ok(toml::to_string(self)?)
    }
}

/// Checks if the target directory is valid and if binaries need to be redownloaded.
/// On an `Ok` result, if the value is true it means that the directory is correct.
fn check_valid_dir(dst: &Path, checksum: Option<&str>) -> Result<bool, BinaryError> {
    // If it doesn't exist yet the download will need to happen
    if !dst.try_exists().map_err(BinaryError::InvalidDirectory)? {
        return Ok(false);
    }

    // Raise an error if it is a file
    if dst.is_file() {
        return Err(BinaryError::DirectoryIsFile(dst.display().to_string()));
    }

    // Check if the checksum is valid
    // If a checksum is not specified, assume the directory is invalid
    if let Some(ch) = checksum {
        let file = dst.join("checksum");
        Ok(file.is_file()
            && ch == fs::read_to_string(file).map_err(BinaryError::InvalidDirectory)?)
    } else {
        Ok(false)
    }
}

/// Retrieve a binary archive from the specified `url` and decompress it in the target directory.
/// "Download" is used as an umbrella term, since this can also be a local file.
fn make_available(bin: UrlBinary, dst: &Path) -> Result<(), BinaryError> {
    // TODO: Find a way of printing download/decompress progress
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    // Check whether the file is local or not
    let (url, local) = match bin.url.strip_prefix("file://") {
        Some(file) => (file, true),
        None => (bin.url.as_str(), false),
    };

    let ext = url.try_into()?;

    // Check if it is a folder and it can be symlinked
    if matches!(ext, Extension::Folder) {
        if !local {
            return Err(BinaryError::UnsupportedExtension("<folder>".into()));
        }
        let _l = LOCK.get_or_init(|| Mutex::new(())).lock();
        if !dst.read_link().is_ok_and(|l| l == Path::new(url)) {
            if dst.is_symlink() {
                std::fs::remove_file(dst).map_err(BinaryError::SymlinkError)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(url, dst).map_err(BinaryError::SymlinkError)?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(url, dst).map_err(BinaryError::SymlinkError)?;
        }
        return Ok(());
    }

    // Otherwise, use a local file or download from the web
    let file = if local {
        fs::read(url).map_err(BinaryError::LocalFileError)?
    } else {
        let res = attohttpc::get(url).send()?;
        res.error_for_status()?.bytes()?
    };

    // Verify the checksum
    let calculated = sha256::digest(&*file);
    let checksum = match bin.checksum {
        Some(ch) if *ch == calculated => Ok(ch),
        _ => Err(BinaryError::InvalidChecksum(
            url.into(),
            bin.checksum.unwrap_or("<empty>".into()),
            calculated,
        )),
    }?;
    fs::create_dir_all(dst).map_err(BinaryError::DecompressError)?;
    fs::write(dst.join("checksum"), checksum).map_err(BinaryError::DecompressError)?;

    // Decompress the binary archive
    decompress(&file, dst, ext)?;

    Ok(())
}

/// Extract a binary archive to the target directory. The methods for unpacking are
/// different depending on the extension. Each file type is gated behind a feature to
/// avoid having too many dependencies.
fn decompress(_file: &[u8], _dst: &Path, ext: Extension) -> Result<(), BinaryError> {
    match ext {
        #[cfg(feature = "gz")]
        Extension::TarGz => {
            let reader = flate2::read::GzDecoder::new(_file);
            let mut archive = tar::Archive::new(reader);
            archive.unpack(_dst).map_err(BinaryError::DecompressError)
        }
        #[cfg(feature = "xz")]
        Extension::TarXz => {
            let reader = xz::read::XzDecoder::new(_file);
            let mut archive = tar::Archive::new(reader);
            archive.unpack(_dst).map_err(BinaryError::DecompressError)
        }
        #[cfg(feature = "zip")]
        Extension::Zip => {
            let reader = std::io::Cursor::new(_file);
            let mut archive =
                zip::ZipArchive::new(reader).map_err(|e| BinaryError::DecompressError(e.into()))?;
            archive
                .extract(_dst)
                .map_err(|e| BinaryError::DecompressError(e.into()))
        }
        _ => unreachable!(),
    }
}

pub fn merge_binary(rhs: &mut Table, lhs: Table, overwrite: bool) -> Result<(), Error> {
    // Update the values for url and follows
    if overwrite {
        for (key, value) in lhs.iter() {
            if value.get("url").is_some() {
                if let Some(Value::Table(pkg)) = rhs.get_mut(key) {
                    pkg.remove("follows");
                }
            }
            if let Some(Value::Array(provides)) = value.get("provides") {
                for name in provides {
                    let name = name.as_str().ok_or(Error::IncompatibleMerge)?;
                    let pkg = rhs
                        .entry(name)
                        .or_insert(Value::Table(Table::new()))
                        .as_table_mut()
                        .unwrap();
                    pkg.insert("follows".into(), Value::String(key.into()));
                    pkg.remove("url");
                }
            }
        }
    }

    // The regular merge
    merge_default(rhs, lhs, overwrite)?;

    // Don't allow both url and follows for the same package
    for value in rhs.values() {
        if value.get("url").is_some() && value.get("follows").is_some() {
            return Err(Error::IncompatibleMerge);
        }
    }

    Ok(())
}
