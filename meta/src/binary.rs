use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::{BinaryError, Error};

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
    /// A macOS `.pkg` (Apple flat package) archive.
    #[cfg(all(feature = "pkg", target_os = "macos"))]
    Pkg,
    /// A Windows Inno Setup `.exe` installer.
    #[cfg(all(feature = "inno", target_os = "windows"))]
    Inno,
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
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("<none>");
        match extension {
            #[cfg(all(feature = "pkg", target_os = "macos"))]
            "pkg" => Ok(Extension::Pkg),
            #[cfg(all(feature = "inno", target_os = "windows"))]
            "exe" => Ok(Extension::Inno),
            #[cfg(feature = "gz")]
            "gz" | "tgz" => Ok(Extension::TarGz),
            #[cfg(feature = "xz")]
            "xz" => Ok(Extension::TarXz),
            #[cfg(feature = "zip")]
            "zip" => Ok(Extension::Zip),
            e => Err(BinaryError::UnsupportedExtension(e.into())),
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
    /// package config files (`.pc`). These directories will be prepended to the `PKG_CONFIG_PATH` when
    /// compiling the affected libraries.
    paths: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Paths {
    paths: HashMap<String, Vec<PathBuf>>,
    follows: HashMap<String, String>,
    wildcards: HashMap<String, String>,
}

impl Paths {
    /// Uses the metadata from the cargo manifests and the environment to build a list of urls
    /// from where to download binaries for dependencies and adds them to their `PKG_CONFIG_PATH`.
    pub fn from_binaries<T>(binaries: impl IntoIterator<Item = (String, T)>) -> Result<Self, Error>
    where
        Binary: TryFrom<T>,
    {
        let mut res = Self::default();
        let mut auto_detect = std::collections::HashSet::new();

        let (url_binaries, follow_binaries): (Vec<_>, Vec<_>) = binaries
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.try_into().ok()?)))
            .partition(|(_, bin)| matches!(bin, Binary::Url(_)));

        // Binaries with its own url
        let errors: Vec<BinaryError> = thread::scope(|s| {
            let mut handles = Vec::new();

            for (name, bin) in url_binaries {
                let Binary::Url(bin) = bin else {
                    unreachable!();
                };

                let dst = Path::new(&crate::TARGET_DIR).join(&name);
                if let Some(ref paths) = bin.paths {
                    res.paths
                        .insert(name, paths.iter().map(|p| dst.join(p)).collect());
                } else {
                    auto_detect.insert(name.clone());
                    res.paths.insert(name, Vec::new());
                }

                // Only refresh the binaries if there isn't already a valid copy
                let valid = check_valid_dir(&dst, bin.checksum.as_deref())?;

                // Allow multiple downloads at the same time
                if !valid {
                    handles.push(s.spawn(move || make_available(bin, &dst)));
                }
            }

            Ok::<_, BinaryError>(
                handles
                    .into_iter()
                    .filter_map(|h| h.join().expect("download thread panicked").err())
                    .collect(),
            )
        })?;

        if let Some(e) = errors.into_iter().next() {
            return Err(e.into());
        }

        // Auto-detect pkgconfig directories for packages that didn't specify paths
        for name in &auto_detect {
            let dst = Path::new(&crate::TARGET_DIR).join(name);
            if let Some(list) = res.paths.get_mut(name) {
                *list = find_pkgconfig_dirs(&dst);
            }
        }

        // Check if the package provided extra configuration
        for (name, list) in res.paths.iter_mut() {
            let dst = Path::new(&crate::TARGET_DIR).join(name);
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
                return Err(BinaryError::InvalidFollows(name, bin.follows).into());
            };
            match name.strip_suffix("*") {
                Some(wildcard) => res.wildcards.insert(wildcard.into(), bin.follows),
                None => res.follows.insert(name, bin.follows),
            };
        }

        Ok(res)
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

    /// First executable `bin/pkg-config` shipped by any downloaded bundle,
    /// or `None`. Used to probe .pc files when the host has no pkg-config.
    pub fn pkg_config_binary(&self) -> Option<PathBuf> {
        let exe = if cfg!(windows) {
            "pkg-config.exe"
        } else {
            "pkg-config"
        };
        self.paths.keys().find_map(|name| {
            let candidate = Path::new(&crate::TARGET_DIR)
                .join(name)
                .join("bin")
                .join(exe);
            which::which(&candidate).ok()
        })
    }

    /// Returns the directories (one per binary install) where runtime DLLs
    /// live. On Windows the test/run binaries cannot find these via
    /// `cargo:rustc-link-search`; the loader looks at `PATH` or the exe's
    /// directory at runtime, so the caller is expected to surface these to
    /// the user.
    #[cfg(windows)]
    pub fn dll_search_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for name in self.paths.keys() {
            let bin = Path::new(&crate::TARGET_DIR).join(name).join("bin");
            if bin.is_dir() && seen.insert(bin.clone()) {
                out.push(bin);
            }
        }
        out
    }
}

/// Iteratively scan `dir` for subdirectories named "pkgconfig" and return their paths.
/// Does not follow symlinks. Limited to `MAX_DEPTH` levels to avoid excessive traversal.
fn find_pkgconfig_dirs(dir: &Path) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 10;

    let mut result = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((current, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Use symlink_metadata (lstat) to avoid following symlinks
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                if path.file_name().is_some_and(|n| n == "pkgconfig") {
                    result.push(path);
                } else if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    result
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
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    // Check whether the file is local or not
    // file:// URLs use three slashes for absolute paths (file:///path),
    // leaving a leading '/' after stripping "file://". On Windows this
    // produces "/C:/..." which is not a valid path, so strip it.
    let (url, local) = match bin.url.strip_prefix("file://") {
        #[cfg(windows)]
        Some(file) => (file.strip_prefix('/').unwrap_or(file), true),
        #[cfg(not(windows))]
        Some(file) => (file, true),
        None => (bin.url.as_str(), false),
    };

    let ext = url.try_into()?;

    // Check if it is a folder and it can be linked
    if matches!(ext, Extension::Folder) {
        if !local {
            return Err(BinaryError::UnsupportedExtension("<folder>".into()));
        }
        let _l = LOCK.get_or_init(|| Mutex::new(())).lock();
        create_link(Path::new(url), dst)?;
        return Ok(());
    }

    // Use a local file or download from the web
    let file = if local {
        fs::read(url).map_err(BinaryError::LocalFileError)?
    } else {
        let res = attohttpc::get(url)
            .header("User-Agent", "system-deps")
            .send()?;
        if res.status() == attohttpc::StatusCode::IM_A_TEAPOT {
            return Err(BinaryError::AntiBot(url.into()));
        }
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

    // Decompress the binary archive
    decompress(&file, dst, ext)?;

    // Generate info.toml with auto-detected pkgconfig paths
    create_info_file(dst)?;

    // Patch the bundle's `.pc` files so Windows pkg-config can't inflate
    // `${libdir}` via its broken auto-prefix heuristic (see the helper).
    #[cfg(windows)]
    rewrite_pc_files(dst)?;

    // Write the checksum last so that concurrent build scripts of other
    // crates (each running their own `system-deps` build.rs) cannot observe
    // a "valid" directory until extraction and info.toml are fully written.
    // Without this, `check_valid_dir` would return true mid-decompress and
    // `find_pkgconfig_dirs` would observe an incomplete tree, producing an
    // empty `paths.toml` entry (race condition).
    fs::write(dst.join("checksum"), checksum).map_err(BinaryError::DecompressError)?;

    Ok(())
}

/// Generate an `info.toml` file listing all directories containing `.pc` files.
/// Skips generation if `info.toml` already exists.
fn create_info_file(dst: &Path) -> Result<(), BinaryError> {
    let info_path = dst.join("info.toml");
    if info_path.exists() {
        return Ok(());
    }

    let pc_dirs: Vec<String> = find_pkgconfig_dirs(dst)
        .into_iter()
        .filter_map(|p| {
            p.strip_prefix(dst)
                .ok()
                .map(|rel| rel.to_string_lossy().into_owned())
        })
        .collect();

    let mut table = toml::Table::new();
    table.insert(
        "paths".to_string(),
        toml::Value::Array(pc_dirs.into_iter().map(toml::Value::String).collect()),
    );

    fs::write(
        info_path,
        toml::to_string(&table)
            .map_err(|e| BinaryError::DecompressError(std::io::Error::other(e)))?,
    )
    .map_err(BinaryError::DecompressError)
}

/// Pre-expand every `${var}` reference in each `.pc` file under `dst` and
/// drop the `prefix=` declaration line.
///
/// Windows `pkg-config` 0.29.2 enables `--define-prefix` by default, which
/// blindly strips two trailing components from `pcfiledir` to recompute
/// `prefix` regardless of what the .pc file declares. That heuristic is
/// correct for `<bundle>/lib/pkgconfig/foo.pc` (yields `<bundle>`) but wrong
/// for `<bundle>/lib/gstreamer-1.0/pkgconfig/foo.pc` (yields `<bundle>/lib`),
/// which inflates `${libdir}` to `<bundle>/lib/lib` and emits broken `-L`
/// flags like `<bundle>/lib/lib/gstreamer-1.0`. The redefine fires only when
/// a `prefix=` declaration is parsed, so dropping that line — combined with
/// rewriting every `${var}` to its resolved absolute path beforehand —
/// short-circuits the heuristic without needing a CLI flag (which we can't
/// pass through `pkg-config-rs::Command::new`).
///
/// `pkgconf` (the modern replacement for the original `pkg-config`) does not
/// suffer from this. We keep this workaround because the GStreamer Windows
/// installer still ships `pkg-config 0.29.2`.
#[cfg(windows)]
fn rewrite_pc_files(dst: &Path) -> Result<(), BinaryError> {
    for pc_dir in find_pkgconfig_dirs(dst) {
        let entries = match fs::read_dir(&pc_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "pc") {
                rewrite_pc_file(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn rewrite_pc_file(path: &Path) -> Result<(), BinaryError> {
    let content = fs::read_to_string(path).map_err(BinaryError::DecompressError)?;
    let pcfiledir = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let mut vars: Vec<(String, String)> = vec![("pcfiledir".into(), pcfiledir)];
    let mut out = String::with_capacity(content.len());

    for line in content.lines() {
        let substituted = expand_vars(line, &vars);

        // Detect a top-level `var=value` declaration: `key` must be a bare
        // identifier (no whitespace, no colon — a colon would mean this is a
        // `Name:`/`Libs:`/`Cflags:` field).
        let is_var_line = substituted
            .find('=')
            .filter(|&eq| {
                let prefix = &substituted[..eq];
                !prefix.is_empty()
                    && !prefix.contains(':')
                    && prefix
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            })
            .map(|eq| (substituted[..eq].trim().to_string(), substituted[eq + 1..].trim().to_string()));

        if let Some((key, value)) = is_var_line {
            // Always record the var (later lines need it for substitution).
            // For `prefix`, normalise `..` segments so downstream references
            // resolve to a clean absolute path, then drop the line so the
            // pkg-config 0.29.2 `--define-prefix` heuristic finds nothing
            // to override.
            if key == "prefix" {
                vars.push((key, normalise_path(&value)));
                continue;
            }
            vars.push((key, value));
        }
        out.push_str(&substituted);
        out.push('\n');
    }

    fs::write(path, out).map_err(BinaryError::DecompressError)
}

/// Resolve `..`/`.` segments in a forward-slash path. Pure string work,
/// no filesystem access — operates on whatever `prefix` evaluates to,
/// even when the parent directory does not exist yet.
#[cfg(windows)]
fn normalise_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            ".." => {
                if matches!(out.last(), Some(&seg) if seg != ".." && !seg.is_empty() && !seg.ends_with(':')) {
                    out.pop();
                } else {
                    out.push(part);
                }
            }
            "." | "" => {
                if out.is_empty() {
                    out.push(part);
                }
            }
            _ => out.push(part),
        }
    }
    out.join("/")
}

#[cfg(windows)]
fn expand_vars(line: &str, vars: &[(String, String)]) -> String {
    let mut s = line.to_string();
    let mut changed = true;
    // Iterate until fixpoint to handle nested expansions (var values
    // referencing other vars already in `vars`).
    while changed {
        changed = false;
        for (k, v) in vars {
            let pat = format!("${{{}}}", k);
            if s.contains(&pat) {
                s = s.replace(&pat, v);
                changed = true;
            }
        }
    }
    s
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
            let reader = liblzma::read::XzDecoder::new(_file);
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
        #[cfg(all(feature = "pkg", target_os = "macos"))]
        Extension::Pkg => {
            let reader = std::io::Cursor::new(_file);
            pkg_extractor::PkgExtractor::new(reader, Some(_dst.into()))
                .extract()
                .map_err(|e| BinaryError::DecompressError(std::io::Error::other(format!("{e:?}"))))
        }
        #[cfg(all(feature = "inno", target_os = "windows"))]
        Extension::Inno => {
            use std::io::{Seek, SeekFrom};
            let mut cursor = std::io::Cursor::new(_file);
            let inno = inno::Inno::new(&mut cursor)
                .map_err(|e| BinaryError::DecompressError(std::io::Error::other(e.to_string())))?;
            cursor
                .seek(SeekFrom::Start(0))
                .map_err(BinaryError::DecompressError)?;
            inno.extract_all(&mut cursor, _dst)
                .map_err(|e| BinaryError::DecompressError(std::io::Error::other(e.to_string())))
        }
        _ => unreachable!(),
    }
}

/// Create a symlink (Unix) or junction (Windows) from `dst` pointing to `src`.
/// If `dst` already points to `src`, this is a no-op.
fn create_link(src: &Path, dst: &Path) -> Result<(), BinaryError> {
    if dst.read_link().is_ok_and(|l| l == src) {
        return Ok(());
    }
    if dst.read_link().is_ok() {
        // Remove existing symlink/junction
        #[cfg(unix)]
        fs::remove_file(dst).map_err(BinaryError::SymlinkError)?;
        #[cfg(windows)]
        fs::remove_dir(dst).map_err(BinaryError::SymlinkError)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(src, dst).map_err(BinaryError::SymlinkError)?;
    // Use a junction on Windows instead of a symlink to avoid requiring
    // administrator privileges or Developer Mode.
    #[cfg(windows)]
    {
        // mklink requires backslash paths; forward slashes cause
        // "invalid modifier" errors in cmd.exe.
        let dst_str = dst.to_string_lossy().replace('/', "\\");
        let src_str = src.to_string_lossy().replace('/', "\\");
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J", &dst_str, &src_str])
            .output()
            .map_err(BinaryError::SymlinkError)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let msg = format!(
                "mklink /J {dst_str} {src_str} failed: {}{}",
                stderr.trim(),
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!(" (stdout: {})", stdout.trim())
                }
            );
            return Err(BinaryError::SymlinkError(std::io::Error::other(msg)));
        }
    }
    Ok(())
}

pub fn merge(rhs: &mut Table, lhs: Table, force: bool) -> Result<(), Error> {
    // Update the values for url and follows
    if force {
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
    crate::parse::merge(rhs, lhs, force)?;

    // Don't allow both url and follows for the same package
    for value in rhs.values() {
        if value.get("url").is_some() && value.get("follows").is_some() {
            return Err(Error::IncompatibleMerge);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("system_deps_test_{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn find_pkgconfig_empty_dir() {
        let dir = create_temp_dir("empty");
        let result = find_pkgconfig_dirs(&dir);
        assert!(result.is_empty());
    }

    #[test]
    fn find_pkgconfig_nonexistent_dir() {
        let dir = PathBuf::from("/tmp/system_deps_test_nonexistent_dir_that_does_not_exist");
        let result = find_pkgconfig_dirs(&dir);
        assert!(result.is_empty());
    }

    #[test]
    fn find_pkgconfig_single() {
        let dir = create_temp_dir("single");
        fs::create_dir_all(dir.join("lib/pkgconfig")).unwrap();
        let result = find_pkgconfig_dirs(&dir);
        assert_eq!(result, vec![dir.join("lib/pkgconfig")]);
    }

    #[test]
    fn find_pkgconfig_multiple() {
        let dir = create_temp_dir("multiple");
        fs::create_dir_all(dir.join("lib/pkgconfig")).unwrap();
        fs::create_dir_all(dir.join("share/pkgconfig")).unwrap();

        let mut result = find_pkgconfig_dirs(&dir);
        result.sort();
        let mut expected = vec![dir.join("lib/pkgconfig"), dir.join("share/pkgconfig")];
        expected.sort();
        assert_eq!(result, expected);
    }

    #[test]
    fn find_pkgconfig_nested_depths() {
        let dir = create_temp_dir("nested");
        fs::create_dir_all(dir.join("a/b/pkgconfig")).unwrap();
        fs::create_dir_all(dir.join("c/pkgconfig")).unwrap();

        let mut result = find_pkgconfig_dirs(&dir);
        result.sort();
        let mut expected = vec![dir.join("a/b/pkgconfig"), dir.join("c/pkgconfig")];
        expected.sort();
        assert_eq!(result, expected);
    }

    #[test]
    fn find_pkgconfig_file_ignored() {
        let dir = create_temp_dir("file_ignored");
        // Create a file named "pkgconfig" — should be ignored
        fs::write(dir.join("pkgconfig"), "not a directory").unwrap();
        // Create a real pkgconfig dir elsewhere
        fs::create_dir_all(dir.join("lib/pkgconfig")).unwrap();

        let result = find_pkgconfig_dirs(&dir);
        assert_eq!(result, vec![dir.join("lib/pkgconfig")]);
    }
}
