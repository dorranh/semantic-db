//! Immutable, bounded staging. A failed or abandoned source never becomes input.
use crate::{QueryContext, failure, unique_id};
use datafusion::{
    arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
    error::Result,
    physical_plan::SendableRecordBatchStream,
};
use futures::StreamExt;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct StagingOptions {
    pub memory_bytes: usize,
    pub disk_bytes: u64,
}
impl Default for StagingOptions {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            disk_bytes: 1024 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct StagedInput {
    pub schema: SchemaRef,
    pub rows: usize,
    batches: Vec<RecordBatch>,
    files: Vec<PathBuf>,
}
impl Drop for StagedInput {
    fn drop(&mut self) {
        for path in &self.files {
            let _ = std::fs::remove_file(path);
        }
    }
}
impl StagedInput {
    pub async fn collect(
        mut stream: SendableRecordBatchStream,
        options: &StagingOptions,
        context: &Arc<QueryContext>,
    ) -> Result<Self> {
        let mut staged = Self {
            schema: stream.schema(),
            rows: 0,
            batches: vec![],
            files: vec![],
        };
        let mut memory = 0usize;
        let mut disk = 0u64;
        while let Some(batch) = context
            .run(async { stream.next().await.transpose() })
            .await?
        {
            memory = memory.saturating_add(batch.get_array_memory_size());
            staged.rows = staged.rows.saturating_add(batch.num_rows());
            staged.batches.push(batch);
            if memory > options.memory_bytes || !staged.files.is_empty() {
                for batch in std::mem::take(&mut staged.batches) {
                    context.check()?;
                    let path =
                        std::env::temp_dir().join(format!("semantic-stage-{}.arrow", unique_id()));
                    let mut open = OpenOptions::new();
                    open.write(true).create_new(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        open.mode(0o600);
                    }
                    let file = open.open(&path)?;
                    staged.files.push(path);
                    let mut limited = Limited {
                        file,
                        remaining: options.disk_bytes.saturating_sub(disk),
                        written: 0,
                    };
                    let mut writer =
                        arrow_ipc::writer::StreamWriter::try_new(&mut limited, &staged.schema)?;
                    writer.write(&batch)?;
                    writer.finish()?;
                    drop(writer);
                    disk += limited.written;
                }
                memory = 0;
            }
        }
        Ok(staged)
    }
    /// Reopens the exact staged input; does not observe the sources again.
    pub fn batches(&self) -> impl Iterator<Item = Result<RecordBatch>> + '_ {
        self.batches
            .iter()
            .cloned()
            .map(Ok)
            .chain(self.files.iter().flat_map(|path| {
                match File::open(path).map_err(Into::into).and_then(|f| {
                    arrow_ipc::reader::StreamReader::try_new(f, None).map_err(Into::into)
                }) {
                    Ok(reader) => Box::new(reader.map(|b| b.map_err(Into::into)))
                        as Box<dyn Iterator<Item = Result<RecordBatch>> + Send>,
                    Err(e) => Box::new(std::iter::once(Err(e))),
                }
            }))
    }
    pub fn empty() -> Self {
        Self {
            schema: Arc::new(datafusion::arrow::datatypes::Schema::empty()),
            rows: 0,
            batches: vec![],
            files: vec![],
        }
    }
}
struct Limited {
    file: File,
    remaining: u64,
    written: u64,
}
impl Write for Limited {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(std::io::Error::other(failure(
                "staging disk budget exhausted",
            )));
        }
        let n = self.file.write(bytes)?;
        self.remaining -= n as u64;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
