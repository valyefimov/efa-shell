use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::db::Db;

/// Records executed commands into the SQLite history.
///
/// Empty (whitespace-only) input is never recorded. Built-in `cd` *is*
/// recorded: it is cheap, and directory-navigation history is likely to be
/// useful once EFA grows navigation shortcuts, so there is no reason to
/// discard it in v0.1.
pub struct History {
    db: Db,
}

impl History {
    pub fn new(db: Db) -> Self {
        History { db }
    }

    pub fn record(
        &self,
        command: &str,
        cwd: &str,
        project_root: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<()> {
        if command.trim().is_empty() {
            return Ok(());
        }
        let executed_at = now_unix();
        self.db
            .record(command, cwd, project_root, exit_code, executed_at)
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
