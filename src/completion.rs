use std::path::{Path, PathBuf};
use std::sync::Arc;

use nu_ansi_term::{Color, Style};
use reedline::{
    Completer, CompletionResult, Hinter, History as ReedlineHistory, Span,
    Suggestion as ReedlineSuggestion,
};

use crate::db::Db;
use crate::gh;
use crate::git;
use crate::pkgmgr;
use crate::project::ProjectDetector;

fn is_pkg_tool(tool: &str) -> bool {
    matches!(tool, "pnpm" | "npm" | "yarn")
}

/// Split `text` into its complete leading tokens and the (possibly empty)
/// token currently being typed - i.e. the token immediately before the
/// cursor, with no trailing whitespace consumed.
fn split_current(text: &str) -> (Vec<&str>, &str) {
    if text.is_empty() {
        return (Vec::new(), "");
    }
    if text.ends_with(char::is_whitespace) {
        (text.split_whitespace().collect(), "")
    } else {
        let mut toks: Vec<&str> = text.split_whitespace().collect();
        let prefix = toks.pop().unwrap_or("");
        (toks, prefix)
    }
}

/// Split a path-ish token into the directory to list and the filename
/// prefix to match within it. `"sub/di"` -> (dir `sub/`, prefix `"di"`); a
/// prefix with no `/` lists `cwd` itself.
fn resolve_prefix_dir(cwd: &Path, prefix: &str) -> (PathBuf, String, String) {
    match prefix.rfind('/') {
        Some(idx) => {
            let dir_part = &prefix[..=idx];
            let file_part = &prefix[idx + 1..];
            let resolved_dir = if let Some(rest) = dir_part.strip_prefix('/') {
                PathBuf::from("/").join(rest)
            } else if let Some(rest) = dir_part.strip_prefix("~/") {
                dirs::home_dir()
                    .map(|h| h.join(rest))
                    .unwrap_or_else(|| cwd.join(dir_part))
            } else {
                cwd.join(dir_part)
            };
            (resolved_dir, dir_part.to_string(), file_part.to_string())
        }
        None => (cwd.to_path_buf(), String::new(), prefix.to_string()),
    }
}

/// Filesystem-entry candidates for a `<partial-path>` argument. Directory
/// names get a trailing `/` so completion can keep drilling in; hidden
/// entries (dotfiles) are only offered once the user has started typing a
/// `.`, matching typical shell completion behavior.
fn fs_path_candidates(cwd: &Path, prefix: &str, dirs_only: bool) -> Vec<String> {
    let (dir, display_prefix, file_prefix) = resolve_prefix_dir(cwd, prefix);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        if dirs_only && !is_dir {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !file_prefix.starts_with('.') {
            continue;
        }
        if name.starts_with(&file_prefix) {
            let suffix = if is_dir { "/" } else { "" };
            out.push(format!("{display_prefix}{name}{suffix}"));
        }
    }
    out
}

/// Candidates for the token currently being typed, based on git/gh/package-
/// manager/filesystem knowledge only (no history). Each candidate is the
/// *completed final token* (e.g. `"checkout"`, a branch name, a script
/// name, a directory name) - not a full command line.
///
/// Priority order, first match wins (mirrors the plan's fallback chain):
/// 1. `git <partial-subcommand>`
/// 2. `git (checkout|switch|merge|rebase) <partial-branch>` (git repo only,
///    current branch excluded)
/// 3. `git (diff|log|show) <partial-branch>` (git repo only, current branch
///    included)
/// 4. `git (add|restore|rm|mv) <partial-path>` - files and directories
/// 5. `git (push|pull|fetch) <partial-remote>`, then
///    `git (push|pull|fetch) <remote> <partial-branch>`
/// 6. `gh <partial-command>` / `gh <known-command> <partial-subcommand>`
/// 7. `(pnpm|npm|yarn)( run)? <partial-script-or-subcommand>`
/// 8. `cd <partial-path>` - directory names from the filesystem
#[allow(clippy::too_many_arguments)]
fn fallback_last_token_candidates(
    toks: &[&str],
    prefix: &str,
    in_git_repo: bool,
    branches: &[String],
    current_branch: Option<&str>,
    remotes: &[String],
    package_scripts: &[String],
    cwd: &Path,
) -> Vec<String> {
    match toks {
        ["git"] => git::matching_subcommands(prefix)
            .into_iter()
            .map(String::from)
            .collect(),
        ["git", sub] if in_git_repo && git::BRANCH_TAKING_SUBCOMMANDS.contains(sub) => branches
            .iter()
            // The branch you're already on isn't a useful checkout/switch target.
            .filter(|b| b.starts_with(prefix) && Some(b.as_str()) != current_branch)
            .cloned()
            .collect(),
        ["git", sub] if in_git_repo && git::REF_TAKING_SUBCOMMANDS.contains(sub) => branches
            .iter()
            .filter(|b| b.starts_with(prefix))
            .cloned()
            .collect(),
        ["git", sub] if git::PATH_TAKING_SUBCOMMANDS.contains(sub) => {
            fs_path_candidates(cwd, prefix, false)
        }
        ["git", sub] if in_git_repo && git::REMOTE_TAKING_SUBCOMMANDS.contains(sub) => remotes
            .iter()
            .filter(|r| r.starts_with(prefix))
            .cloned()
            .collect(),
        ["git", sub, _remote] if in_git_repo && git::REMOTE_TAKING_SUBCOMMANDS.contains(sub) => {
            branches
                .iter()
                .filter(|b| b.starts_with(prefix))
                .cloned()
                .collect()
        }
        ["gh"] => gh::matching_commands(prefix)
            .into_iter()
            .map(String::from)
            .collect(),
        ["gh", top] if gh::COMMANDS.iter().any(|(n, _)| n == top) => {
            gh::matching_subcommands(top, prefix)
                .into_iter()
                .map(String::from)
                .collect()
        }
        [tool] if is_pkg_tool(tool) => {
            let mut out: Vec<String> = package_scripts
                .iter()
                .filter(|s| s.starts_with(prefix))
                .cloned()
                .collect();
            out.extend(
                pkgmgr::matching_subcommands(tool, prefix)
                    .into_iter()
                    .map(String::from),
            );
            out
        }
        [tool, "run"] if is_pkg_tool(tool) => package_scripts
            .iter()
            .filter(|s| s.starts_with(prefix))
            .cloned()
            .collect(),
        ["cd"] => fs_path_candidates(cwd, prefix, true),
        _ => Vec::new(),
    }
}

/// Pure priority logic for the single dimmed inline hint: history always
/// wins when it has a match; git/gh/pnpm fallback only kicks in on a
/// history miss. Factored out of `EfaHinter::handle` so it's testable
/// without a real `Db` or filesystem.
#[allow(clippy::too_many_arguments)]
pub fn resolve_inline_hint(
    line: &str,
    history_match: Option<&str>,
    in_git_repo: bool,
    branches: &[String],
    current_branch: Option<&str>,
    remotes: &[String],
    package_scripts: &[String],
    cwd: &Path,
) -> Option<String> {
    if let Some(command) = history_match {
        let rest = command.strip_prefix(line)?;
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }

    let (toks, prefix) = split_current(line);
    let mut candidates = fallback_last_token_candidates(
        &toks,
        prefix,
        in_git_repo,
        branches,
        current_branch,
        remotes,
        package_scripts,
        cwd,
    );
    candidates.sort();
    candidates.into_iter().next().map(|candidate| {
        // `candidate` is the completed final token; `prefix` is what the
        // user already typed of it, so the hint is just the remainder.
        candidate[prefix.len()..].to_string()
    })
}

/// Fish-style inline autosuggestion hinter, backed by EFA's own SQLite
/// history rather than reedline's built-in `History` (which only knows
/// simple last-match-by-prefix, not our per-directory ranking).
///
/// History is the top-priority signal; when it has no match for the
/// current line, `resolve_inline_hint` falls through to git/gh/package-
/// manager knowledge (see that function's doc comment for the exact
/// priority chain).
pub struct EfaHinter {
    db: Db,
    project: Arc<ProjectDetector>,
    branch_cache: Arc<git::BranchCache>,
    style: Style,
    current_hint: String,
    min_chars: usize,
}

impl EfaHinter {
    pub fn new(db: Db, project: Arc<ProjectDetector>, branch_cache: Arc<git::BranchCache>) -> Self {
        EfaHinter {
            db,
            project,
            branch_cache,
            style: Style::new().fg(Color::DarkGray),
            current_hint: String::new(),
            min_chars: 1,
        }
    }
}

impl Hinter for EfaHinter {
    fn handle(
        &mut self,
        line: &str,
        _pos: usize,
        _history: &dyn ReedlineHistory,
        use_ansi_coloring: bool,
        cwd: &str,
    ) -> String {
        self.current_hint = if line.chars().count() >= self.min_chars {
            let history_match = self.db.best_suggestion(cwd, line).ok().flatten();

            let roots = self.project.detect(&PathBuf::from(cwd));
            let in_git_repo = roots.git_root.is_some();
            let branches = roots
                .git_root
                .as_deref()
                .map(|root| self.branch_cache.branches(root))
                .unwrap_or_default();
            let current_branch = roots.git_root.as_deref().and_then(git::current_branch);
            let remotes = roots
                .git_root
                .as_deref()
                .map(git::remotes)
                .unwrap_or_default();
            let package_scripts = roots
                .package_root
                .as_deref()
                .map(|root| pkgmgr::read_scripts(&root.join("package.json")))
                .unwrap_or_default();

            resolve_inline_hint(
                line,
                history_match.as_ref().map(|s| s.command.as_str()),
                in_git_repo,
                &branches,
                current_branch.as_deref(),
                &remotes,
                &package_scripts,
                &PathBuf::from(cwd),
            )
            .unwrap_or_default()
        } else {
            String::new()
        };

        if use_ansi_coloring && !self.current_hint.is_empty() {
            self.style.paint(&self.current_hint).to_string()
        } else {
            self.current_hint.clone()
        }
    }

    fn complete_hint(&self) -> String {
        self.current_hint.clone()
    }

    fn next_hint_token(&self) -> String {
        self.current_hint
            .split_inclusive(char::is_whitespace)
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

/// Fish-style Tab-cycled completion menu. Unlike `EfaHinter`, this
/// aggregates *all* matching candidates - history, git subcommands/branches,
/// gh commands, and pnpm/npm/yarn subcommands + scripts - deduplicated, so
/// Tab still surfaces real alternatives even when history already won the
/// inline hint.
pub struct EfaCompleter {
    db: Db,
    project: Arc<ProjectDetector>,
    branch_cache: Arc<git::BranchCache>,
}

impl EfaCompleter {
    pub fn new(db: Db, project: Arc<ProjectDetector>, branch_cache: Arc<git::BranchCache>) -> Self {
        EfaCompleter {
            db,
            project,
            branch_cache,
        }
    }
}

impl Completer for EfaCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let line_upto = &line[..pos];
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

        let roots = self.project.detect(&cwd);
        let in_git_repo = roots.git_root.is_some();
        let branches = roots
            .git_root
            .as_deref()
            .map(|root| self.branch_cache.branches(root))
            .unwrap_or_default();
        let current_branch = roots.git_root.as_deref().and_then(git::current_branch);
        let remotes = roots
            .git_root
            .as_deref()
            .map(git::remotes)
            .unwrap_or_default();
        let package_scripts = roots
            .package_root
            .as_deref()
            .map(|root| pkgmgr::read_scripts(&root.join("package.json")))
            .unwrap_or_default();

        let mut suggestions: Vec<ReedlineSuggestion> = Vec::new();

        if !line_upto.is_empty() {
            let cwd_str = cwd.display().to_string();
            if let Ok(history_hits) = self.db.top_suggestions(&cwd_str, line_upto, 10) {
                for hit in history_hits {
                    suggestions.push(ReedlineSuggestion {
                        value: hit.command,
                        span: Span::new(0, pos),
                        append_whitespace: true,
                        ..Default::default()
                    });
                }
            }
        }

        let (toks, prefix) = split_current(line_upto);
        let prefix_start = line_upto.len() - prefix.len();
        let mut fallback_candidates = fallback_last_token_candidates(
            &toks,
            prefix,
            in_git_repo,
            &branches,
            current_branch.as_deref(),
            &remotes,
            &package_scripts,
            &cwd,
        );
        fallback_candidates.sort();
        fallback_candidates.dedup();
        for candidate in fallback_candidates {
            // Directory candidates end in `/`: leave the cursor right after
            // it so the user can keep drilling into the path, rather than
            // inserting a trailing space.
            let append_whitespace = !candidate.ends_with('/');
            suggestions.push(ReedlineSuggestion {
                value: candidate,
                span: Span::new(prefix_start, pos),
                append_whitespace,
                ..Default::default()
            });
        }

        suggestions.dedup_by(|a, b| a.value == b.value && a.span == b.span);
        CompletionResult::fresh(suggestions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn hint(
        line: &str,
        history_match: Option<&str>,
        in_git_repo: bool,
        branches: &[String],
        current_branch: Option<&str>,
        remotes: &[String],
        package_scripts: &[String],
        cwd: &Path,
    ) -> Option<String> {
        resolve_inline_hint(
            line,
            history_match,
            in_git_repo,
            branches,
            current_branch,
            remotes,
            package_scripts,
            cwd,
        )
    }

    fn nowhere() -> &'static Path {
        Path::new("/nonexistent")
    }

    #[test]
    fn history_wins_over_git_fallback() {
        let branches = vec!["main".to_string()];
        let h = hint(
            "git ch",
            Some("git checkout-history"),
            true,
            &branches,
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("eckout-history".to_string()));
    }

    #[test]
    fn git_fallback_activates_only_on_history_miss() {
        let h = hint("git chec", None, true, &[], None, &[], &[], nowhere());
        assert_eq!(h, Some("kout".to_string()));
    }

    #[test]
    fn git_branch_fallback_requires_git_repo() {
        let branches = vec!["main".to_string(), "feature/foo".to_string()];
        let h = hint(
            "git checkout ma",
            None,
            false,
            &branches,
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, None);

        let h = hint(
            "git checkout ma",
            None,
            true,
            &branches,
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("in".to_string()));
    }

    #[test]
    fn git_ref_taking_subcommands_include_current_branch() {
        let branches = vec!["main".to_string(), "master".to_string()];
        // Unlike checkout/switch, `log`/`diff`/`show` don't exclude the
        // branch you're already on.
        let h = hint(
            "git log ma",
            None,
            true,
            &branches,
            Some("main"),
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("in".to_string()));
    }

    #[test]
    fn git_ref_taking_subcommands_require_git_repo() {
        let branches = vec!["main".to_string()];
        let h = hint(
            "git show ma",
            None,
            false,
            &branches,
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, None);
    }

    #[test]
    fn git_remote_taking_subcommands_offer_remote_first() {
        let remotes = vec!["origin".to_string(), "upstream".to_string()];
        let h = hint(
            "git push or",
            None,
            true,
            &[],
            None,
            &remotes,
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("igin".to_string()));
    }

    #[test]
    fn git_remote_taking_subcommands_offer_branch_after_remote() {
        let branches = vec!["main".to_string(), "feature/foo".to_string()];
        let h = hint(
            "git push origin ma",
            None,
            true,
            &branches,
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("in".to_string()));
    }

    #[test]
    fn git_path_taking_subcommands_work_outside_git_repo_flag() {
        // `git add`/`restore`/`rm`/`mv` complete filesystem paths; this
        // doesn't depend on `in_git_repo` since it's plain fs listing.
        let root = tempdir();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("staged.rs"), "").unwrap();

        assert_eq!(
            hint("git add sta", None, false, &[], None, &[], &[], &root),
            Some("ged.rs".to_string())
        );
        assert_eq!(
            hint("git rm sr", None, true, &[], None, &[], &[], &root),
            Some("c/".to_string())
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gh_fallback() {
        let h = hint("gh p", None, false, &[], None, &[], &[], nowhere());
        assert_eq!(h, Some("r".to_string()));

        let h = hint("gh pr cre", None, false, &[], None, &[], &[], nowhere());
        assert_eq!(h, Some("ate".to_string()));
    }

    #[test]
    fn pkgmgr_scripts_win_over_subcommands() {
        let scripts = vec!["devserver".to_string()];
        // "dev" only matches the script, not any static pnpm subcommand.
        let h = hint("pnpm dev", None, false, &[], None, &[], &scripts, nowhere());
        assert_eq!(h, Some("server".to_string()));
    }

    #[test]
    fn pkgmgr_run_form_only_offers_scripts() {
        let scripts = vec!["build".to_string()];
        let h = hint(
            "pnpm run bui",
            None,
            false,
            &[],
            None,
            &[],
            &scripts,
            nowhere(),
        );
        assert_eq!(h, Some("ld".to_string()));
    }

    #[test]
    fn current_branch_excluded_from_checkout_candidates() {
        let branches = vec!["main".to_string(), "master".to_string()];
        let h = hint(
            "git checkout ma",
            None,
            true,
            &branches,
            Some("main"),
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, Some("ster".to_string()));
    }

    #[test]
    fn no_fallback_when_nothing_matches() {
        let h = hint(
            "echo hel",
            None,
            true,
            &["main".to_string()],
            None,
            &[],
            &[],
            nowhere(),
        );
        assert_eq!(h, None);
    }

    #[test]
    fn split_current_trailing_whitespace_means_empty_prefix() {
        assert_eq!(split_current("git "), (vec!["git"], ""));
        assert_eq!(split_current("git chec"), (vec!["git"], "chec"));
    }

    fn tempdir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "efa-completion-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        path.push(unique);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn cd_completes_directory_names() {
        let root = tempdir();
        std::fs::create_dir_all(root.join("vaeproject")).unwrap();
        std::fs::create_dir_all(root.join("vaeother")).unwrap();
        std::fs::write(root.join("vaefile.txt"), "").unwrap();

        let h = hint("cd vae", None, false, &[], None, &[], &[], &root);
        // Both "vaeproject/" and "vaeother/" match; alphabetical sort picks
        // "vaeother/" first, and files are never offered for `cd`.
        assert_eq!(h, Some("other/".to_string()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cd_completes_nested_path() {
        let root = tempdir();
        std::fs::create_dir_all(root.join("sub/deep")).unwrap();

        let h = hint("cd sub/de", None, false, &[], None, &[], &[], &root);
        assert_eq!(h, Some("ep/".to_string()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cd_hides_dotfiles_unless_typed() {
        let root = tempdir();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(root.join("visible")).unwrap();

        assert_eq!(
            hint("cd ", None, false, &[], None, &[], &[], &root),
            Some("visible/".to_string())
        );
        assert_eq!(
            hint("cd .", None, false, &[], None, &[], &[], &root),
            Some("hidden/".to_string())
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn cd_no_match_returns_none() {
        let root = tempdir();
        let h = hint("cd zzz", None, false, &[], None, &[], &[], &root);
        assert_eq!(h, None);
        std::fs::remove_dir_all(&root).ok();
    }
}
