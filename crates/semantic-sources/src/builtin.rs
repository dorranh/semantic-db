use super::*;
use serde::de::DeserializeOwned;
#[cfg(feature = "postgres")]
#[path = "postgres.rs"]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PostgresConnector;

#[cfg(feature = "clickhouse")]
#[path = "clickhouse.rs"]
mod clickhouse;
#[cfg(feature = "clickhouse")]
pub use clickhouse::ClickHouseConnector;

pub(super) fn options<T: DeserializeOwned>(value: &Options) -> Result<T> {
    serde_json::from_value(Value::Object(value.clone()))
        .map_err(|e| SourceError::configuration("options", "/", e.to_string()))
}
