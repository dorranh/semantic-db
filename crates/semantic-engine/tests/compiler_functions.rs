use datafusion::common::ScalarValue;
use semantic_engine::Engine;
#[tokio::test]
async fn integer_ratios_preserve_precision_sign_extremes_and_null_zero_contracts() {
    let engine = Engine::new();
    let batches = engine.query("SELECT semantic_ratio_i64_v1(n,d,z) AS ratio FROM (VALUES (1::bigint,3::bigint,false),(-1,3,false),(-9223372036854775808,-1,false),(1,0,false),(1,0,true),(NULL,1,true),(1,NULL,true)) t(n,d,z)").await.unwrap();
    let actual = batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows())
                .map(|row| ScalarValue::try_from_array(batch.column(0), row).unwrap())
        })
        .collect::<Vec<_>>();
    let values = [
        Some(333333333333333333),
        Some(-333333333333333333),
        Some(9223372036854775808000000000000000000),
        None,
        Some(0),
        None,
        None,
    ];
    assert_eq!(
        actual,
        values
            .into_iter()
            .map(|v| ScalarValue::Decimal128(v, 38, 18))
            .collect::<Vec<_>>()
    );
    let scalar = engine
        .query("SELECT semantic_ratio_i64_v1(1::bigint,3::bigint,false)")
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(scalar[0].column(0), 0).unwrap(),
        ScalarValue::Decimal128(Some(333333333333333333), 38, 18)
    );
}

// A remote engine without compiler UDFs proves that federation cannot silently
// substitute remote arithmetic for the versioned local contract.
#[tokio::test]
async fn exact_ratio_stays_local_while_its_aggregate_is_federated() {
    use async_trait::async_trait;
    use datafusion::{
        arrow::{
            array::Int64Array,
            datatypes::{DataType, Field, Schema, SchemaRef},
            record_batch::RecordBatch,
        },
        common::TableReference,
        datasource::MemTable,
        physical_plan::{
            PhysicalExpr, SendableRecordBatchStream, stream::RecordBatchStreamAdapter,
        },
        prelude::SessionContext,
        sql::unparser::dialect::{DefaultDialect, Dialect},
    };
    use datafusion_federation::{
        FederatedTableProviderAdaptor,
        sql::{SQLExecutor, SQLFederationProvider, SQLTableSource},
    };
    use futures::TryStreamExt;
    use std::sync::{Arc, Mutex};
    struct Remote {
        session: Arc<SessionContext>,
        queries: Arc<Mutex<Vec<String>>>,
        schema: SchemaRef,
    }
    #[async_trait]
    impl SQLExecutor for Remote {
        fn name(&self) -> &str {
            "test"
        }
        fn compute_context(&self) -> Option<String> {
            Some("ratio-test".into())
        }
        fn dialect(&self) -> Arc<dyn Dialect> {
            Arc::new(DefaultDialect {})
        }
        fn execute(
            &self,
            sql: &str,
            schema: SchemaRef,
            _: &[Arc<dyn PhysicalExpr>],
        ) -> datafusion::error::Result<SendableRecordBatchStream> {
            self.queries.lock().unwrap().push(sql.to_owned());
            let session = self.session.clone();
            let sql = sql.to_owned();
            let stream =
                futures::stream::once(
                    async move { session.sql(&sql).await?.execute_stream().await },
                )
                .try_flatten();
            Ok(Box::pin(RecordBatchStreamAdapter::new(schema, stream)))
        }
        async fn table_names(&self) -> datafusion::error::Result<Vec<String>> {
            Ok(vec!["amounts".into()])
        }
        async fn get_table_schema(&self, _: &str) -> datafusion::error::Result<SchemaRef> {
            Ok(self.schema.clone())
        }
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("n", DataType::Int64, false),
        Field::new("d", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 1])),
            Arc::new(Int64Array::from(vec![3, 3])),
        ],
    )
    .unwrap();
    let table = Arc::new(MemTable::try_new(schema.clone(), vec![vec![batch]]).unwrap());
    let session = Arc::new(SessionContext::new());
    session.register_table("amounts", table.clone()).unwrap();
    let queries = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(SQLFederationProvider::new(Arc::new(Remote {
        session,
        queries: queries.clone(),
        schema: schema.clone(),
    })));
    let source = Arc::new(SQLTableSource::new_with_schema(
        provider,
        TableReference::bare("amounts").into(),
        schema.clone(),
    ));
    let federated = Arc::new(FederatedTableProviderAdaptor::new_with_provider(
        source, table,
    ));
    let mut engine = Engine::new();
    engine
        .register_table(
            semantic_catalog::Relation::base("amounts", schema, "test"),
            federated,
        )
        .unwrap();
    let result = engine
        .query("SELECT semantic_ratio_i64_v1(SUM(n), SUM(d), false) FROM amounts")
        .await
        .unwrap();
    assert_eq!(
        ScalarValue::try_from_array(result[0].column(0), 0).unwrap(),
        ScalarValue::Decimal128(Some(333333333333333333), 38, 18)
    );
    let sql = queries.lock().unwrap().join("\n").to_lowercase();
    assert!(sql.contains("sum("), "aggregate was not federated: {sql}");
    assert!(
        !sql.contains("semantic_ratio_i64_v1"),
        "local contract shipped remotely: {sql}"
    );
}

#[tokio::test]
async fn uniqueness_guard_rejects_multiple_even_identical_matches() {
    let engine = Engine::new();
    assert!(
        engine
            .query("SELECT semantic_assert_single_v1(1::bigint)")
            .await
            .is_ok()
    );
    for value in ["2", "-1", "NULL"] {
        let sql = format!("SELECT semantic_assert_single_v1({value}::bigint)");
        assert!(
            engine
                .query(&sql)
                .await
                .unwrap_err()
                .to_string()
                .contains("uniqueness obligation failed")
        );
    }
}

#[tokio::test]
async fn checked_sums_reject_overflow_preserve_cancellation_distinct_and_empty_state() {
    let engine = Engine::new();
    for sql in [
        "SELECT semantic_sum_v1(v) FROM (VALUES (9223372036854775807::bigint),(1)) t(v)",
        "SELECT semantic_sum_v1(v) FROM (VALUES ('99999999999999999999999999999999999999'::decimal(38,0)),(1::decimal(38,0))) t(v)",
        "SELECT semantic_sum_v1(v) OVER (ORDER BY id RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM (VALUES (1,9223372036854775807::bigint),(2,1)) t(id,v)",
    ] {
        assert!(
            engine
                .query(sql)
                .await
                .unwrap_err()
                .to_string()
                .contains("semantic sum overflow"),
            "{sql}"
        );
    }
    for sql in [
        "SELECT semantic_sum_v1(v) FROM (VALUES (9223372036854775807::bigint),(1),(-1)) t(v)",
        "SELECT semantic_sum_v1(DISTINCT v) FROM (VALUES (9223372036854775807::bigint),(9223372036854775807)) t(v)",
    ] {
        let batches = engine.query(sql).await.unwrap();
        assert_eq!(
            ScalarValue::try_from_array(batches[0].column(0), 0).unwrap(),
            ScalarValue::Int64(Some(i64::MAX))
        );
    }
    for sql in [
        "SELECT semantic_sum_v1(v) FROM (VALUES (NULL::bigint)) t(v)",
        "SELECT semantic_sum_v1(DISTINCT v) FROM (VALUES (1::bigint)) t(v) WHERE false",
    ] {
        let batches = engine.query(sql).await.unwrap();
        assert_eq!(
            ScalarValue::try_from_array(batches[0].column(0), 0).unwrap(),
            ScalarValue::Int64(None)
        );
    }
    let batches = engine.query("SELECT semantic_sum_v1(DISTINCT v) FROM (VALUES (1.25::decimal(12,2)),(1.25::decimal(12,2)),(-0.25::decimal(12,2))) t(v)").await.unwrap();
    assert_eq!(
        ScalarValue::try_from_array(batches[0].column(0), 0).unwrap(),
        ScalarValue::Decimal128(Some(100), 22, 2)
    );
}
