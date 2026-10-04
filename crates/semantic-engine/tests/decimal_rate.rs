use datafusion::arrow::{
    array::{Array, Decimal128Array},
    datatypes::DataType,
};
use semantic_engine::{Engine, FunctionKind, FunctionPlacement, MVP_EXECUTION_PROFILE};
#[tokio::test]
async fn decimal_rate_sql_keeps_exact_schema_signed_ties_and_constant_controls() {
    let engine = Engine::new();
    let batches=engine.query("SELECT semantic_decimal_rate_v1(CAST(100 AS DECIMAL(3,0)),CAST(0.950050 AS DECIMAL(18,6)),18,2,2) AS positive, semantic_decimal_rate_v1(CAST(-100 AS DECIMAL(3,0)),CAST(0.950050 AS DECIMAL(18,6)),18,2,2) AS negative").await.unwrap();
    assert_eq!(
        batches[0].schema().field(0).data_type(),
        &DataType::Decimal128(18, 2)
    );
    assert_eq!(
        batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap()
            .value(0),
        9501
    );
    assert_eq!(
        batches[0]
            .column(1)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap()
            .value(0),
        -9501
    );
    let null=engine.query("SELECT semantic_decimal_rate_v1(CAST(NULL AS DECIMAL(3,0)),CAST(1 AS DECIMAL(3,0)),18,2,2)").await.unwrap();
    assert!(null[0].column(0).is_null(0));
    for sql in [
        "SELECT semantic_decimal_rate_v1(CAST(1 AS DECIMAL(3,0)),CAST(0 AS DECIMAL(3,0)),18,2,2)",
        "SELECT semantic_decimal_rate_v1(CAST(1 AS DECIMAL(3,0)),CAST(NULL AS DECIMAL(3,0)),18,2,2)",
        "SELECT semantic_decimal_rate_v1(CAST(100 AS DECIMAL(3,0)),CAST(1 AS DECIMAL(3,0)),2,0,2)",
        "SELECT semantic_decimal_rate_v1(1.0,1.0,18,2,2)",
    ] {
        assert!(engine.query(sql).await.is_err(), "{sql}");
    }
    assert_eq!(
        MVP_EXECUTION_PROFILE
            .compiler_function(FunctionKind::Scalar, "semantic_decimal_rate_v1")
            .unwrap()
            .placement,
        FunctionPlacement::LocalOnly
    );
}
