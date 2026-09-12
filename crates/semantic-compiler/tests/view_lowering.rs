use semantic_compiler::views::{ViewSelectionError, lower_view_selection};
use semantic_engine::{Engine, pretty_format_batches};
use semantic_plan::{
    Comparison, RequestLiteral, SortDirection, ViewFilter, ViewOrder, ViewSelection,
};

async fn fixture() -> Engine {
    let mut engine = Engine::new();
    engine.create_view("records", "SELECT 1 AS id, 'O''Brien' AS \"odd\"\"column\", 2500.125 AS amount, true AS enabled, CAST(NULL AS VARCHAR) AS missing UNION ALL SELECT 2, 'x'' OR TRUE --', 3.5, false, 'known'").await.unwrap();
    engine
}

fn selection(filters: Vec<ViewFilter>) -> ViewSelection {
    ViewSelection {
        view: "records".into(),
        phrase: "records".into(),
        columns: vec!["id".into()],
        filters,
        order_by: vec![ViewOrder {
            column: "id".into(),
            direction: SortDirection::Asc,
        }],
    }
}

fn compare(column: &str, op: Comparison, value: RequestLiteral) -> ViewFilter {
    ViewFilter::Compare {
        column: column.into(),
        op,
        value,
    }
}

#[tokio::test]
async fn quotes_identifiers_and_treats_sql_looking_text_as_a_value() {
    let engine = fixture().await;
    for (text, expected_id) in [("O'Brien", "1"), ("x' OR TRUE --", "2")] {
        let query = lower_view_selection(
            &engine,
            &format!("Find records for {text}"),
            &selection(vec![compare(
                "odd\"column",
                Comparison::Eq,
                RequestLiteral::Text(text.into()),
            )]),
        )
        .await
        .unwrap();
        let rows = engine.query(&query.sql).await.unwrap();
        assert_eq!(rows.iter().map(|batch| batch.num_rows()).sum::<usize>(), 1);
        assert!(
            pretty_format_batches(&rows)
                .unwrap()
                .to_string()
                .contains(&format!("| {expected_id}  |"))
        );
    }
}

#[tokio::test]
async fn numeric_comparisons_match_reference_queries_and_preserve_decimal_spelling() {
    let engine = fixture().await;
    for (op, sql_op) in [
        (Comparison::Eq, "="),
        (Comparison::NotEq, "<>"),
        (Comparison::Lt, "<"),
        (Comparison::Lte, "<="),
        (Comparison::Gt, ">"),
        (Comparison::Gte, ">="),
    ] {
        let query = lower_view_selection(
            &engine,
            "Find records with amount compared to 2500.125.",
            &selection(vec![compare(
                "amount",
                op,
                RequestLiteral::Number("2500.125".into()),
            )]),
        )
        .await
        .unwrap();
        assert!(query.sql.contains("2500.125"));
        let actual = engine.query(&query.sql).await.unwrap();
        let expected = engine
            .query(&format!(
                "SELECT id FROM records WHERE amount {sql_op} 2500.125 ORDER BY id"
            ))
            .await
            .unwrap();
        assert_eq!(
            pretty_format_batches(&actual).unwrap().to_string(),
            pretty_format_batches(&expected).unwrap().to_string()
        );
    }
}

#[tokio::test]
async fn boolean_null_filters_and_nulls_last_sorting_execute() {
    let engine = fixture().await;
    for (filters, expected_id) in [
        (
            vec![
                compare(
                    "enabled",
                    Comparison::Eq,
                    RequestLiteral::Boolean("true".into()),
                ),
                ViewFilter::IsNull {
                    column: "missing".into(),
                },
            ],
            "1",
        ),
        (
            vec![ViewFilter::IsNotNull {
                column: "missing".into(),
            }],
            "2",
        ),
    ] {
        let query = lower_view_selection(
            &engine,
            "Find records where enabled is true and missing is null",
            &selection(filters),
        )
        .await
        .unwrap();
        let rows = engine.query(&query.sql).await.unwrap();
        assert_eq!(rows.iter().map(|batch| batch.num_rows()).sum::<usize>(), 1);
        assert!(
            pretty_format_batches(&rows)
                .unwrap()
                .to_string()
                .contains(&format!("| {expected_id}  |"))
        );
    }
    for direction in [SortDirection::Asc, SortDirection::Desc] {
        let mut selection = selection(vec![]);
        selection.order_by = vec![ViewOrder {
            column: "missing".into(),
            direction,
        }];
        let query = lower_view_selection(&engine, "List records ordered by missing", &selection)
            .await
            .unwrap();
        let rows = pretty_format_batches(&engine.query(&query.sql).await.unwrap())
            .unwrap()
            .to_string();
        assert!(rows.find("| 2  |").unwrap() < rows.find("| 1  |").unwrap());
    }
}

#[tokio::test]
async fn rejects_inferred_values_numeric_fragments_and_implicit_type_conversions() {
    let engine = fixture().await;
    for request in [
        "records above 2500",
        "records above -500",
        "records above 500.5",
        "records above 2,500",
        "records above 500e2",
        "records above 5000",
    ] {
        let error = lower_view_selection(
            &engine,
            request,
            &selection(vec![compare(
                "amount",
                Comparison::Gt,
                RequestLiteral::Number("500".into()),
            )]),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, ViewSelectionError::MissingLiteral(_)),
            "{request}: {error}"
        );
    }
    for (column, value) in [
        ("amount", RequestLiteral::Text("2500".into())),
        ("odd\"column", RequestLiteral::Number("2500".into())),
        ("enabled", RequestLiteral::Number("1".into())),
        ("amount", RequestLiteral::Number("1 OR true".into())),
        ("amount", RequestLiteral::Number("NaN".into())),
        ("enabled", RequestLiteral::Boolean("TRUE".into())),
    ] {
        let error = lower_view_selection(
            &engine,
            "records 2500 1 OR true NaN TRUE",
            &selection(vec![compare(column, Comparison::Eq, value)]),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, ViewSelectionError::InvalidLiteral(_)),
            "{error}"
        );
    }
}

#[tokio::test]
async fn compilation_does_not_read_rows() {
    let mut engine = Engine::new();
    // This definition plans successfully but fails when its value is evaluated.
    engine
        .create_view("bad_data", "SELECT CAST('not-an-integer' AS INT) AS id")
        .await
        .unwrap();
    let mut selection = selection(vec![]);
    selection.view = "bad_data".into();
    let query = lower_view_selection(&engine, "List records", &selection)
        .await
        .unwrap();
    assert!(engine.query(&query.sql).await.is_err());
}
