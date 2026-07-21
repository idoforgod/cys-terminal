use crate::browser_runtime::{BrowserError, TargetAssets, TargetTriple};
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
            return Err(BrowserError::runtime_path("target runtime root is not a directory"));
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
            for (kind, path) in [("supervisor", &supervisor), ("engine", &engine), ("chromium", &chromium)] {
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
        Ok(Self { target_root, supervisor, engine, chromium })
    }
}

fn resolve_without_links(base: &Path, relative: &Path) -> Result<PathBuf, BrowserError> {
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(BrowserError::runtime_path("runtime path contains a non-normal segment"));
        };
        current.push(part);
        reject_symlink(&current, "runtime path segment")?;
    }
    let canonical = current
        .canonicalize()
        .map_err(|e| BrowserError::runtime_path(format!("runtime path unavailable: {e}")))?;
    if !canonical.starts_with(base) {
        return Err(BrowserError::runtime_path("runtime path escapes target root"));
    }
    Ok(canonical)
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), BrowserError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| BrowserError::runtime_path(format!("{label} unavailable: {e}")))?;
    if metadata.file_type().is_symlink() {
        return Err(BrowserError::runtime_path(format!("{label} must not be a symlink")));
    }
    Ok(())
}
