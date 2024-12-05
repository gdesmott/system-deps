use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::Error;

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
    /// An Apple package archive. Untested.
    #[cfg(feature = "pkg")]
    Pkg,
}

/// Represents one location from where to download library binaries.
#[derive(Debug, Deserialize)]
pub struct Binary {
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
    pkg_paths: Vec<String>,
    /// Controls if the paths from this binary apply to all packages or just to this one.
    global: Option<bool>,
    /// The `system-deps` formatted name of another library which has binaries specified.
    /// This library will alias the configuration of the followed one. If `url` is specified
    /// alongside this field, it will no longer follow the original configuration.
    follows: Option<String>,
}

impl Binary {
    pub fn paths(&self, name: &str) -> Result<HashSet<PathBuf>, Error> {
        // Set this binary to follow
        //if let Some(follows) = self.follows {
        //    follow_list.insert(name.clone(), follows);
        //}

        // The binaries are stored in the target dir set by `system_deps_meta`.
        // If they are specific to a dependency, they live in a subfolder.
        let mut dst = PathBuf::from(&crate::BUILD_TARGET_DIR);
        if !name.is_empty() {
            dst.push(name);
        };

        // Only download the binaries if there isn't already a valid copy
        if !check_valid_dir(&dst, self.checksum.as_deref())? {
            download(&self.url, &dst)?;
        }

        Ok(self.pkg_paths.iter().map(|p| dst.join(p)).collect())
    }

    pub fn is_global(&self) -> bool {
        self.global.unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct BinaryPaths(HashMap<String, Vec<PathBuf>>);

impl<I: IntoIterator<Item = (String, Binary)>> From<I> for BinaryPaths {
    /// Uses the metadata from the cargo manifests and the environment to build a list of urls
    /// from where to download binaries for dependencies and adds them to their `PKG_CONFIG_PATH`.
    fn from(binaries: I) -> Self {
        let mut paths: HashMap<String, Vec<PathBuf>> = HashMap::new();

        for (name, bin) in binaries {
            let p = bin.paths(&name).unwrap();
            if bin.is_global() {
                paths
                    .entry("".into())
                    .or_default()
                    .extend(p.iter().cloned())
            }
            paths.entry(name).or_default().extend(p.into_iter());
        }

        Self(paths)
    }
}

impl BinaryPaths {
    pub fn build(self) -> String {
        let options = self
            .0
            .into_iter()
            .map(|(name, paths)| format!(r#""{}" => &{:?},"#, name, paths))
            .collect::<Vec<_>>()
            .join("\n        ");

        format!(
            r#"
/// TODO: 
pub fn get_path(name: &str) -> &[&'static str] {{
    match name {{
        {}
        _ => &[],
    }}
}}
"#,
            options
        )
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

    // Go through the list of follows and if they don't already have binaries,
    // link them to the followed one.
    //for (from, to) in follow_list {
    //    if !paths.contains_key(&from) {
    //        let followed = paths
    //            .get(to.as_str())
    //            .unwrap_or_else(|| {
    //                panic!(
    //                    "The library `{}` tried to follow `{}` but it doesn't exist",
    //                    from, to,
    //                )
    //            })
    //            .clone();
    //        paths.insert(from, followed);
    //    };
    //}
}

/// Checks if the target directory is valid and if binaries need to be redownloaded.
/// On an `Ok` result, if the value is true it means that the directory is correct.
fn check_valid_dir(dst: &Path, checksum: Option<&str>) -> Result<bool, Error> {
    let e = |e| Error::InvalidDirectory(e);

    // If it doesn't exist yet the download will need to happen
    if !dst.try_exists().map_err(e)? {
        return Ok(false);
    }

    // Raise an error if it is a file
    if dst.is_file() {
        return Err(Error::DirectoryIsFile(dst.display().to_string()));
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
fn download(url: &str, dst: &Path) -> Result<(), Error> {
    let ext = match url {
        #[cfg(feature = "gz")]
        u if u.ends_with(".tar.gz") => Ok(Extension::TarGz),
        #[cfg(feature = "xz")]
        u if u.ends_with(".tar.xz") => Ok(Extension::TarXz),
        #[cfg(feature = "zip")]
        u if u.ends_with(".zip") => Ok(Extension::Zip),
        #[cfg(feature = "pkg")]
        u if u.ends_with(".pkg") => Ok(Extension::Pkg),
        u => Err(Error::InvalidExtension(u.into())),
    };

    // Local file
    if let Some(file_path) = url.strip_prefix("file://") {
        let path = Path::new(file_path);
        match ext {
            Ok(ext) => {
                let file = fs::read(path).map_err(|e| Error::LocalFileError(e))?;
                decompress(&file, dst, ext)?;
            }
            Err(e) => {
                // If it is a folder it can be symlinked
                if !path.is_dir() {
                    return Err(e);
                }
                if !dst.read_link().is_ok_and(|l| l == path) {
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(file_path, dst)
                        .map_err(|e| Error::SymlinkError(e))?;
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_dir(file_path, dst)
                        .map_err(|e| Error::SymlinkError(e))?;
                }
            }
        };
    }
    // Download from the web
    else {
        #[cfg(not(feature = "web"))]
        panic!("To download a binary file you must enable the `web` feature");
        #[cfg(feature = "web")]
        {
            let ext = ext?;
            let file = reqwest::blocking::get(url).and_then(|req| req.bytes())?;
            decompress(&file, dst, ext)?;
        }
    }

    Ok(())
}

/// Extract a binary archive to the target directory. The methods for unpacking are
/// different depending on the extension. Each file type is gated behind a feature to
/// avoid having too many dependencies.
#[allow(unused)]
fn decompress(file: &[u8], dst: &Path, ext: Extension) -> Result<(), Error> {
    #[cfg(any(feature = "gz", feature = "xz", feature = "zip", feature = "pkg"))]
    {
        let e = |e| Error::DecompressError(e);

        match ext {
            #[cfg(feature = "gz")]
            Extension::TarGz => {
                let reader = flate2::read::GzDecoder::new(file);
                let mut archive = tar::Archive::new(reader);
                archive.unpack(dst).map_err(e)?;
            }
            #[cfg(feature = "xz")]
            Extension::TarXz => {
                let reader = xz::read::XzDecoder::new(file);
                let mut archive = tar::Archive::new(reader);
                archive.unpack(dst).map_err(e)?;
            }
            #[cfg(feature = "zip")]
            Extension::Zip => {
                let reader = io::Cursor::new(file);
                let mut archive = zip::ZipArchive::new(reader).map_err(e)?;
                archive.extract(dst).map_err(e)?;
            }
            #[cfg(feature = "pkg")]
            Extension::Pkg => {
                // TODO: Test this with actual pkg files, do they have pc files inside?
                // TODO: Error handling
                let reader = io::Cursor::new(file);
                let mut archive = apple_flat_package::PkgReader::new(reader).unwrap();
                let pkgs = archive.component_packages().unwrap();
                let mut cpio = pkgs.first().unwrap().payload_reader().unwrap().unwrap();
                while let Some(next) = cpio.next() {
                    let entry = next.unwrap();
                    let mut file = Vec::new();
                    cpio.read_to_end(&mut file).unwrap();
                    if entry.file_size() != 0 {
                        let dst = dst.join(entry.name());
                        fs::create_dir_all(dst.parent().unwrap())?;
                        fs::write(&dst, file).map_err(e)?;
                    }
                }
            }
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
