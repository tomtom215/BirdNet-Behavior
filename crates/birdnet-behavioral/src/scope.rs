//! Which rows an analytics query reads: one source, a confidence floor
//! (UIC-11, MSC-18, N06-02).
//!
//! The query builders read `detections_ts` and `recording_effort` by name.
//! [`RowScope::apply`] rewrites every read of either into a read of the rows
//! the scope admits, so one scope applies to every builder alike, and the
//! effort a rate divides by is the effort of the source whose detections it
//! counts (a source's detections over every source's listening would be
//! neither a per-source nor a site rate).
//!
//! A confidence floor applies to detections only: it is a classifier score,
//! not a probability, and listening has none.

use std::fmt::Write as _;

/// The rows a query reads. The default reads every row, as the builders did
/// before a scope existed.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RowScope {
    /// Only this source's detections and listening.
    pub source: Option<String>,
    /// Only detections at or above this confidence.
    pub min_confidence: Option<f64>,
}

impl RowScope {
    /// Whether the scope admits every row.
    #[must_use]
    pub const fn is_everything(&self) -> bool {
        self.source.is_none() && self.min_confidence.is_none()
    }

    fn detections_where(&self) -> String {
        let mut c = Vec::new();
        if let Some(s) = &self.source {
            c.push(format!("Source = '{}'", s.replace('\'', "''")));
        }
        if let Some(m) = self.min_confidence.filter(|m| m.is_finite()) {
            c.push(format!("Confidence >= {m}"));
        }
        c.join(" AND ")
    }

    /// `sql` with every `FROM`/`JOIN` of `detections_ts` and `recording_effort`
    /// replaced by a read of the rows this scope admits. An alias (`detections_ts
    /// d`) is kept; an unaliased read keeps the table's name.
    #[must_use]
    pub fn apply(&self, sql: &str) -> String {
        if self.is_everything() {
            return sql.to_owned();
        }
        let det = self.detections_where();
        let mut out = rewrite(sql, "detections_ts", &det);
        if let Some(s) = &self.source {
            out = rewrite(
                &out,
                "recording_effort",
                &format!("source = '{}'", s.replace('\'', "''")),
            );
        }
        out
    }
}

/// Rewrite every `FROM table` / `JOIN table` in `sql` to a filtered subquery.
fn rewrite(sql: &str, table: &str, cond: &str) -> String {
    if cond.is_empty() {
        return sql.to_owned();
    }
    let mut out = String::with_capacity(sql.len() + 64);
    let mut rest = sql;
    loop {
        let next = ["FROM ", "JOIN "]
            .iter()
            .filter_map(|kw| {
                rest.match_indices(&format!("{kw}{table}"))
                    .find(|(i, m)| {
                        // A whole word: `recording_effort_hourly` is another table.
                        rest[i + m.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                    })
                    .map(|(i, _)| (i, kw.len()))
            })
            .min();
        let Some((at, kw)) = next else {
            out.push_str(rest);
            return out;
        };
        let after = at + kw + table.len();
        out.push_str(&rest[..at + kw]);
        let tail = &rest[after..];
        // An alias of one or two letters (`d`, `e`): kept as the subquery's.
        let alias = tail.strip_prefix(' ').and_then(|t| {
            let word: String = t.chars().take_while(|c| c.is_alphanumeric()).collect();
            let keyword = [
                "WHERE", "GROUP", "ORDER", "LEFT", "JOIN", "ON", "LIMIT", "HAVING",
            ]
            .contains(&word.to_ascii_uppercase().as_str());
            (!word.is_empty() && word.len() <= 2 && !keyword).then_some(word)
        });
        if let Some(a) = alias {
            let _ = write!(out, "(SELECT * FROM {table} WHERE {cond}) {a}");
            rest = &tail[1 + a.len()..];
        } else {
            let _ = write!(out, "(SELECT * FROM {table} WHERE {cond}) AS {table}");
            rest = tail;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_leaves_the_query_alone() {
        let sql = "SELECT * FROM detections_ts";
        assert_eq!(RowScope::default().apply(sql), sql);
    }

    #[test]
    fn a_source_scopes_detections_and_listening_and_keeps_aliases() {
        let s = RowScope {
            source: Some("cam'1".into()),
            min_confidence: Some(0.8),
        };
        let sql = "WITH e AS (SELECT * FROM recording_effort WHERE x) \
                   SELECT * FROM detections_ts d LEFT JOIN e ON 1 \
                   JOIN detections_ts ON 1 FROM recording_effort_hourly";
        let got = s.apply(sql);
        assert!(got.contains(
            "FROM (SELECT * FROM recording_effort WHERE source = 'cam''1') AS recording_effort WHERE x"
        ), "{got}");
        assert!(got.contains(
            "FROM (SELECT * FROM detections_ts WHERE Source = 'cam''1' AND Confidence >= 0.8) d LEFT"
        ), "{got}");
        assert!(got.contains(
            "JOIN (SELECT * FROM detections_ts WHERE Source = 'cam''1' AND Confidence >= 0.8) AS detections_ts ON"
        ), "{got}");
        assert!(got.ends_with("FROM recording_effort_hourly"), "{got}");
    }

    #[test]
    fn a_confidence_floor_does_not_touch_listening() {
        let s = RowScope {
            source: None,
            min_confidence: Some(0.5),
        };
        let got = s.apply("SELECT * FROM recording_effort, x FROM detections_ts\n WHERE 1");
        assert!(got.starts_with("SELECT * FROM recording_effort,"), "{got}");
        assert!(
            got.contains("WHERE Confidence >= 0.5) AS detections_ts\n WHERE 1"),
            "{got}"
        );
    }
}
