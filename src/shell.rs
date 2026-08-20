use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Tracks EFA's current working directory and knows how to run commands
/// against it.
///
/// `cd` is handled here rather than by the child shell: spawning `cd` in a
/// subprocess only changes that subprocess's directory, not ours.
pub struct Shell {
    cwd: PathBuf,
    shell_bin: String,
}

pub enum CommandOutcome {
    /// Built-in `cd` was handled; no child process was spawned.
    ChangedDirectory,
    /// A child process ran and exited with this status (`None` if it was
    /// terminated by a signal rather than exiting normally).
    Exited(Option<i32>),
    /// The command could not be spawned at all (e.g. shell binary missing).
    SpawnFailed(String),
}

impl Shell {
    pub fn new() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let shell_bin = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        Shell { cwd, shell_bin }
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Render the current directory for the prompt, using `~` for home.
    pub fn display_cwd(&self) -> String {
        if let Some(home) = dirs::home_dir()
            && let Ok(rest) = self.cwd.strip_prefix(&home)
        {
            return if rest.as_os_str().is_empty() {
                "~".to_string()
            } else {
                format!("~/{}", rest.display())
            };
        }
        self.cwd.display().to_string()
    }

    /// Run a line of input: `cd` is intercepted, everything else is
    /// delegated to the user's normal shell via `$SHELL -c` so that pipes,
    /// redirects, quoting, and environment expansion all behave normally.
    pub fn run(&mut self, line: &str) -> CommandOutcome {
        let trimmed = line.trim();
        if let Some(rest) = strip_cd(trimmed) {
            return self.builtin_cd(rest);
        }

        // EFA installs a no-op SIGINT handler (see `main::install_sigint_handler`)
        // rather than SIG_IGN specifically so that `exec` resets it to the
        // default disposition here, letting the child receive and react to
        // Ctrl+C normally while EFA itself stays alive.
        let result = Command::new(&self.shell_bin)
            .arg("-c")
            .arg(line)
            .current_dir(&self.cwd)
            .status();

        match result {
            Ok(status) => CommandOutcome::Exited(status.code()),
            Err(e) => CommandOutcome::SpawnFailed(e.to_string()),
        }
    }

    fn builtin_cd(&mut self, arg: &str) -> CommandOutcome {
        let target = self.resolve_cd_target(arg);
        match target {
            Some(path) => match path.canonicalize() {
                Ok(canon) => {
                    // Keep the real process cwd in sync with our tracked one:
                    // reedline's hinter (and EfaCompleter) fall back to
                    // `std::env::current_dir()` whenever no explicit cwd is
                    // configured, so without this, suggestions would keep
                    // looking at the directory efa was launched from instead
                    // of wherever the user has since `cd`-ed to.
                    if let Err(e) = env::set_current_dir(&canon) {
                        eprintln!("cd: {}: {}", canon.display(), e);
                    }
                    self.cwd = canon;
                    CommandOutcome::ChangedDirectory
                }
                Err(e) => {
                    eprintln!("cd: {}: {}", path.display(), e);
                    CommandOutcome::ChangedDirectory
                }
            },
            None => {
                eprintln!("cd: could not determine home directory");
                CommandOutcome::ChangedDirectory
            }
        }
    }

    fn resolve_cd_target(&self, arg: &str) -> Option<PathBuf> {
        let arg = arg.trim();
        if arg.is_empty() || arg == "~" {
            return dirs::home_dir();
        }
        let path = if let Some(rest) = arg.strip_prefix("~/") {
            dirs::home_dir()?.join(rest)
        } else {
            Path::new(arg).to_path_buf()
        };
        Some(if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        })
    }
}

/// Recognize `cd`, `cd <arg>`, and bare-word `cd` with trailing whitespace,
/// without pulling in a full shell parser. Anything more complex (e.g. `cd
/// foo && ls`) is intentionally left to fall through to the system shell.
fn strip_cd(trimmed: &str) -> Option<&str> {
    if trimmed == "cd" {
        return Some("");
    }
    trimmed.strip_prefix("cd ").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_at(cwd: &str) -> Shell {
        Shell {
            cwd: PathBuf::from(cwd),
            shell_bin: "/bin/sh".to_string(),
        }
    }

    #[test]
    fn cd_bare_goes_home() {
        let shell = shell_at("/tmp");
        assert_eq!(shell.resolve_cd_target(""), dirs::home_dir());
    }

    #[test]
    fn cd_tilde_goes_home() {
        let shell = shell_at("/tmp");
        assert_eq!(shell.resolve_cd_target("~"), dirs::home_dir());
    }

    #[test]
    fn cd_relative_joins_cwd() {
        let shell = shell_at("/tmp/foo");
        assert_eq!(
            shell.resolve_cd_target("bar"),
            Some(PathBuf::from("/tmp/foo/bar"))
        );
    }

    #[test]
    fn cd_absolute_is_used_directly() {
        let shell = shell_at("/tmp/foo");
        assert_eq!(shell.resolve_cd_target("/etc"), Some(PathBuf::from("/etc")));
    }

    #[test]
    fn cd_dotdot_joins_cwd() {
        let shell = shell_at("/tmp/foo");
        assert_eq!(
            shell.resolve_cd_target(".."),
            Some(PathBuf::from("/tmp/foo/.."))
        );
    }

    #[test]
    fn strip_cd_recognizes_bare_and_argument_forms() {
        assert_eq!(strip_cd("cd"), Some(""));
        assert_eq!(strip_cd("cd ~"), Some("~"));
        assert_eq!(strip_cd("cd  ./foo"), Some("./foo"));
        assert_eq!(strip_cd("cdish"), None);
        assert_eq!(strip_cd("echo cd"), None);
    }

    #[test]
    fn display_cwd_uses_tilde_for_home() {
        if let Some(home) = dirs::home_dir() {
            let shell = shell_at(home.join("projects/payslick").to_str().unwrap());
            assert_eq!(shell.display_cwd(), "~/projects/payslick");
        }
    }
}
