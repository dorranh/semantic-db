//! Independent rows for two nested canonical direct-projection views.

use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    Catalog, ObjectRef, Relation, RelationKind, RelationSemantics, ViewOutputLineage,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions, RelationBackend, TableProvider};
use semantic_plan::typed::{
    Direction, FieldRef, NullOrder, RelationInput, Requirement, RowOperation, RowQuery,
};

struct Backend(Arc<MemTable>);
impl RelationBackend for Backend {
    async fn resolve(&self, _: &Relation) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        Ok(self.0.clone())
    }
}

fn source_reference(relations: impl IntoIterator<Item = Relation>, name: &str) -> ObjectRef {
    Catalog::from_relations(relations)
        .unwrap()
        .snapshot()
        .relation(name)
        .unwrap()
        .reference()
        .clone()
}

fn view(
    name: &str,
    schema: Arc<Schema>,
    sql: &str,
    source: ObjectRef,
    columns: BTreeMap<String, String>,
) -> Relation {
    let mut relation = Relation::view(name, schema, sql);
    let RelationKind::View { dependencies, .. } = &mut relation.kind else {
        unreachable!()
    };
    *dependencies = vec![source.id.clone()];
    relation.semantics = Some(RelationSemantics {
        view_lineage: Some(ViewOutputLineage {
            source,
            columns,
            source_refs: vec![],
        }),
        ..Default::default()
    });
    relation
}

fn query() -> RowQuery {
    let field = FieldRef {
        instance: "v".into(),
        field: "id".into(),
    };
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "projected_twice".into(),
            instance: "v".into(),
        },
        requirements: vec![
            Requirement {
                id: "id".into(),
                source_text: "ticket id".into(),
                operation: RowOperation::Project {
                    field: field.clone(),
                    alias: "id".into(),
                },
            },
            Requirement {
                id: "order".into(),
                source_text: "ascending id".into(),
                operation: RowOperation::Order {
                    field,
                    direction: Direction::Asc,
                    nulls: NullOrder::Last,
                },
            },
        ],
        unresolved: vec![],
    }
}

fn ids(batches: &[RecordBatch]) -> Vec<String> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| array_value_to_string(batch.column(0), row).unwrap())
        })
        .collect()
}

#[tokio::test]
async fn nested_direct_projection_lineage_pins_source_and_executes_both_paths() {
    let base_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        base_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![3, 1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("open"), None, Some("closed")])) as ArrayRef,
        ],
    )
    .unwrap();
    let backend = Backend(Arc::new(
        MemTable::try_new(base_schema.clone(), vec![vec![batch]]).unwrap(),
    ));
    let base = Relation::base("tickets", base_schema.clone(), "fixture:tickets");
    let first = view(
        "projected_once",
        base_schema,
        "SELECT id AS id, state AS state FROM tickets",
        source_reference([base.clone()], "tickets"),
        BTreeMap::from([("id".into(), "id".into()), ("state".into(), "state".into())]),
    );
    let second = view(
        "projected_twice",
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
        "SELECT id AS id FROM projected_once",
        source_reference([base.clone(), first.clone()], "projected_once"),
        BTreeMap::from([("id".into(), "id".into())]),
    );
    let engine = Engine::from_catalog([second, base, first], &backend)
        .await
        .unwrap();
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("nested direct view rejected: {:?}", result.outcome)
    };
    let bound = serde_json::to_value(query.bound()).unwrap();
    assert!(
        bound["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|definition| definition["id"] == "view_lineage/projected_twice")
    );
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(ids(&direct), ["1", "2", "3"]);
    assert_eq!(ids(&sql), ["1", "2", "3"]);
}
