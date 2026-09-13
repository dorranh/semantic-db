#[path = "../../support/clickhouse.rs"]
mod clickhouse_fixture;
use clickhouse_fixture::Database;
use semantic_engine::Engine;
use serde_json::json;

#[tokio::test]
#[ignore = "benchmark: requires Docker; writes docs/generated benchmark artifact"]
async fn clickhouse_controlled_benchmark() {
    use semantic_catalog::Relation;
    let database = Database::start().await;
    database.execute("CREATE TABLE bench ENGINE=MergeTree ORDER BY id AS SELECT number AS id,number%100 AS k,toFloat64(number) AS value,repeat(toString(cityHash64(number)),16) AS payload FROM numbers(50000)").await;
    let mut records = vec![];
    for federation in [false, true] {
        let connection = database.connection(federation);
        let provider = connection.table("bench").await.unwrap();
        let mut engine = Engine::new();
        engine
            .register_table(
                Relation::base("bench", provider.schema(), "benchmark"),
                provider,
            )
            .unwrap();
        for (workload, sql) in [
            (
                "aggregate",
                "SELECT k,SUM(value) FROM bench GROUP BY k ORDER BY k",
            ),
            (
                "partial",
                "SELECT k,SQRT(SUM(value)) FROM bench GROUP BY k ORDER BY k",
            ),
            (
                "selective",
                "SELECT id FROM bench WHERE k=1 ORDER BY id LIMIT 50",
            ),
        ] {
            for repetition in 0..6 {
                let before = connection.metrics();
                let start = std::time::Instant::now();
                let batches = engine.query(sql).await.unwrap();
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                let after = connection.metrics();
                if repetition > 0 {
                    records.push(json!({"workload":workload,"federation":federation,"latency_ms":elapsed,"remote_rows":after.rows-before.rows,"remote_bytes":after.bytes-before.bytes,"rows":batches.iter().map(|b|b.num_rows()).sum::<usize>()}));
                }
            }
        }
    }
    let mut experiments = vec![];
    for repetition in 0..6 {
        for (name, clause) in [("where", "WHERE"), ("prewhere", "PREWHERE")] {
            let sql = format!(
                "SELECT sum(value) FROM bench {clause} k=1 SETTINGS optimize_move_to_prewhere=0,use_query_cache=0,max_threads=2"
            );
            let start = std::time::Instant::now();
            let result = database.admin.query(&sql).fetch_one::<f64>().await.unwrap();
            if repetition > 0 {
                experiments.push(json!({"experiment":name,"latency_ms":start.elapsed().as_secs_f64()*1000.0,"result":result}));
            }
        }
        let start = std::time::Instant::now();
        let (left,right)=tokio::join!(
            database.admin.query("SELECT sum(value) FROM bench WHERE k=1 AND id<25000 SETTINGS use_query_cache=0,max_threads=1").fetch_one::<f64>(),
            database.admin.query("SELECT sum(value) FROM bench WHERE k=1 AND id>=25000 SETTINGS use_query_cache=0,max_threads=1").fetch_one::<f64>()
        );
        let result = left.unwrap() + right.unwrap();
        if repetition > 0 {
            experiments.push(json!({"experiment":"parallel","latency_ms":start.elapsed().as_secs_f64()*1000.0,"result":result}));
        }
    }
    let version = database
        .admin
        .query("SELECT version()")
        .fetch_one::<String>()
        .await
        .unwrap();
    let report = json!({"profile":"controlled","server_version":version,"records":records,"experiments":experiments,"adoption_gate":{"median_latency_improvement":0.15,"remote_bytes_improvement":0.25,"maximum_comparison_regression":0.10}});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/generated/clickhouse-controlled-benchmark.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("{}", path.display());
}
