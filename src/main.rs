mod completion;
mod db;
mod gh;
mod git;
mod history;
mod pkgmgr;
mod project;
mod shell;

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::Result;
use reedline::{
    ColumnarMenu, Emacs, FileBackedHistory, KeyCode, KeyModifiers, MenuBuilder, Prompt,
    PromptEditMode, PromptHistorySearch, Reedline, ReedlineEvent, ReedlineMenu, Signal,
    default_emacs_keybindings,
};

use completion::{EfaCompleter, EfaHinter};
use db::Db;
use git::BranchCache;
use history::History;
use project::ProjectDetector;
use shell::{CommandOutcome, Shell};

/// Minimal prompt: `<cwd> ❯ `, updated every iteration to track `cd`.
struct EfaPrompt {
    cwd: String,
}

impl Prompt for EfaPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(format!("{} ", self.cwd))
    }
    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }
    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("❯ ")
    }
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("::: ")
    }
    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        Cow::Borrowed("(search) ")
    }
}

extern "C" fn sigint_noop(_: libc::c_int) {}

/// Install a real (no-op) SIGINT handler so Ctrl+C never terminates EFA
/// itself, whether it arrives while a child command is attached to the
/// terminal (reedline intercepts Ctrl+C as a keypress while editing, so no
/// real signal reaches us then) or otherwise.
///
/// This deliberately installs a handler rather than `SIG_IGN`: POSIX resets
/// a *caught* signal's disposition to default across `exec`, but an
/// *ignored* one is inherited as-is. `SIG_IGN` here would silently make
/// every child process ignore Ctrl+C too (e.g. `sleep 100` would no longer
/// stop), whereas a handler lets `exec` restore normal Ctrl+C behavior for
/// children automatically.
fn install_sigint_handler() {
    unsafe {
        libc::signal(libc::SIGINT, sigint_noop as *const () as libc::sighandler_t);
    }
}

fn history_file_path() -> Result<std::path::PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let dir = home.join(".efa");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("line_history.txt"))
}

fn main() -> Result<()> {
    install_sigint_handler();

    let db = Db::open_default()?;
    let history = History::new(db.clone());
    let project = Arc::new(ProjectDetector::new());
    let branch_cache = Arc::new(BranchCache::new());

    let line_history = Box::new(FileBackedHistory::with_file(1000, history_file_path()?)?);
    let hinter = Box::new(EfaHinter::new(
        db.clone(),
        project.clone(),
        branch_cache.clone(),
    ));
    let completer = Box::new(EfaCompleter::new(db, project.clone(), branch_cache));
    let completion_menu = ReedlineMenu::EngineCompleter(Box::new(
        ColumnarMenu::default().with_name("completion_menu"),
    ));

    let mut keybindings = default_emacs_keybindings();
    // First Tab press either accepts the inline hint (if one exists) or
    // opens the completion menu; once the menu is open, further Tab presses
    // cycle forward and Shift+Tab cycles backward, matching fish's
    // Tab/Shift+Tab pager navigation.
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::HistoryHintComplete,
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::MenuPrevious,
    );
    let edit_mode = Box::new(Emacs::new(keybindings));

    let mut line_editor = Reedline::create()
        .with_hinter(hinter)
        .with_completer(completer)
        .with_menu(completion_menu)
        .with_history(line_history)
        .with_edit_mode(edit_mode);

    let mut shell = Shell::new();

    loop {
        let prompt = EfaPrompt {
            cwd: shell.display_cwd(),
        };

        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed == "exit" {
                    break;
                }

                let cwd = shell.cwd().display().to_string();
                let outcome = shell.run(&line);
                let exit_code = match &outcome {
                    CommandOutcome::ChangedDirectory => Some(0),
                    CommandOutcome::Exited(code) => *code,
                    CommandOutcome::SpawnFailed(err) => {
                        eprintln!("efa: {}", err);
                        None
                    }
                };
                let project_root = project
                    .detect(shell.cwd())
                    .primary()
                    .map(|p| p.display().to_string());
                if let Err(e) = history.record(trimmed, &cwd, project_root.as_deref(), exit_code) {
                    eprintln!("efa: failed to record history: {}", e);
                }
            }
            Ok(Signal::CtrlC) => {
                // Cancel current input; keep the shell running.
                continue;
            }
            Ok(Signal::CtrlD) => {
                break;
            }
            Ok(_) => {
                // Other signal variants (e.g. host-command payloads) are
                // not produced by this configuration; ignore defensively.
                continue;
            }
            Err(e) => {
                eprintln!("efa: input error: {}", e);
                break;
            }
        }
    }

    Ok(())
}
