use std::path::Path;

pub const PNPM_SUBCOMMANDS: &[&str] = &[
    "add", "audit", "build", "config", "dedupe", "deploy", "dlx", "exec", "fetch", "import",
    "init", "install", "licenses", "link", "list", "outdated", "pack", "patch", "prune", "publish",
    "rebuild", "remove", "run", "start", "store", "test", "unlink", "update", "why",
];

pub const NPM_SUBCOMMANDS: &[&str] = &[
    "access",
    "audit",
    "bugs",
    "cache",
    "ci",
    "config",
    "dedupe",
    "diff",
    "doctor",
    "exec",
    "explain",
    "explore",
    "fund",
    "init",
    "install",
    "link",
    "list",
    "login",
    "logout",
    "ls",
    "outdated",
    "owner",
    "pack",
    "ping",
    "prefix",
    "profile",
    "prune",
    "publish",
    "rebuild",
    "repo",
    "restart",
    "root",
    "run",
    "search",
    "start",
    "stop",
    "team",
    "test",
    "token",
    "uninstall",
    "unpublish",
    "unstar",
    "update",
    "version",
    "view",
    "whoami",
];

pub const YARN_SUBCOMMANDS: &[&str] = &[
    "add",
    "audit",
    "autoclean",
    "bin",
    "cache",
    "check",
    "config",
    "create",
    "dedupe",
    "exec",
    "generate-lock-entry",
    "global",
    "import",
    "info",
    "init",
    "install",
    "licenses",
    "link",
    "list",
    "login",
    "logout",
    "node",
    "outdated",
    "owner",
    "pack",
    "publish",
    "remove",
    "run",
    "tag",
    "team",
    "test",
    "unlink",
    "unplug",
    "upgrade",
    "upgrade-interactive",
    "version",
    "why",
    "workspace",
    "workspaces",
];

pub fn subcommands_for(tool: &str) -> &'static [&'static str] {
    match tool {
        "pnpm" => PNPM_SUBCOMMANDS,
        "npm" => NPM_SUBCOMMANDS,
        "yarn" => YARN_SUBCOMMANDS,
        _ => &[],
    }
}

pub fn matching_subcommands(tool: &str, prefix: &str) -> Vec<&'static str> {
    subcommands_for(tool)
        .iter()
        .copied()
        .filter(|c| c.starts_with(prefix))
        .collect()
}

/// Read the `"scripts"` object's keys from a `package.json` file. Any
/// failure (missing file, malformed JSON, `scripts` missing/wrong type)
/// degrades silently to an empty list - never panics, never blocks typing.
pub fn read_scripts(package_json_path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(package_json_path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    value
        .get("scripts")
        .and_then(|s| s.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempfile(contents: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "efa-pkgmgr-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        path.push(unique);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn well_formed_scripts() {
        let path = tempfile(r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#);
        let mut scripts = read_scripts(&path);
        scripts.sort();
        assert_eq!(scripts, vec!["build".to_string(), "dev".to_string()]);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_returns_empty() {
        let path = std::env::temp_dir().join("efa-pkgmgr-does-not-exist.json");
        assert_eq!(read_scripts(&path), Vec::<String>::new());
    }

    #[test]
    fn malformed_json_returns_empty() {
        let path = tempfile("{ not json");
        assert_eq!(read_scripts(&path), Vec::<String>::new());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn wrong_type_scripts_returns_empty() {
        let path = tempfile(r#"{"scripts": "not an object"}"#);
        assert_eq!(read_scripts(&path), Vec::<String>::new());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_scripts_key_returns_empty() {
        let path = tempfile(r#"{"name": "foo"}"#);
        assert_eq!(read_scripts(&path), Vec::<String>::new());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn subcommand_prefix_matching() {
        assert!(matching_subcommands("pnpm", "ins").contains(&"install"));
        assert!(matching_subcommands("npm", "zzz").is_empty());
        assert!(subcommands_for("unknown").is_empty());
    }
}
