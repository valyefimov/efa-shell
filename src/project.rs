use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Nearest project roots detected by walking up from a `cwd`, found
/// independently since a git root and a `package.json` root can differ
/// (e.g. monorepos, or a package nested inside a larger git repo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectRoots {
    pub git_root: Option<PathBuf>,
    pub package_root: Option<PathBuf>,
}

impl ProjectRoots {
    /// The single "project root" used for history's `project_root` column:
    /// git root wins over package root when both exist.
    pub fn primary(&self) -> Option<&Path> {
        self.git_root.as_deref().or(self.package_root.as_deref())
    }
}

/// Detect the nearest `.git` and `package.json` roots by walking up from
/// `cwd`, caching the result keyed on the last-seen `cwd` so repeated
/// keystrokes in the same directory don't re-walk the filesystem.
pub struct ProjectDetector {
    cache: Mutex<Option<(PathBuf, ProjectRoots)>>,
}

impl ProjectDetector {
    pub fn new() -> Self {
        ProjectDetector {
            cache: Mutex::new(None),
        }
    }

    pub fn detect(&self, cwd: &Path) -> ProjectRoots {
        let mut cache = self.cache.lock().expect("project detector mutex poisoned");
        if let Some((cached_cwd, roots)) = cache.as_ref()
            && cached_cwd == cwd
        {
            return roots.clone();
        }
        let roots = detect_roots(cwd);
        *cache = Some((cwd.to_path_buf(), roots.clone()));
        roots
    }
}

impl Default for ProjectDetector {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_roots(cwd: &Path) -> ProjectRoots {
    ProjectRoots {
        git_root: find_upwards(cwd, ".git"),
        package_root: find_upwards(cwd, "package.json"),
    }
}

/// Walk from `start` up to the filesystem root, returning the first
/// ancestor directory that contains `marker`.
fn find_upwards(start: &Path, marker: &str) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(marker).exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "efa-project-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        path.push(unique);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn finds_git_root_at_various_depths() {
        let root = tempdir();
        fs::create_dir_all(root.join(".git")).unwrap();
        let deep = root.join("a/b/c");
        fs::create_dir_all(&deep).unwrap();

        let roots = detect_roots(&deep);
        assert_eq!(roots.git_root, Some(root.clone()));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn monorepo_git_and_package_roots_diverge() {
        let root = tempdir();
        fs::create_dir_all(root.join(".git")).unwrap();
        let pkg_dir = root.join("packages/app");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("package.json"), "{}").unwrap();

        let roots = detect_roots(&pkg_dir);
        assert_eq!(roots.git_root, Some(root.clone()));
        assert_eq!(roots.package_root, Some(pkg_dir.clone()));
        assert_ne!(roots.git_root, roots.package_root);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn git_as_file_still_counts_as_root() {
        let root = tempdir();
        fs::write(root.join(".git"), "gitdir: /somewhere/else").unwrap();

        let roots = detect_roots(&root);
        assert_eq!(roots.git_root, Some(root.clone()));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_root_found_anywhere_returns_none() {
        // A path with no `.git` or `package.json` anywhere up the chain
        // (using a fresh temp dir guarantees no accidental markers above it
        // within the temp root itself).
        let root = tempdir();
        let roots = detect_roots(&root);
        assert_eq!(roots.package_root, None);
        // git_root may legitimately be Some(...) if the OS temp dir happens
        // to live inside a git repo; only assert package_root here.
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cache_reused_for_same_cwd() {
        let root = tempdir();
        fs::create_dir_all(root.join(".git")).unwrap();

        let detector = ProjectDetector::new();
        let first = detector.detect(&root);
        // Remove the marker; a cached result should still be returned since
        // only a `cd` (i.e. a differing cwd) triggers recomputation.
        fs::remove_dir_all(root.join(".git")).unwrap();
        let second = detector.detect(&root);
        assert_eq!(first, second);

        fs::remove_dir_all(&root).ok();
    }
}
