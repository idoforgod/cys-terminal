use crate::browser_runtime::{BrowserError, TargetAssets, TargetTriple};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePaths {
    pub target_root: PathBuf,
    pub supervisor: PathBuf,
    pub engine: PathBuf,
    pub chromium: PathBuf,
}

impl RuntimePaths {
    /// Resolve only the fixed layout selected by the signed manifest. Caller
    /// supplied executable paths never enter this interface.
    pub fn resolve_existing(
        runtime_root: &Path,
        target: &TargetTriple,
        assets: &TargetAssets,
    ) -> Result<Self, BrowserError> {
        if !runtime_root.is_absolute() {
            return Err(BrowserError::runtime_path("runtime root must be absolute"));
        }
        reject_symlink(runtime_root, "runtime root")?;
        let root = runtime_root
            .canonicalize()
            .map_err(|e| BrowserError::runtime_path(format!("runtime root unavailable: {e}")))?;
        let target_root = resolve_without_links(&root, Path::new(target.as_str()))?;
        if !target_root.is_dir() {
            return Err(BrowserError::runtime_path(
                "target runtime root is not a directory",
            ));
        }
        let suffix = target.executable_suffix();
        let supervisor = resolve_without_links(
            &target_root,
            Path::new(&format!("supervisor/cys-browserd{suffix}")),
        )?;
        let engine = resolve_without_links(
            &target_root,
            Path::new(&format!("engine/cys-browser-engine{suffix}")),
        )?;
        let chromium = resolve_without_links(&target_root, Path::new(&assets.chromium_executable))?;
        for (kind, path) in [
            ("supervisor", &supervisor),
            ("engine", &engine),
            ("chromium", &chromium),
        ] {
            let metadata = std::fs::metadata(path).map_err(|e| {
                BrowserError::runtime_path(format!("{kind} executable unavailable: {e}"))
            })?;
            if !metadata.is_file() {
                return Err(BrowserError::runtime_path(format!(
                    "{kind} executable is not a regular file"
                )));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for (kind, path) in [
                ("supervisor", &supervisor),
                ("engine", &engine),
                ("chromium", &chromium),
            ] {
                let mode = std::fs::metadata(path)
                    .map_err(|e| BrowserError::runtime_path(format!("{kind} metadata: {e}")))?
                    .permissions()
                    .mode();
                if mode & 0o111 == 0 {
                    return Err(BrowserError::runtime_path(format!(
                        "{kind} executable has no execute bit"
                    )));
                }
            }
        }
        Ok(Self {
            target_root,
            supervisor,
            engine,
            chromium,
        })
    }

    pub fn verify_pinned_executables(&self, assets: &TargetAssets) -> Result<(), BrowserError> {
        verify_file_hash(&self.supervisor, &assets.supervisor_sha256, "supervisor")?;
        verify_file_hash(&self.engine, &assets.engine_sha256, "engine")?;
        let chromium_root = self.target_root.join("chromium");
        let actual_tree = hash_tree(&chromium_root)?;
        if actual_tree != assets.chromium_tree_sha256.to_ascii_lowercase() {
            return Err(BrowserError::runtime_integrity(
                "Chromium tree hash mismatch",
            ));
        }
        Ok(())
    }
}

pub fn hash_tree(root: &Path) -> Result<String, BrowserError> {
    if !root.is_dir() {
        return Err(BrowserError::runtime_integrity(
            "Chromium tree root is not a directory",
        ));
    }
    let mut files = Vec::new();
    collect_tree_files(root, root, &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    digest.update(b"cys.browser.chromium-tree.v1\0");
    for relative in files {
        let relative_bytes = relative.to_string_lossy().replace('\\', "/").into_bytes();
        digest.update((relative_bytes.len() as u64).to_be_bytes());
        digest.update(&relative_bytes);
        let path = root.join(&relative);
        let mut file = std::fs::File::open(&path).map_err(|error| {
            BrowserError::runtime_integrity(format!("Chromium tree file open failed: {error}"))
        })?;
        let size = file
            .metadata()
            .map_err(|error| {
                BrowserError::runtime_integrity(format!("Chromium tree metadata failed: {error}"))
            })?
            .len();
        digest.update(size.to_be_bytes());
        std::io::copy(&mut file, &mut digest).map_err(|error| {
            BrowserError::runtime_integrity(format!("Chromium tree hash failed: {error}"))
        })?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect_tree_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), BrowserError> {
    let entries = std::fs::read_dir(current).map_err(|error| {
        BrowserError::runtime_integrity(format!("Chromium tree read failed: {error}"))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            BrowserError::runtime_integrity(format!("Chromium tree entry failed: {error}"))
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            BrowserError::runtime_integrity(format!("Chromium tree metadata failed: {error}"))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(BrowserError::runtime_integrity(
                "Chromium tree contains a symlink",
            ));
        }
        if metadata.is_dir() {
            collect_tree_files(root, &path, output)?;
        } else if metadata.is_file() {
            output.push(
                path.strip_prefix(root)
                    .map_err(|_| {
                        BrowserError::runtime_integrity("Chromium tree path escaped its root")
                    })?
                    .to_path_buf(),
            );
        } else {
            return Err(BrowserError::runtime_integrity(
                "Chromium tree contains a non-regular entry",
            ));
        }
    }
    Ok(())
}

fn verify_file_hash(path: &Path, expected: &str, label: &str) -> Result<(), BrowserError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| BrowserError::runtime_integrity(format!("{label} open failed: {e}")))?;
    let mut digest = Sha256::new();
    std::io::copy(&mut file, &mut digest)
        .map_err(|e| BrowserError::runtime_integrity(format!("{label} hash failed: {e}")))?;
    let actual = format!("{:x}", digest.finalize());
    if actual != expected.to_ascii_lowercase() {
        return Err(BrowserError::runtime_integrity(format!(
            "{label} hash mismatch"
        )));
    }
    Ok(())
}

fn resolve_without_links(base: &Path, relative: &Path) -> Result<PathBuf, BrowserError> {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(BrowserError::runtime_path(
                "runtime path contains a non-normal segment",
            ));
        };
        current.push(part);
        reject_symlink(&current, "runtime path segment")?;
    }
    let canonical = current
        .canonicalize()
        .map_err(|e| BrowserError::runtime_path(format!("runtime path unavailable: {e}")))?;
    if !canonical.starts_with(base) {
        return Err(BrowserError::runtime_path(
            "runtime path escapes target root",
        ));
    }
    Ok(canonical)
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), BrowserError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| BrowserError::runtime_path(format!("{label} unavailable: {e}")))?;
    if metadata.file_type().is_symlink() {
        return Err(BrowserError::runtime_path(format!(
            "{label} must not be a symlink"
        )));
    }
    Ok(())
}
