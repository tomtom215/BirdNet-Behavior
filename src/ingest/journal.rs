//! The import journal: what has been staged, so a rerun stages nothing twice.
//!
//! One JSON object per line in `journal.jsonl`, appended and synced as each
//! piece is staged and as each recording finishes. A recording is identified
//! by its content (SHA-256), the source it is imported as and its start
//! ([`key`]), not by its path: the same card copied to another folder is the
//! same recording, and the same file imported as another source or at another
//! start is a different import.
//!
//! A run stopped part-way (Ctrl-C, a power cut, the station not consuming)
//! has its pieces recorded one by one, so the rerun splits the recording again
//! and stages only the pieces the journal does not have. The split is
//! deterministic, so piece `n` is the same audio both times.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The journal's file name in the ingest directory.
pub const JOURNAL_FILE: &str = "journal.jsonl";

/// A recording's identity in the journal.
#[must_use]
pub fn key(sha256: &str, source: &str, start_unix_ms: i64) -> String {
    format!("{sha256}:{source}:{start_unix_ms}")
}

/// The journal, read once and appended to.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    done: HashSet<String>,
    staged: HashMap<String, HashSet<u64>>,
    /// Lines that did not parse: a torn last line from a stop mid-write, or
    /// damage. Skipped, and reported.
    pub unreadable_lines: usize,
}

impl Journal {
    /// Open (creating) the journal in `dir`.
    ///
    /// # Errors
    ///
    /// The directory or the file cannot be created or read.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(JOURNAL_FILE);
        let mut journal = Self {
            path,
            done: HashSet::new(),
            staged: HashMap::new(),
            unreadable_lines: 0,
        };
        let text = match std::fs::read_to_string(&journal.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                journal.unreadable_lines += 1;
                continue;
            };
            let Some(k) = v.get("key").and_then(Value::as_str) else {
                journal.unreadable_lines += 1;
                continue;
            };
            match (
                v.get("kind").and_then(Value::as_str),
                v.get("index").and_then(Value::as_u64),
            ) {
                (Some("done"), _) => {
                    journal.done.insert(k.to_owned());
                }
                (Some("piece"), Some(i)) => {
                    journal.staged.entry(k.to_owned()).or_default().insert(i);
                }
                _ => journal.unreadable_lines += 1,
            }
        }
        Ok(journal)
    }

    /// Whether the recording `key` names was imported whole.
    #[must_use]
    pub fn is_done(&self, key: &str) -> bool {
        self.done.contains(key)
    }

    /// Whether piece `index` of `key` was staged.
    #[must_use]
    pub fn is_staged(&self, key: &str, index: u64) -> bool {
        self.staged.get(key).is_some_and(|s| s.contains(&index))
    }

    fn append(&self, line: &Value) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut text = line.to_string();
        text.push('\n');
        f.write_all(text.as_bytes())?;
        f.sync_data()
    }

    /// Record piece `index` of `key` as staged; `detail` is kept beside it.
    ///
    /// # Errors
    ///
    /// The line cannot be written and synced.
    pub fn record_piece(&mut self, key: &str, index: u64, detail: Value) -> std::io::Result<()> {
        let mut line = serde_json::json!({ "kind": "piece", "key": key, "index": index });
        merge(&mut line, detail);
        self.append(&line)?;
        self.staged.entry(key.to_owned()).or_default().insert(index);
        Ok(())
    }

    /// Record `key` as imported whole; `detail` is kept beside it.
    ///
    /// # Errors
    ///
    /// As [`Self::record_piece`].
    pub fn record_done(&mut self, key: &str, detail: Value) -> std::io::Result<()> {
        let mut line = serde_json::json!({ "kind": "done", "key": key });
        merge(&mut line, detail);
        self.append(&line)?;
        self.done.insert(key.to_owned());
        Ok(())
    }
}

/// Add `extra`'s fields to `line`, which keeps its own where both have one.
fn merge(line: &mut Value, extra: Value) {
    if let (Some(l), Value::Object(e)) = (line.as_object_mut(), extra) {
        for (k, v) in e {
            l.entry(k).or_insert(v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_recorded_is_known_to_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let k = key("ab", "card1", 1_779_165_000_000);
        {
            let mut j = Journal::open(dir.path()).unwrap();
            assert!(!j.is_done(&k));
            j.record_piece(&k, 0, serde_json::json!({ "name": "a.wav" }))
                .unwrap();
            j.record_piece(&k, 1, serde_json::json!({ "key": "not this" }))
                .unwrap();
        }
        let mut j = Journal::open(dir.path()).unwrap();
        assert!(j.is_staged(&k, 0) && j.is_staged(&k, 1) && !j.is_staged(&k, 2));
        assert!(!j.is_done(&k));
        j.record_done(&k, serde_json::json!({})).unwrap();
        assert!(Journal::open(dir.path()).unwrap().is_done(&k));
        // Another source, or another start, is another import.
        let j = Journal::open(dir.path()).unwrap();
        assert!(!j.is_done(&key("ab", "card2", 1_779_165_000_000)));
        assert!(!j.is_done(&key("ab", "card1", 1_779_165_001_000)));
    }

    /// A stop in the middle of a write leaves a torn last line: it is skipped
    /// and counted, and what came before it stands.
    #[test]
    fn a_torn_last_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let k = key("ab", "s", 0);
        Journal::open(dir.path())
            .unwrap()
            .record_piece(&k, 0, Value::Null)
            .unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.path().join(JOURNAL_FILE))
            .unwrap();
        f.write_all(b"{\"kind\":\"piece\",\"key\":\"ab:s:0\",\"ind")
            .unwrap();
        let j = Journal::open(dir.path()).unwrap();
        assert!(j.is_staged(&k, 0));
        assert_eq!(j.unreadable_lines, 1);
    }
}
