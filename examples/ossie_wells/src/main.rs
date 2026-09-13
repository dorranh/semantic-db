//! Run with: cargo run -p example-ossie-wells
use semantic_db::{
    engine::pretty_format_batches,
    ossie::{OssieDocument, SourceBindings},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let document = OssieDocument::parse(include_str!("../../geospatial/wells.ossie.yaml"))?;
    let mut sources = SourceBindings::new();
    sources
        .bind_csv(
            "fixtures.geospatial.wells",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../geospatial/wells.csv"),
        )
        .await?;
    let imported = document.load(Some("geospatial_wells"), &sources)?;
    for warning in imported.warnings {
        eprintln!("Warning: {warning}");
    }
    let batches = imported
        .engine
        .query(include_str!("../../geospatial/query.sql"))
        .await?;
    println!("{}", pretty_format_batches(&batches)?);
    Ok(())
}
