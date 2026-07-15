//! Semantic core for TraceFold archives.

pub mod aggregate;
pub mod contract;
pub mod generator;
pub mod normalize;
pub mod schema;

pub use aggregate::{
    AggregateRow, CellKey, CellState, MetricState, Oracle, OracleIndex, QueryResult, QuerySpec,
    ScalarValue,
};
pub use contract::{Contract, Family, Measure, MeasureOp, RecentRetention};
pub use schema::{CanonicalEvent, Severity, Status};

pub const CANONICAL_SCHEMA_VERSION: u16 = 1;
pub const CONTRACT_SCHEMA_VERSION: u16 = 1;
pub const QUERY_RESULT_SCHEMA_VERSION: u16 = 1;
