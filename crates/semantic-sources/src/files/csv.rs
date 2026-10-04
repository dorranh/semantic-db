//! Arrow CSV decoding with explicit null semantics, before any projection or COUNT optimization.
use datafusion::{
    arrow::{csv::ReaderBuilder, datatypes::SchemaRef},
    catalog::{TableProvider, streaming::StreamingTable},
    datasource::file_format::file_compression_type::FileCompressionType,
    error::{DataFusionError, Result},
    execution::TaskContext,
    physical_plan::{
        SendableRecordBatchStream, stream::RecordBatchStreamAdapter, streaming::PartitionStream,
    },
};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Debug)]
pub(super) struct Options {
    pub header: bool,
    pub delimiter: u8,
    pub quote: u8,
    pub escape: Option<u8>,
    pub null_regex: Option<String>,
}
pub(super) fn table(
    path: &Path,
    extension: &str,
    schema: SchemaRef,
    compression: FileCompressionType,
    options: Options,
) -> Result<Arc<dyn TableProvider>> {
    let mut paths = vec![];
    super::json::list(path, extension, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        return Err(DataFusionError::Plan("no matching CSV files".into()));
    }
    let partitions = paths
        .into_iter()
        .map(|path| {
            Arc::new(CsvPartition {
                path,
                schema: schema.clone(),
                compression,
                options: options.clone(),
            }) as Arc<dyn PartitionStream>
        })
        .collect();
    Ok(Arc::new(StreamingTable::try_new(schema, partitions)?))
}
#[derive(Debug)]
struct CsvPartition {
    path: PathBuf,
    schema: SchemaRef,
    compression: FileCompressionType,
    options: Options,
}
impl PartitionStream for CsvPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, _: Arc<TaskContext>) -> SendableRecordBatchStream {
        let schema = self.schema.clone();
        let path = self.path.clone();
        let compression = self.compression;
        let options = self.options.clone();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::task::spawn_blocking(move || {
            let result = (|| -> Result<()> {
                let reader = BufReader::new(compression.convert_read(std::fs::File::open(path)?)?);
                let mut builder = ReaderBuilder::new(schema)
                    .with_header(options.header)
                    .with_delimiter(options.delimiter)
                    .with_quote(options.quote)
                    .with_batch_size(1024);
                if let Some(escape) = options.escape {
                    builder = builder.with_escape(escape);
                }
                if let Some(pattern) = options.null_regex {
                    builder = builder.with_null_regex(pattern.parse().map_err(|error| {
                        DataFusionError::Plan(format!("invalid CSV null regex: {error}"))
                    })?);
                }
                // Always decode the complete declared schema. StreamingTable projects only after
                // batch construction, so nullable constraints cannot be hidden by COUNT/projection.
                let mut batches = builder.build(reader)?;
                while !sender.is_closed() {
                    let Some(batch) = batches.next() else {
                        return Ok(());
                    };
                    if sender
                        .blocking_send(batch.map_err(DataFusionError::from))
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = sender.blocking_send(Err(error));
            }
        });
        let stream = futures::stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|batch| (batch, receiver))
        });
        Box::pin(RecordBatchStreamAdapter::new(self.schema.clone(), stream))
    }
}
