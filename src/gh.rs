/// Static table of `gh` top-level commands and their common subcommands.
/// No network calls, no `gh` shell-outs, no auth handling - purely a
/// hardcoded reference table.
pub const COMMANDS: &[(&str, &[&str])] = &[
    ("alias", &["delete", "import", "list", "set"]),
    ("api", &[]),
    (
        "auth",
        &[
            "login",
            "logout",
            "refresh",
            "setup-git",
            "status",
            "switch",
            "token",
        ],
    ),
    ("browse", &[]),
    ("cache", &["delete", "list"]),
    (
        "codespace",
        &[
            "code", "create", "delete", "list", "logs", "ssh", "stop", "view",
        ],
    ),
    (
        "gist",
        &[
            "clone", "create", "delete", "edit", "list", "rename", "view",
        ],
    ),
    (
        "issue",
        &[
            "close", "comment", "create", "delete", "edit", "list", "lock", "pin", "reopen",
            "status", "transfer", "unlock", "unpin", "view",
        ],
    ),
    ("label", &["clone", "create", "delete", "edit", "list"]),
    ("org", &["list"]),
    (
        "pr",
        &[
            "checkout", "checks", "close", "comment", "create", "diff", "edit", "list", "lock",
            "merge", "ready", "reopen", "review", "status", "unlock", "view",
        ],
    ),
    (
        "project",
        &["close", "copy", "create", "delete", "edit", "list", "view"],
    ),
    (
        "release",
        &[
            "create", "delete", "download", "edit", "list", "upload", "view",
        ],
    ),
    (
        "repo",
        &[
            "archive", "clone", "create", "delete", "edit", "fork", "list", "rename", "sync",
            "view",
        ],
    ),
    ("run", &["cancel", "list", "rerun", "view", "watch"]),
    ("secret", &["delete", "list", "set"]),
    ("ssh-key", &["add", "delete", "list"]),
    ("status", &[]),
    ("workflow", &["disable", "enable", "list", "run", "view"]),
];

pub fn matching_commands(prefix: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .map(|(name, _)| *name)
        .filter(|c| c.starts_with(prefix))
        .collect()
}

pub fn matching_subcommands(command: &str, prefix: &str) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, subs)| {
            subs.iter()
                .copied()
                .filter(|s| s.starts_with(prefix))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_prefix_matching() {
        assert!(matching_commands("pr").contains(&"pr"));
        assert!(matching_commands("zzz").is_empty());
    }

    #[test]
    fn subcommand_prefix_matching() {
        assert!(matching_subcommands("pr", "cre").contains(&"create"));
        assert!(matching_subcommands("unknown-command", "cre").is_empty());
    }
}
