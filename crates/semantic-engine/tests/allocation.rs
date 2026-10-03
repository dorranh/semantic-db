use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_engine::Engine;

fn fixture(
    amount: i64,
    categories: &[&str],
    weights: &[i64],
    visible: &[bool],
    expected_count: i64,
    expected_weight: i64,
) -> Engine {
    let mut engine = Engine::new();
    let source_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("eligible_count", DataType::Int64, false),
        Field::new("eligible_weight", DataType::Int64, false),
    ]));
    let source = RecordBatch::try_new(
        source_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(Int64Array::from(vec![amount])),
            Arc::new(Int64Array::from(vec![expected_count])),
            Arc::new(Int64Array::from(vec![expected_weight])),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("orders", source_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(source_schema, vec![vec![source]]).unwrap()),
        )
        .unwrap();
    let bridge_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let bridge = RecordBatch::try_new(
        bridge_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1; categories.len()])) as ArrayRef,
            Arc::new(StringArray::from(categories.to_vec())),
            Arc::new(Int64Array::from(weights.to_vec())),
            Arc::new(BooleanArray::from(visible.to_vec())),
        ],
    )
    .unwrap();
    engine
        .register_table(
            Relation::base("order_category", bridge_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(bridge_schema, vec![vec![bridge]]).unwrap()),
        )
        .unwrap();
    engine
}

const ALLOCATION_SQL: &str = r#"
WITH joined AS (
  SELECT o.id, o.amount, o.eligible_count, o.eligible_weight,
         b.category, b.weight,
         COUNT(b.category) OVER (PARTITION BY o.id) AS actual_count,
         SUM(b.weight) OVER (PARTITION BY o.id) AS actual_weight,
         COUNT(*) OVER (PARTITION BY o.id, b.category) AS target_count,
         semantic_allocation_floor_v1(o.amount,b.weight,o.eligible_weight) AS floor_share,
         semantic_allocation_remainder_v1(o.amount,b.weight,o.eligible_weight) AS remainder
  FROM orders AS o
  LEFT JOIN order_category AS b ON b.order_id = o.id AND b.visible = true
), ranked AS (
  SELECT *,
         MAX(target_count) OVER (PARTITION BY id) AS max_target_count,
         SUM(floor_share) OVER (PARTITION BY id) AS floor_total,
         CAST(ROW_NUMBER() OVER (PARTITION BY id ORDER BY remainder DESC, category ASC) AS BIGINT) AS remainder_rank
  FROM joined
)
SELECT id, category,
       semantic_allocation_share_v1(amount,weight,eligible_weight,remainder_rank,
                                    amount-floor_total,actual_count) AS allocated
FROM ranked
WHERE semantic_assert_allocation_v1(actual_count,eligible_count,actual_weight,
                                     eligible_weight,max_target_count)
ORDER BY id, category
"#;

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                batch
                    .columns()
                    .iter()
                    .map(|column| array_value_to_string(column, row).unwrap())
                    .collect()
            })
        })
        .collect()
}

#[tokio::test]
async fn sql_bridge_allocation_is_exact_order_independent_and_signed() {
    let engine = fixture(101, &["B", "A"], &[2, 1], &[true, true], 2, 3);
    assert_eq!(
        rows(&engine.query(ALLOCATION_SQL).await.unwrap()),
        vec![vec!["1", "A", "34"], vec!["1", "B", "67"],]
    );
    let negative = fixture(-101, &["A", "B"], &[1, 2], &[true, true], 2, 3);
    assert_eq!(
        rows(&negative.query(ALLOCATION_SQL).await.unwrap()),
        vec![vec!["1", "A", "-34"], vec!["1", "B", "-67"],]
    );
    let tie = fixture(1, &["B", "A"], &[1, 1], &[true, true], 2, 2);
    assert_eq!(
        rows(&tie.query(ALLOCATION_SQL).await.unwrap()),
        vec![vec!["1", "A", "1"], vec!["1", "B", "0"],]
    );
}

#[tokio::test]
async fn sql_bridge_allocation_rejects_missing_duplicate_and_policy_filtered_rows() {
    for engine in [
        fixture(101, &["A"], &[1], &[true], 2, 3),
        fixture(101, &["A", "A"], &[1, 2], &[true, true], 2, 3),
        fixture(101, &["A", "B"], &[1, 2], &[true, false], 2, 3),
        fixture(101, &["A", "B"], &[0, 0], &[true, true], 2, 0),
    ] {
        assert!(engine.query(ALLOCATION_SQL).await.is_err());
    }
}

#[tokio::test]
async fn scalar_inputs_reject_null_negative_zero_and_unrepresentable_shares() {
    let engine = Engine::new();
    for sql in [
        "SELECT semantic_allocation_floor_v1(NULL::bigint,1::bigint,2::bigint)",
        "SELECT semantic_allocation_floor_v1(1::bigint,-1::bigint,2::bigint)",
        "SELECT semantic_allocation_floor_v1(1::bigint,1::bigint,0::bigint)",
        "SELECT semantic_allocation_floor_v1(1::bigint,3::bigint,2::bigint)",
        "SELECT semantic_allocation_share_v1(1::bigint,1::bigint,2::bigint,1::bigint,2::bigint,2::bigint)",
        "SELECT semantic_assert_allocation_v1(2::bigint,2::bigint,3::bigint,3::bigint,2::bigint)",
    ] {
        assert!(engine.query(sql).await.is_err(), "{sql}");
    }
}
