//! Offline example: cargo run -p example-authored-views
//! An application supplies the typed selection here. Compiler::compile_views
//! produces this same selection contract from natural language using a model.
use semantic_db::{
    compiler::views::lower_view_selection,
    engine::pretty_format_batches,
    ossie::{OssieDocument, SourceBindings},
    plan::{Comparison, RequestLiteral, SortDirection, ViewFilter, ViewOrder, ViewSelection},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = OssieDocument::parse(include_str!("../../geospatial/wells.ossie.yaml"))?;
    let mut bindings = SourceBindings::new();
    bindings
        .bind_csv(
            "fixtures.geospatial.wells",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../geospatial/wells.csv"),
        )
        .await?;
    let imported = document.load(Some("geospatial_wells"), &bindings)?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let mut engine = imported.engine;
    // This example explicitly authors its own convention; the source model does
    // not define a universal meaning of "deep".
    engine
        .create_view(
            "active_deep_wells",
            "SELECT * FROM wells WHERE status = 'active' AND total_depth_m >= 2500",
        )
        .await?;
    let request = "List well IDs for active deep wells in North Basin, ordered by well_id";
    let selection = ViewSelection {
        view: "active_deep_wells".into(),
        phrase: "active deep wells".into(),
        columns: vec!["well_id".into()],
        filters: vec![ViewFilter::Compare {
            column: "basin".into(),
            op: Comparison::Eq,
            value: RequestLiteral::Text("North Basin".into()),
        }],
        order_by: vec![ViewOrder {
            column: "well_id".into(),
            direction: SortDirection::Asc,
        }],
    };
    let query = lower_view_selection(&engine, request, &selection).await?;
    println!("{}", query.sql);
    println!("{}", query.evidence[0].interpretation);
    let rows = engine.query(&query.sql).await?;
    println!("{}", pretty_format_batches(&rows)?);
    Ok(())
}
