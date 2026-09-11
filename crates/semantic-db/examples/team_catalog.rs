//! Run with: cargo run -p semantic-db --example team_catalog
use std::sync::Arc;

use semantic_db::{
    Catalog, Engine, Relation, RelationBackend, RelationKind, TableProvider,
    arrow::{
        array::{Int64Array, StringArray},
        record_batch::RecordBatch,
    },
    catalog::{DataType, Field, Schema},
    datafusion::{
        datasource::MemTable,
        error::{DataFusionError, Result},
    },
    engine::pretty_format_batches,
};

// A real application can own database pools, API clients, or provider registries
// here. Its physical identifiers need not be the names exposed to SQL or the LLM.
struct TeamBackend {
    orders: Arc<dyn TableProvider>,
}

impl RelationBackend for TeamBackend {
    async fn resolve(&self, relation: &Relation) -> Result<Arc<dyn TableProvider>> {
        match &relation.kind {
            RelationKind::Base { source } if source == "warehouse:orders" => {
                Ok(self.orders.clone())
            }
            _ => Err(DataFusionError::Plan(format!(
                "No source configured for {}",
                relation.name
            ))),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("status", DataType::Utf8, false),
    ]));
    let catalog = Catalog::from_relations([
        // Definitions can be supplied in any order. View dependencies are parsed
        // from SQL; the output schema is checked at startup.
        Relation::view(
            "completed_orders",
            schema.clone(),
            "SELECT * FROM orders WHERE status = 'completed'",
        )
        .with_description("Orders whose fulfillment has completed")
        .with_grain("One row per order"),
        Relation::base("orders", schema.clone(), "warehouse:orders")
            .with_description("Customer orders; status is pending or completed")
            .with_owner("commerce")
            .with_grain("One row per order"),
    ])?;

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["completed", "pending", "completed"])),
        ],
    )?;
    let backend = TeamBackend {
        orders: Arc::new(MemTable::try_new(schema, vec![vec![batch]])?),
    };
    let engine = Engine::from_catalog(catalog, &backend).await?;
    let batches = engine
        .query("SELECT order_id FROM completed_orders ORDER BY order_id")
        .await?;
    println!("{}", pretty_format_batches(&batches)?);
    Ok(())
}
