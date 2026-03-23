//! Error types for time-series analytics.

use std::fmt;

/// Errors that can occur during time-series query building or execution.
#[derive(Debug)]
pub enum TimeSeriesError {
    /// The underlying DuckDB query failed.
    #[cfg(feature = "analytics")]
    Database(duckdb::Error),
    /// A required view or table was not found in the `DuckDB` database.
    MissingView(String),
    /// A query parameter was invalid (e.g., end before start).
    InvalidParam(String),
    /// The query returned data in an unexpected format.
    InvalidData(String),
}

impl fmt::Display for TimeSeriesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "analytics")]
            Self::Database(e) => write!(f, "DuckDB error: {e}"),
            Self::MissingView(v) => write!(f, "required view not found: {v}"),
            Self::InvalidParam(msg) => write!(f, "invalid parameter: {msg}"),
            Self::InvalidData(msg) => write!(f, "unexpected query result: {msg}"),
        }
    }
}

impl std::error::Error for TimeSeriesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(feature = "analytics")]
            Self::Database(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(feature = "analytics")]
impl From<duckdb::Error> for TimeSeriesError {
    fn from(e: duckdb::Error) -> Self {
        Self::Database(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_missing_view() {
        let e = TimeSeriesError::MissingView("detections_ts".into());
        assert!(e.to_string().contains("detections_ts"));
    }

    #[test]
    fn display_invalid_param() {
        let e = TimeSeriesError::InvalidParam("end < start".into());
        assert!(e.to_string().contains("end < start"));
    }
}
