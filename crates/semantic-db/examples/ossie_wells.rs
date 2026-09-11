//! Run with: cargo run -p semantic-db --features ossie --example ossie_wells
use semantic_db::{
    engine::pretty_format_batches,
    ossie::{OssieDocument, SourceBindings},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = OssieDocument::parse(include_str!(
        "../../../examples/geospatial/wells.ossie.yaml"
    ))?;
    let mut sources = SourceBindings::new();
    sources
        .bind_csv(
            "fixtures.geospatial.wells",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/geospatial/wells.csv"
            ),
        )
        .await?;
    let imported = document.load(Some("geospatial_wells"), &sources)?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let batches = imported
        .engine
        .query(include_str!("../../../examples/geospatial/query.sql"))
        .await?;
    println!("{}", pretty_format_batches(&batches)?);
    Ok(())
}
