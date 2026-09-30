use datafusion::common::ScalarValue;
use semantic_engine::Engine;

async fn scalar(sql: &str) -> ScalarValue {
    let batches = Engine::new().query(sql).await.unwrap();
    ScalarValue::try_from_array(batches[0].column(0), 0).unwrap()
}

#[tokio::test]
async fn rational_conversion_is_exact_with_explicit_rounding() {
    assert_eq!(
        scalar("SELECT semantic_scale_i64_v1(5::bigint,1::bigint,2::bigint,false)").await,
        ScalarValue::Decimal128(Some(2_500_000_000_000_000_000), 38, 18)
    );
    assert_eq!(
        scalar("SELECT semantic_scale_i64_v1(-1::bigint,1::bigint,3::bigint,false)").await,
        ScalarValue::Decimal128(Some(-333_333_333_333_333_333), 38, 18)
    );
    assert_eq!(
        scalar(
            "SELECT semantic_scale_i64_v1(1::bigint,1::bigint,2000000000000000000::bigint,true)"
        )
        .await,
        ScalarValue::Decimal128(Some(0), 38, 18)
    );
    assert_eq!(
        scalar(
            "SELECT semantic_scale_i64_v1(3::bigint,1::bigint,2000000000000000000::bigint,true)"
        )
        .await,
        ScalarValue::Decimal128(Some(2), 38, 18)
    );
    assert_eq!(
        scalar("SELECT semantic_scale_i64_v1(NULL::bigint,1::bigint,2::bigint,false)").await,
        ScalarValue::Decimal128(None, 38, 18)
    );
}

#[tokio::test]
async fn invalid_factor_and_decimal_overflow_fail() {
    let engine = Engine::new();
    for sql in [
        "SELECT semantic_scale_i64_v1(1::bigint,1::bigint,0::bigint,false)",
        "SELECT semantic_scale_i64_v1(1::bigint,0::bigint,1::bigint,false)",
        "SELECT semantic_scale_i64_v1(1::bigint,-1::bigint,1::bigint,false)",
        "SELECT semantic_scale_i64_v1(9223372036854775807::bigint,9223372036854775807::bigint,1::bigint,false)",
        "SELECT semantic_scale_i64_v1('x',1::bigint,2::bigint,false)",
    ] {
        assert!(engine.query(sql).await.is_err(), "{sql}");
    }
}
