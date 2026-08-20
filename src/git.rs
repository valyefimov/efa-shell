use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Common git subcommands, used for fallback prefix matching when history
/// has no match. Not exhaustive - just the ones people actually type.
pub const SUBCOMMANDS: &[&str] = &[
    "add",
    "am",
    "apply",
    "bisect",
    "blame",
    "branch",
    "checkout",
    "cherry-pick",
    "clean",
    "clone",
    "commit",
    "config",
    "describe",
    "diff",
    "fetch",
    "grep",
    "init",
    "log",
    "merge",
    "mv",
    "pull",
    "push",
    "rebase",
    "reflog",
    "remote",
    "reset",
    "restore",
    "revert",
    "rm",
    "show",
    "stash",
    "status",
    "submodule",
    "switch",
    "tag",
    "worktree",
];

/// Git subcommands that take a branch/ref name as their next argument,
/// where the branch you're already on isn't a useful target.
pub const BRANCH_TAKING_SUBCOMMANDS: &[&str] = &["checkout", "switch", "merge", "rebase"];

/// Git subcommands that take a branch/ref name as their next argument,
/// where the current branch remains a perfectly sensible target.
pub const REF_TAKING_SUBCOMMANDS: &[&str] = &["diff", "log", "show"];

/// Git subcommands that take a file/directory path as their next argument.
pub const PATH_TAKING_SUBCOMMANDS: &[&str] = &["add", "restore", "rm", "mv"];

/// Git subcommands that take a remote name as their next argument, and
/// optionally a branch name as the argument after that.
pub const REMOTE_TAKING_SUBCOMMANDS: &[&str] = &["push", "pull", "fetch"];

pub fn matching_subcommands(prefix: &str) -> Vec<&'static str> {
    SUBCOMMANDS
        .iter()
        .copied()
        .filter(|c| c.starts_with(prefix))
        .collect()
}

/// Resolved location of a git repo's shared ref data - the directory that
/// contains `HEAD`, `refs/heads/`, and `packed-refs`. For worktrees this is
/// the *main* repo's git dir, not the worktree's own `.git` file target,
/// since worktrees share branches but not `HEAD`.
struct RefSource {
    /// Directory to read `HEAD` from (the worktree's own, if applicable).
    head_dir: PathBuf,
    /// Directory to read `refs/heads` and `packed-refs` from (shared).
    common_dir: PathBuf,
}

/// Follow `.git` (file or directory) at `repo_root` down to the real git
/// dir, resolving worktree `commondir` indirection along the way.
fn resolve_ref_source(repo_root: &Path) -> Option<RefSource> {
    let dot_git = repo_root.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else if dot_git.is_file() {
        let contents = fs::read_to_string(&dot_git).ok()?;
        let path = contents.strip_prefix("gitdir:")?.trim();
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            repo_root.join(path)
        }
    } else {
        return None;
    };

    let commondir_file = git_dir.join("commondir");
    let common_dir = if let Ok(contents) = fs::read_to_string(&commondir_file) {
        let path = PathBuf::from(contents.trim());
        if path.is_absolute() {
            path
        } else {
            git_dir.join(path)
        }
    } else {
        git_dir.clone()
    };

    Some(RefSource {
        head_dir: git_dir,
        common_dir,
    })
}

/// Current branch name, or `None` for detached HEAD / no HEAD.
pub fn current_branch(repo_root: &Path) -> Option<String> {
    let source = resolve_ref_source(repo_root)?;
    let head = fs::read_to_string(source.head_dir.join("HEAD")).ok()?;
    let head = head.trim();
    head.strip_prefix("ref: refs/heads/")
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// All local branch names (loose refs + packed-refs, deduped and sorted).
/// Remote-tracking branches and peeled-tag lines are excluded.
pub fn local_branches(repo_root: &Path) -> Vec<String> {
    let Some(source) = resolve_ref_source(repo_root) else {
        return Vec::new();
    };

    let mut branches = Vec::new();
    let heads_dir = source.common_dir.join("refs/heads");
    collect_loose_refs(&heads_dir, &heads_dir, &mut branches);

    let packed = source.common_dir.join("packed-refs");
    if let Ok(contents) = fs::read_to_string(&packed) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some((_sha, refname)) = line.split_once(' ')
                && let Some(name) = refname.strip_prefix("refs/heads/")
            {
                branches.push(name.to_string());
            }
        }
    }

    branches.sort();
    branches.dedup();
    branches
}

/// All remote names (e.g. `origin`), read from `refs/remotes/<name>/...`
/// directories plus any packed `refs/remotes/<name>/...` entries.
pub fn remotes(repo_root: &Path) -> Vec<String> {
    let Some(source) = resolve_ref_source(repo_root) else {
        return Vec::new();
    };

    let mut remotes = Vec::new();
    let remotes_dir = source.common_dir.join("refs/remotes");
    if let Ok(entries) = fs::read_dir(&remotes_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                remotes.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }

    let packed = source.common_dir.join("packed-refs");
    if let Ok(contents) = fs::read_to_string(&packed) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some((_sha, refname)) = line.split_once(' ')
                && let Some(rest) = refname.strip_prefix("refs/remotes/")
                && let Some((name, _branch)) = rest.split_once('/')
            {
                remotes.push(name.to_string());
            }
        }
    }

    remotes.sort();
    remotes.dedup();
    remotes
}

/// Recurse `dir` (relative to `heads_root`) collecting loose ref file paths
/// as branch names, preserving nested `/`-separated names.
fn collect_loose_refs(dir: &Path, heads_root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_loose_refs(&path, heads_root, out);
        } else if let Ok(rel) = path.strip_prefix(heads_root) {
            let name = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if !name.is_empty() {
                out.push(name);
            }
        }
    }
}

fn refs_mtime(repo_root: &Path) -> Option<SystemTime> {
    let source = resolve_ref_source(repo_root)?;
    let heads_mtime = fs::metadata(source.common_dir.join("refs/heads"))
        .and_then(|m| m.modified())
        .ok();
    let packed_mtime = fs::metadata(source.common_dir.join("packed-refs"))
        .and_then(|m| m.modified())
        .ok();
    match (heads_mtime, packed_mtime) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

type CachedBranches = (PathBuf, Option<SystemTime>, Vec<String>);

/// Cache of local branch lists keyed on git root, invalidated on `mtime`
/// change of `refs/heads`/`packed-refs` so branches created mid-session
/// (without `cd`-ing away and back) still show up.
pub struct BranchCache {
    cache: Mutex<Option<CachedBranches>>,
}

impl BranchCache {
    pub fn new() -> Self {
        BranchCache {
            cache: Mutex::new(None),
        }
    }

    pub fn branches(&self, repo_root: &Path) -> Vec<String> {
        let mtime = refs_mtime(repo_root);
        let mut cache = self.cache.lock().expect("branch cache mutex poisoned");
        if let Some((cached_root, cached_mtime, branches)) = cache.as_ref()
            && cached_root == repo_root
            && *cached_mtime == mtime
        {
            return branches.clone();
        }
        let branches = local_branches(repo_root);
        *cache = Some((repo_root.to_path_buf(), mtime, branches.clone()));
        branches
    }
}

impl Default for BranchCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "efa-git-test-{}-{}",
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

    fn init_bare_git_layout(root: &Path) {
        fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
    }

    #[test]
    fn head_parsing_branch() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(current_branch(&root), Some("main".to_string()));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn head_parsing_detached() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::write(
            root.join(".git/HEAD"),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\n",
        )
        .unwrap();
        assert_eq!(current_branch(&root), None);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn loose_ref_recursion_including_nested_names() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::write(root.join(".git/refs/heads/main"), "sha").unwrap();
        fs::create_dir_all(root.join(".git/refs/heads/feature")).unwrap();
        fs::write(root.join(".git/refs/heads/feature/foo"), "sha").unwrap();

        let mut branches = local_branches(&root);
        branches.sort();
        assert_eq!(
            branches,
            vec!["feature/foo".to_string(), "main".to_string()]
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn packed_refs_excludes_comments_peeled_and_remote_lines() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::write(
            root.join(".git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n\
             abc123 refs/heads/develop\n\
             ^def456\n\
             abc789 refs/remotes/origin/main\n",
        )
        .unwrap();

        let branches = local_branches(&root);
        assert_eq!(branches, vec!["develop".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn empty_missing_refs_heads_returns_empty() {
        let root = tempdir();
        fs::create_dir_all(root.join(".git")).unwrap();
        assert_eq!(local_branches(&root), Vec::<String>::new());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dedup_loose_and_packed() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::write(root.join(".git/refs/heads/main"), "sha").unwrap();
        fs::write(root.join(".git/packed-refs"), "abc123 refs/heads/main\n").unwrap();

        let branches = local_branches(&root);
        assert_eq!(branches, vec!["main".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn subcommand_prefix_matching() {
        assert!(matching_subcommands("chec").contains(&"checkout"));
        assert!(matching_subcommands("zzz").is_empty());
    }

    #[test]
    fn remotes_from_loose_dir() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::create_dir_all(root.join(".git/refs/remotes/origin")).unwrap();
        fs::write(root.join(".git/refs/remotes/origin/main"), "sha").unwrap();

        assert_eq!(remotes(&root), vec!["origin".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remotes_from_packed_refs_dedup_with_loose() {
        let root = tempdir();
        init_bare_git_layout(&root);
        fs::create_dir_all(root.join(".git/refs/remotes/origin")).unwrap();
        fs::write(root.join(".git/refs/remotes/origin/main"), "sha").unwrap();
        fs::write(
            root.join(".git/packed-refs"),
            "abc123 refs/remotes/origin/main\ndef456 refs/remotes/upstream/main\n",
        )
        .unwrap();

        let mut found = remotes(&root);
        found.sort();
        assert_eq!(found, vec!["origin".to_string(), "upstream".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remotes_missing_returns_empty() {
        let root = tempdir();
        init_bare_git_layout(&root);
        assert_eq!(remotes(&root), Vec::<String>::new());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn git_as_file_worktree_resolution() {
        let main_root = tempdir();
        init_bare_git_layout(&main_root);
        fs::write(main_root.join(".git/refs/heads/main"), "sha").unwrap();

        let worktree_root = tempdir();
        let worktree_git_dir = main_root.join(".git/worktrees/wt");
        fs::create_dir_all(&worktree_git_dir).unwrap();
        fs::write(
            worktree_git_dir.join("commondir"),
            format!("{}", main_root.join(".git").display()),
        )
        .unwrap();
        fs::write(worktree_git_dir.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        fs::write(
            worktree_root.join(".git"),
            format!("gitdir: {}", worktree_git_dir.display()),
        )
        .unwrap();

        assert_eq!(current_branch(&worktree_root), Some("feature".to_string()));
        assert_eq!(local_branches(&worktree_root), vec!["main".to_string()]);

        fs::remove_dir_all(&main_root).ok();
        fs::remove_dir_all(&worktree_root).ok();
    }
}
