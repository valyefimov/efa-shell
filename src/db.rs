use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;

/// A ranked command suggestion for a given directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub command: String,
    pub usage_count: i64,
    pub last_used: i64,
}

/// Thread-safe handle to the EFA SQLite database.
///
/// `command_history` is the single source of truth for directory-aware
/// suggestions. `cwd` and `project_root` are stored as separate columns
/// (with `project_root` unused in v0.1) so project-root-aware ranking can
/// be added later without a schema migration.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Open (creating if necessary) the database at `~/.efa/efa.db`.
    pub fn open_default() -> Result<Self> {
        let dir = default_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
        Self::open(dir.join("efa.db"))
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        let db = Db {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS command_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command TEXT NOT NULL,
                cwd TEXT NOT NULL,
                project_root TEXT,
                exit_code INTEGER,
                executed_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_command_history_cwd
                ON command_history (cwd);
            CREATE INDEX IF NOT EXISTS idx_command_history_cwd_command
                ON command_history (cwd, command);
            ",
        )?;
        Ok(())
    }

    /// Record an executed command. Empty (whitespace-only) commands are
    /// silently ignored by the caller before this is reached.
    pub fn record(
        &self,
        command: &str,
        cwd: &str,
        exit_code: Option<i32>,
        executed_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "INSERT INTO command_history (command, cwd, exit_code, executed_at) VALUES (?1, ?2, ?3, ?4)",
            (command, cwd, exit_code, executed_at),
        )?;
        Ok(())
    }

    /// Return the single best-ranked suggestion for `prefix` within `cwd`,
    /// or `None` if there is no history match.
    ///
    /// Ranking: usage frequency, then recency (see the module-level SQL).
    /// Restricted to an exact `cwd` match in v0.1; project-root-aware
    /// matching can widen this query later.
    pub fn best_suggestion(&self, cwd: &str, prefix: &str) -> Result<Option<Suggestion>> {
        Ok(self.top_suggestions(cwd, prefix, 1)?.into_iter().next())
    }

    /// Return up to `limit` ranked suggestions for `prefix` within `cwd`.
    pub fn top_suggestions(&self, cwd: &str, prefix: &str, limit: i64) -> Result<Vec<Suggestion>> {
        if prefix.is_empty() {
            return Ok(Vec::new());
        }
        let like_pattern = format!("{}%", escape_like(prefix));
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT command, COUNT(*) AS usage_count, MAX(executed_at) AS last_used
             FROM command_history
             WHERE cwd = ?1 AND command LIKE ?2 ESCAPE '\\'
             GROUP BY command
             ORDER BY usage_count DESC, last_used DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map((cwd, like_pattern, limit), |row| {
            Ok(Suggestion {
                command: row.get(0)?,
                usage_count: row.get(1)?,
                last_used: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

/// Escape `%` and `_` so a user-typed prefix is matched literally by LIKE.
fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn default_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".efa"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        let db = Db {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate().unwrap();
        db
    }

    #[test]
    fn insertion_and_prefix_search() {
        let db = mem_db();
        db.record("pnpm dev", "/proj", Some(0), 1).unwrap();
        let hit = db.best_suggestion("/proj", "pn").unwrap();
        assert_eq!(hit.unwrap().command, "pnpm dev");
    }

    #[test]
    fn directory_isolation() {
        let db = mem_db();
        db.record("pnpm start", "/proj/other", Some(0), 1).unwrap();
        let hit = db.best_suggestion("/proj/payslick", "pn").unwrap();
        assert!(hit.is_none());
    }

    #[test]
    fn frequency_ranking() {
        let db = mem_db();
        db.record("pnpm test", "/proj", Some(0), 1).unwrap();
        db.record("pnpm dev", "/proj", Some(0), 2).unwrap();
        db.record("pnpm dev", "/proj", Some(0), 3).unwrap();
        let hit = db.best_suggestion("/proj", "pn").unwrap().unwrap();
        assert_eq!(hit.command, "pnpm dev");
        assert_eq!(hit.usage_count, 2);
    }

    #[test]
    fn recency_tie_break() {
        let db = mem_db();
        db.record("pnpm dev", "/proj", Some(0), 10).unwrap();
        db.record("pnpm test", "/proj", Some(0), 20).unwrap();
        let hit = db.best_suggestion("/proj", "pn").unwrap().unwrap();
        assert_eq!(hit.command, "pnpm test");
    }

    #[test]
    fn empty_history_returns_none() {
        let db = mem_db();
        assert!(db.best_suggestion("/proj", "pn").unwrap().is_none());
    }

    #[test]
    fn empty_prefix_returns_none() {
        let db = mem_db();
        db.record("pnpm dev", "/proj", Some(0), 1).unwrap();
        assert!(db.best_suggestion("/proj", "").unwrap().is_none());
    }

    #[test]
    fn special_characters_in_commands() {
        let db = mem_db();
        db.record("cat package.json | grep scripts", "/proj", Some(0), 1)
            .unwrap();
        let hit = db.best_suggestion("/proj", "cat pack").unwrap().unwrap();
        assert_eq!(hit.command, "cat package.json | grep scripts");
    }

    #[test]
    fn like_wildcards_in_prefix_are_escaped() {
        let db = mem_db();
        db.record("echo 100%", "/proj", Some(0), 1).unwrap();
        db.record("echo xyz", "/proj", Some(0), 2).unwrap();
        // A literal '%' in the prefix must not act as a wildcard.
        let hit = db.best_suggestion("/proj", "echo 100%").unwrap().unwrap();
        assert_eq!(hit.command, "echo 100%");
    }

    #[test]
    fn paths_with_spaces() {
        let db = mem_db();
        let cwd = "/Users/me/My Projects/payslick";
        db.record("pnpm dev", cwd, Some(0), 1).unwrap();
        let hit = db.best_suggestion(cwd, "pn").unwrap().unwrap();
        assert_eq!(hit.command, "pnpm dev");
    }

    #[test]
    fn top_suggestions_multiple_matches() {
        let db = mem_db();
        db.record("pnpm dev", "/proj", Some(0), 1).unwrap();
        db.record("pnpm dev", "/proj", Some(0), 2).unwrap();
        db.record("pnpm test", "/proj", Some(0), 3).unwrap();
        let hits = db.top_suggestions("/proj", "pn", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].command, "pnpm dev");
    }
}
