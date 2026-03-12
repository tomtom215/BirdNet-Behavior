//! Window specification traits and common enumerations.
//!
//! A `WindowSpec` knows how to emit the SQL fragment that defines
//! the time boundary for one type of DuckDB window.

pub mod hopping;
pub mod session;
pub mod sliding;
pub mod tumbling;

pub use hopping::HoppingSpec;
pub use session::SessionSpec;
pub use sliding::SlidingSpec;
pub use tumbling::TumblingSpec;

/// The temporal granularity used when bucketing detections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    /// 15-minute buckets.
    QuarterHour,
    /// 1-hour buckets.
    Hour,
    /// 1-day buckets.
    Day,
    /// 7-day buckets.
    Week,
    /// 1-month buckets.
    Month,
}

impl Granularity {
    /// DuckDB `INTERVAL` literal for the bucket size.
    pub const fn interval_sql(self) -> &'static str {
        match self {
            Self::QuarterHour => "INTERVAL 15 MINUTE",
            Self::Hour => "INTERVAL 1 HOUR",
            Self::Day => "INTERVAL 1 DAY",
            Self::Week => "INTERVAL 7 DAYS",
            Self::Month => "INTERVAL 1 MONTH",
        }
    }

    /// DuckDB `date_trunc` precision string (used for display labels).
    pub const fn trunc_unit(self) -> &'static str {
        match self {
            Self::QuarterHour | Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }
}

/// A window specification that can produce a SQL fragment.
///
/// Implementors describe the temporal boundaries (start expression,
/// end expression, grouping logic) for one type of DuckDB window query.
pub trait WindowSpec {
    /// Return the SQL `SELECT … FROM … GROUP BY` body for this window.
    ///
    /// The returned string should be a complete, runnable DuckDB query
    /// that selects from `detections_ts`.
    fn build_sql(&self) -> String;

    /// Human-readable description of this window type.
    fn description(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granularity_intervals() {
        assert_eq!(Granularity::Hour.interval_sql(), "INTERVAL 1 HOUR");
        assert_eq!(Granularity::Day.interval_sql(), "INTERVAL 1 DAY");
        assert_eq!(Granularity::QuarterHour.interval_sql(), "INTERVAL 15 MINUTE");
    }

    #[test]
    fn granularity_trunc_units() {
        assert_eq!(Granularity::Hour.trunc_unit(), "hour");
        assert_eq!(Granularity::Day.trunc_unit(), "day");
        assert_eq!(Granularity::Month.trunc_unit(), "month");
    }
}
