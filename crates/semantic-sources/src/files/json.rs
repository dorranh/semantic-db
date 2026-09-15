//! Validate integer tokens before Arrow decoding, which otherwise permits truncation.
use datafusion::{
    arrow::{datatypes::SchemaRef, json::ReaderBuilder},
    catalog::{TableProvider, streaming::StreamingTable},
    datasource::file_format::file_compression_type::FileCompressionType,
    error::{DataFusionError, Result},
    execution::TaskContext,
    physical_plan::{
        SendableRecordBatchStream, stream::RecordBatchStreamAdapter, streaming::PartitionStream,
    },
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) fn table(
    path: &Path,
    extension: &str,
    schema: SchemaRef,
    compression: FileCompressionType,
) -> Result<Arc<dyn TableProvider>> {
    let mut paths = Vec::new();
    list(path, extension, &mut paths)?;
    paths.sort();
    if paths.is_empty() {
        return Err(DataFusionError::Plan("no matching NDJSON files".into()));
    }
    let partitions = paths
        .into_iter()
        .map(|path| {
            Arc::new(JsonPartition {
                path,
                schema: schema.clone(),
                compression,
            }) as Arc<dyn PartitionStream>
        })
        .collect();
    Ok(Arc::new(StreamingTable::try_new(schema, partitions)?))
}
fn list(path: &Path, extension: &str, paths: &mut Vec<PathBuf>) -> Result<()> {
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            // Do not traverse symlink directories (cycles / different listing semantics).
            if entry.file_type()?.is_symlink() {
                continue;
            }
            list(&entry.path(), extension, paths)?;
        }
    } else if path.is_file() && path.to_string_lossy().ends_with(extension) {
        paths.push(path.to_owned());
    }
    Ok(())
}
#[derive(Debug)]
struct JsonPartition {
    path: PathBuf,
    schema: SchemaRef,
    compression: FileCompressionType,
}
impl PartitionStream for JsonPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, _: Arc<TaskContext>) -> SendableRecordBatchStream {
        let schema = self.schema.clone();
        let path = self.path.clone();
        let compression = self.compression;
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::task::spawn_blocking(move || {
            let result = (|| -> Result<()> {
                let mut reader =
                    BufReader::new(compression.convert_read(std::fs::File::open(path)?)?);
                loop {
                    if sender.is_closed() {
                        return Ok(());
                    }
                    let mut bytes = Vec::new();
                    let mut rows = 0;
                    while rows < 1024 && bytes.len() < 8 * 1024 * 1024 {
                        if sender.is_closed() {
                            return Ok(());
                        }
                        let mut line = Vec::new();
                        const MAX_RECORD: u64 = 16 * 1024 * 1024;
                        let count = reader
                            .by_ref()
                            .take(MAX_RECORD + 1)
                            .read_until(b'\n', &mut line)?;
                        if count == 0 {
                            break;
                        }
                        if count as u64 > MAX_RECORD {
                            return Err(DataFusionError::Execution(
                                "NDJSON record exceeds 16 MiB".into(),
                            ));
                        }
                        if line.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }
                        let value: Value = serde_json::from_slice(&line).map_err(|e| {
                            DataFusionError::Execution(format!("invalid NDJSON: {e}"))
                        })?;
                        let object = value.as_object().ok_or_else(|| {
                            DataFusionError::Execution("NDJSON requires one object per line".into())
                        })?;
                        for field in schema
                            .fields()
                            .iter()
                            .filter(|f| f.data_type().is_integer())
                        {
                            let Some(value) = object.get(field.name()).filter(|v| !v.is_null())
                            else {
                                continue;
                            };
                            let valid = match value {
                                Value::Number(n) => n.is_i64() || n.is_u64(),
                                Value::String(s) => s.parse::<i128>().is_ok(),
                                _ => false,
                            };
                            if !valid {
                                return Err(DataFusionError::Execution(format!(
                                    "NDJSON field {:?} requires an integer token or integer string; refusing lossy conversion",
                                    field.name()
                                )));
                            }
                        }
                        bytes.extend_from_slice(&line);
                        if !line.ends_with(b"\n") {
                            bytes.push(b'\n');
                        }
                        rows += 1;
                    }
                    if rows == 0 {
                        return Ok(());
                    }
                    let decoder = ReaderBuilder::new(schema.clone())
                        .with_batch_size(1024)
                        .build(Cursor::new(bytes))?;
                    for batch in decoder {
                        if sender
                            .blocking_send(batch.map_err(DataFusionError::from))
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
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
