use nu_ansi_term::{Color, Style};
use reedline::{Hinter, History as ReedlineHistory};

use crate::db::Db;

/// Fish-style inline autosuggestion hinter, backed by EFA's own SQLite
/// history rather than reedline's built-in `History` (which only knows
/// simple last-match-by-prefix, not our per-directory ranking).
///
/// v0.1 only considers the exact current working directory; project-root
/// awareness is intentionally left for a later version (see `db.rs`).
pub struct EfaHinter {
    db: Db,
    style: Style,
    current_hint: String,
    min_chars: usize,
}

impl EfaHinter {
    pub fn new(db: Db) -> Self {
        EfaHinter {
            db,
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
            self.db
                .best_suggestion(cwd, line)
                .ok()
                .flatten()
                .and_then(|s| {
                    s.command
                        .strip_prefix(line)
                        .filter(|rest| !rest.is_empty())
                        .map(str::to_string)
                })
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
