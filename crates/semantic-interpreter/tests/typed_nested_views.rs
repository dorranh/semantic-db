//! Independent expected rows for the architecture's nested local view example.

use std::sync::{Arc, Mutex};

use datafusion::{
    arrow::{
        array::{ArrayRef, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::Relation;
use semantic_compiler::typed::TypedOutcome;
use semantic_engine::{Engine, QueryOptions, RelationBackend, TableProvider};
use semantic_interpreter::{
    Interpreter,
    provider::{Message, ModelProvider, ProviderError},
    typed::{InterpretOptions, SelectionMode},
};
use semantic_plan::typed::{
    Direction, FieldRef, NullOrder, RelationInput, Requirement, RowOperation, RowQuery,
};

struct Backend(Arc<MemTable>);
impl RelationBackend for Backend {
    async fn resolve(&self, _: &Relation) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        Ok(self.0.clone())
    }
}

struct Proposal {
    output: String,
    contexts: Arc<Mutex<Vec<String>>>,
}
impl ModelProvider for Proposal {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        self.contexts
            .lock()
            .unwrap()
            .push(messages[1].content.clone());
        Ok(self.output.clone())
    }
}

fn query() -> RowQuery {
    let field = FieldRef {
        instance: "v".into(),
        field: "well_id".into(),
    };
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "active_deep_wells".into(),
            instance: "v".into(),
        },
        requirements: vec![
            Requirement {
                id: "ids".into(),
                source_text: "IDs".into(),
                operation: RowOperation::Project {
                    field: field.clone(),
                    alias: "well_id".into(),
                },
            },
            Requirement {
                id: "order".into(),
                source_text: "ordered by ID".into(),
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

async fn engine() -> Engine {
    let base_schema = Arc::new(Schema::new(vec![
        Field::new("well_id", DataType::Int64, false),
        Field::new("total_depth_m", DataType::Int64, false),
        Field::new("status", DataType::Utf8, false),
        Field::new("basin", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        base_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])) as ArrayRef,
            Arc::new(Int64Array::from(vec![3000, 2700, 2000, 2600, 3500])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "active", "inactive", "active", "active", "active",
            ])) as ArrayRef,
            Arc::new(StringArray::from(vec![
                "North Basin",
                "North Basin",
                "North Basin",
                "South Basin",
                "North Basin",
            ])) as ArrayRef,
        ],
    )
    .unwrap();
    let backend = Backend(Arc::new(
        MemTable::try_new(base_schema.clone(), vec![vec![batch]]).unwrap(),
    ));
    let wells = Relation::base("wells", base_schema.clone(), "fixture:wells");
    let deep = Relation::view(
        "deep_wells",
        base_schema,
        "SELECT well_id, total_depth_m, status, basin FROM wells WHERE total_depth_m >= 2500",
    )
    .with_description("Locally authored deep threshold; 2500 metres is not universal");
    let active_schema = Arc::new(Schema::new(vec![Field::new(
        "well_id",
        DataType::Int64,
        false,
    )]));
    let active = Relation::view(
        "active_deep_wells",
        active_schema,
        "SELECT well_id FROM deep_wells WHERE status = 'active' AND basin = 'North Basin'",
    )
    .with_description("Active deep wells in North Basin");
    Engine::from_catalog([active, wells, deep], &backend)
        .await
        .unwrap()
}

fn rows(batches: &[RecordBatch]) -> Vec<String> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| array_value_to_string(batch.column(0), row).unwrap())
        })
        .collect()
}

#[tokio::test]
async fn nested_authored_views_retain_local_definition_and_exact_rows() {
    let engine = engine().await;
    let contexts = Arc::new(Mutex::new(Vec::new()));
    let provider = Proposal {
        output: serde_json::json!({"status":"query", "query": query()}).to_string(),
        contexts: contexts.clone(),
    };
    let result = Interpreter::new(provider)
        .compile_typed(
            &engine,
            "List IDs of active deep wells in North Basin, ordered by ID",
            InterpretOptions::default(),
        )
        .await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("nested authored view failed: {:?}", result.outcome)
    };
    let seen = contexts.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains("active_deep_wells"));
    assert!(seen[0].contains("deep_wells"));
    assert!(seen[0].contains("2500"));
    drop(seen);
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
    assert_eq!(rows(&direct), ["1", "5"]);
    assert_eq!(rows(&sql), ["1", "5"]);
}
