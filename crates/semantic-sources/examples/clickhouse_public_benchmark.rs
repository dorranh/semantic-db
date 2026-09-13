//! Opt-in, sequential public-instance smoke benchmark (32 requests maximum).
//! SEMANTIC_PUBLIC_CLICKHOUSE=1 cargo run -p semantic-sources --features clickhouse --example clickhouse_public_benchmark
use semantic_catalog::Relation;
use semantic_clickhouse::{ClickHouse, ClickHouseConfig, ServerLimits, TableOptions};
use semantic_engine::{Engine, QueryOptions};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("SEMANTIC_PUBLIC_CLICKHOUSE").as_deref() != Ok("1") {
        return Err("set SEMANTIC_PUBLIC_CLICKHOUSE=1 to opt into the public benchmark".into());
    }
    let mut records = vec![];
    let queries = [
        (
            "projection",
            "SELECT id,title FROM stories WHERE id >= 0 AND id < 1000 ORDER BY id LIMIT 100",
        ),
        (
            "aggregate",
            "SELECT type,COUNT(*) AS n FROM stories WHERE id >= 0 AND id < 1000 GROUP BY type ORDER BY type",
        ),
        (
            "partial",
            "SELECT SQRT(CAST(COUNT(*) AS DOUBLE)) AS n FROM stories WHERE id >= 0 AND id < 1000 GROUP BY type ORDER BY type",
        ),
    ];
    let mut reference = std::collections::BTreeMap::new();
    let mut requests = 0usize;
    for federation in [false, true] {
        let mut config = ClickHouseConfig::new(
            "https://sql-clickhouse.clickhouse.com:443",
            "hackernews",
            "demo",
            "",
        );
        config.federation = federation;
        config.filter_pushdown = true;
        config.max_concurrent_requests = 1;
        config.max_block_size = 10000;
        config.max_attempts = 1;
        config.query_timeout = Duration::from_secs(10);
        config.max_response_bytes = 16 * 1024 * 1024;
        config.max_decoded_bytes = 64 * 1024 * 1024;
        config.server = ServerLimits {
            max_rows_to_read: Some(1_000_000),
            max_bytes_to_read: Some(268_435_456),
            max_result_rows: Some(10_000),
            max_result_bytes: Some(16_777_216),
            ..Default::default()
        };
        let client = ClickHouse::new(config)?;
        let capabilities = client.capabilities().await?;
        requests += 1;
        let provider = client
            .table_with_options(
                "hackernews",
                TableOptions {
                    columns: Some(vec!["id".into(), "title".into(), "type".into()]),
                    ..Default::default()
                },
            )
            .await?;
        requests += 3; // table metadata, column metadata, Arrow schema
        let mut engine = Engine::new();
        engine.register_table(
            Relation::base("stories", provider.schema(), "public-hackernews"),
            provider,
        )?;
        for (workload, sql) in queries {
            let plan = engine
                .plan_sql(sql)
                .await?
                .logical_plan()
                .display_indent()
                .to_string();
            for repetition in 0..4 {
                if requests >= 40 {
                    return Err("public request cap reached".into());
                }
                let start = Instant::now();
                let execution = engine
                    .execute(
                        sql,
                        QueryOptions {
                            timeout_seconds: 10,
                            max_remote_bytes: 16 * 1024 * 1024,
                            max_decoded_bytes: 64 * 1024 * 1024,
                            ..Default::default()
                        },
                    )
                    .await?;
                let context = execution.context.clone();
                let batches = execution.collect().await?;
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                requests += context.metrics().remote_requests;
                let result = semantic_engine::pretty_format_batches(&batches)?.to_string();
                if let Some(expected) = reference.get(workload) {
                    if expected != &result {
                        return Err(format!("result mismatch: {workload}").into());
                    }
                } else {
                    reference.insert(workload, result);
                }
                if repetition != 0 {
                    records.push(serde_json::json!({"workload":workload,"federation":federation,"repetition":repetition,"latency_ms":elapsed,"metrics":context.metrics(),"server_version":capabilities.version,"logical_plan":plan}));
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"profile":"public-smoke","requests":requests,"performance_gate":false,"records":records})
        )?
    );
    Ok(())
}
