//! Atomic, access-scoped materializations. No hidden background refresh.
use arrow_ipc::writer::StreamWriter;
use datafusion::{
    arrow::{array::RecordBatch, datatypes::SchemaRef},
    catalog::{TableProvider, streaming::StreamingTable},
    error::{DataFusionError, Result},
    execution::TaskContext,
    physical_plan::{
        SendableRecordBatchStream, stream::RecordBatchStreamAdapter, streaming::PartitionStream,
    },
};
use futures::{StreamExt, future::BoxFuture};
use semantic_runtime::{
    QueryContext, SourceDescriptor, failure, fingerprint, ipc::IpcDecoder, unique_id,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheOptions {
    pub directory: PathBuf,
    pub max_memory_bytes: usize,
    pub max_disk_bytes: u64,
}
impl CacheOptions {
    pub fn validate(&self) -> Result<()> {
        if self.directory.as_os_str().is_empty() || self.max_disk_bytes == 0 {
            return Err(failure("cache directory and disk budget required"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializationPolicy {
    pub max_age_seconds: u64,
    pub max_fill_bytes: usize,
}
impl MaterializationPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.max_age_seconds == 0 || self.max_fill_bytes == 0 {
            return Err(failure(
                "materialization age and fill budget must be positive",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub key: String,
    pub generation: String,
    pub schema_revision: String,
    pub acquired_at_ms: u64,
    #[serde(default)]
    pub published_at_ms: u64,
    pub rows: usize,
    pub decoded_bytes: usize,
    pub disk_bytes: u64,
    pub files: usize,
}
#[derive(Debug, Clone)]
pub struct Materialized {
    pub provider: Arc<dyn TableProvider>,
    pub manifest: Manifest,
}
#[derive(Debug)]
struct MemoryEntry {
    batches: Vec<RecordBatch>,
    bytes: usize,
}
#[derive(Debug)]
pub struct MaterializationManager {
    options: CacheOptions,
    memory: Mutex<BTreeMap<String, Arc<MemoryEntry>>>,
}
fn io(error: std::io::Error) -> DataFusionError {
    DataFusionError::IoError(error)
}
fn ensure_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(io)
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn schema_revision(schema: &SchemaRef) -> String {
    fingerprint(&[format!("{schema:?}").as_bytes()])
}
fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
}

impl MaterializationManager {
    /// Creates no files until materialization is requested.
    pub fn new(options: CacheOptions) -> Result<Arc<Self>> {
        options.validate()?;
        Ok(Arc::new(Self {
            options,
            memory: Mutex::new(BTreeMap::new()),
        }))
    }
    pub fn options(&self) -> &CacheOptions {
        &self.options
    }
    pub fn status(&self) -> Result<Vec<Manifest>> {
        if !self.options.directory.exists() {
            return Ok(vec![]);
        }
        let mut manifests = vec![];
        for entry in std::fs::read_dir(&self.options.directory).map_err(io)? {
            let entry = entry.map_err(io)?;
            if let Some(key) = entry.file_name().to_str().filter(|v| valid_component(v))
                && let Some(manifest) = self.current(key)?
            {
                manifests.push(manifest);
            }
        }
        Ok(manifests)
    }
    fn current(&self, key: &str) -> Result<Option<Manifest>> {
        let entry = self.options.directory.join(key);
        let generation = match std::fs::read_to_string(entry.join("CURRENT")) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if !valid_component(&generation) {
            return Err(failure("invalid cache generation"));
        }
        let file = File::open(entry.join(&generation).join("manifest.json")).map_err(io)?;
        let manifest: Manifest = serde_json::from_reader(std::io::Read::take(file, 65536))
            .map_err(|_| failure("invalid cache manifest"))?;
        if manifest.version == 1 {
            return Ok(None);
        }
        if manifest.version != 2 || manifest.key != key || manifest.generation != generation {
            return Err(failure("incompatible cache manifest"));
        }
        Ok(Some(manifest))
    }
    pub async fn invalidate(&self, key: &str, query: &Arc<QueryContext>) -> Result<()> {
        if !valid_component(key) {
            return Err(failure("invalid cache key"));
        }
        let _lock = self
            .lock(
                &self.options.directory.join(key).join("refresh.lock"),
                query,
            )
            .await?;
        let _catalog = self
            .lock(&self.options.directory.join("catalog.lock"), query)
            .await?;
        let path = self.options.directory.join(key).join("CURRENT");
        match std::fs::remove_file(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(io(e)),
        }
        self.memory
            .lock()
            .unwrap()
            .retain(|_, entry| Arc::strong_count(entry) > 1);
        Ok(())
    }
    async fn lock(&self, path: &Path, query: &Arc<QueryContext>) -> Result<File> {
        ensure_directory(path.parent().unwrap())?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(io)?;
        loop {
            query.check()?;
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {
                    query
                        .run(async {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                            Ok(())
                        })
                        .await?
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(io(e)),
            }
        }
    }

    pub async fn resolve(
        &self,
        descriptor: &SourceDescriptor,
        policy: &MaterializationPolicy,
        schema: SchemaRef,
        query: &Arc<QueryContext>,
        oldest_input_ms: Option<u64>,
        fetch: impl FnOnce() -> BoxFuture<'static, Result<SendableRecordBatchStream>>,
    ) -> Result<Materialized> {
        policy.validate()?;
        let key = descriptor.cache_key();
        // A global writer lease makes disk admission and publication atomic across processes.
        // Reads of already published generations continue without this lease.
        if let Some(value) = self.cached(&key, policy, &schema, query).await? {
            query.cache_hit();
            return Ok(value);
        }
        query.cache_miss();
        let _entry_lock = self
            .lock(
                &self.options.directory.join(&key).join("refresh.lock"),
                query,
            )
            .await?;
        if let Some(value) = self.cached(&key, policy, &schema, query).await? {
            query.cache_hit();
            return Ok(value);
        }
        let _writer = self
            .lock(&self.options.directory.join("writer.lock"), query)
            .await?;
        {
            let _catalog = self
                .lock(&self.options.directory.join("catalog.lock"), query)
                .await?;
            self.evict(policy.max_fill_bytes as u64)?;
        }
        let acquired = oldest_input_ms.unwrap_or_else(now_ms).min(now_ms());
        let generation = unique_id();
        let directory = self.options.directory.join(&key).join(&generation);
        ensure_directory(&directory)?;
        let mut unpublished = Unpublished(Some(directory.clone()));
        let mut stream = query.run(fetch()).await?;
        let mut rows = 0usize;
        let mut decoded = 0usize;
        let mut disk = 0u64;
        let mut files = 0usize;
        let mut batches = vec![];
        let mut retain = self.options.max_memory_bytes != 0;
        while let Some(batch) = query.run(async { stream.next().await.transpose() }).await? {
            if batch.schema() != schema {
                return Err(failure("materialization schema changed"));
            }
            decoded = decoded
                .checked_add(batch.get_array_memory_size())
                .filter(|n| *n <= policy.max_fill_bytes)
                .ok_or_else(|| failure("materialization fill budget exhausted"))?;
            rows = rows
                .checked_add(batch.num_rows())
                .ok_or_else(|| failure("materialization row count overflow"))?;
            let path = directory.join(format!("{files:08}.arrow"));
            let file = File::create(&path).map_err(io)?;
            let mut writer = StreamWriter::try_new(file, &schema)?;
            writer.write(&batch)?;
            writer.finish()?;
            writer.get_ref().sync_all().map_err(io)?;
            disk = disk.saturating_add(writer.get_ref().metadata().map_err(io)?.len());
            if disk > self.options.max_disk_bytes || disk > policy.max_fill_bytes as u64 {
                return Err(failure("materialization disk budget exhausted"));
            }
            files += 1;
            if retain && !self.admit_memory(decoded) {
                retain = false;
                batches.clear();
            }
            if retain {
                batches.push(batch);
            }
        }
        query.check()?;
        let manifest = Manifest {
            version: 2,
            key: key.clone(),
            generation,
            schema_revision: schema_revision(&schema),
            acquired_at_ms: acquired,
            published_at_ms: now_ms(),
            rows,
            decoded_bytes: decoded,
            disk_bytes: disk,
            files,
        };
        let file = File::create(directory.join("manifest.json")).map_err(io)?;
        serde_json::to_writer(&file, &manifest)
            .map_err(|_| failure("cache manifest write failed"))?;
        file.sync_all().map_err(io)?;
        File::open(&directory)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        let _catalog = self
            .lock(&self.options.directory.join("catalog.lock"), query)
            .await?;
        let pointer = self
            .options
            .directory
            .join(&key)
            .join(format!("{}.current", unique_id()));
        std::fs::write(&pointer, &manifest.generation).map_err(io)?;
        File::open(&pointer)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        std::fs::rename(pointer, self.options.directory.join(&key).join("CURRENT")).map_err(io)?;
        File::open(self.options.directory.join(&key))
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        unpublished.0 = None;
        if retain {
            self.memory.lock().unwrap().insert(
                manifest.generation.clone(),
                Arc::new(MemoryEntry {
                    batches,
                    bytes: decoded,
                }),
            );
        }
        self.provider(manifest, schema)
    }
    async fn cached(
        &self,
        key: &str,
        policy: &MaterializationPolicy,
        schema: &SchemaRef,
        query: &Arc<QueryContext>,
    ) -> Result<Option<Materialized>> {
        let _catalog = self
            .lock(&self.options.directory.join("catalog.lock"), query)
            .await?;
        self.fresh(key, policy, schema, query.options.max_cache_age_ms)?
            .map(|manifest| self.provider(manifest, schema.clone()))
            .transpose()
    }
    fn admit_memory(&self, requested: usize) -> bool {
        let mut memory = self.memory.lock().unwrap();
        while memory
            .values()
            .map(|entry| entry.bytes)
            .sum::<usize>()
            .saturating_add(requested)
            > self.options.max_memory_bytes
        {
            let removable = memory
                .iter()
                .find(|(_, entry)| Arc::strong_count(entry) == 1)
                .map(|(key, _)| key.clone());
            let Some(key) = removable else {
                return false;
            };
            memory.remove(&key);
        }
        true
    }
    fn fresh(
        &self,
        key: &str,
        policy: &MaterializationPolicy,
        schema: &SchemaRef,
        max_age_ms: Option<u64>,
    ) -> Result<Option<Manifest>> {
        Ok(self.current(key)?.filter(|m| {
            m.decoded_bytes <= policy.max_fill_bytes
                && m.disk_bytes <= policy.max_fill_bytes as u64
                && m.schema_revision == schema_revision(schema)
                && m.published_at_ms <= now_ms()
                && now_ms() - m.published_at_ms
                    <= max_age_ms
                        .unwrap_or(u64::MAX)
                        .min(policy.max_age_seconds.saturating_mul(1000))
        }))
    }
    fn provider(&self, manifest: Manifest, schema: SchemaRef) -> Result<Materialized> {
        if let Some(entry) = self.memory.lock().unwrap().get(&manifest.generation) {
            let partition = Arc::new(MemoryPartition {
                entry: entry.clone(),
                schema: schema.clone(),
            });
            return Ok(Materialized {
                provider: Arc::new(StreamingTable::try_new(schema, vec![partition])?),
                manifest,
            });
        }
        let directory = self
            .options
            .directory
            .join(&manifest.key)
            .join(&manifest.generation);
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("lease"))
            .map_err(io)?;
        lease.lock_shared().map_err(io)?;
        let partition = Arc::new(DiskPartition {
            directory,
            schema: schema.clone(),
            manifest: manifest.clone(),
            _lease: Arc::new(lease),
        });
        Ok(Materialized {
            provider: Arc::new(StreamingTable::try_new(schema, vec![partition])?),
            manifest,
        })
    }
    fn evict(&self, requested: u64) -> Result<()> {
        if requested > self.options.max_disk_bytes {
            return Err(failure("fill budget exceeds total disk budget"));
        }
        let mut generations = vec![];
        let mut total = 0u64;
        for entry in std::fs::read_dir(&self.options.directory).map_err(io)? {
            let entry = entry.map_err(io)?;
            if !entry.path().is_dir() {
                continue;
            }
            for generation in std::fs::read_dir(entry.path()).map_err(io)? {
                let generation = generation.map_err(io)?;
                if !generation.path().is_dir() {
                    continue;
                }
                let manifest_path = generation.path().join("manifest.json");
                if let Ok(file) = File::open(&manifest_path) {
                    if let Ok(manifest) =
                        serde_json::from_reader::<_, Manifest>(std::io::Read::take(file, 65536))
                    {
                        total = total.saturating_add(manifest.disk_bytes);
                        generations.push((manifest.acquired_at_ms, generation.path(), manifest));
                    }
                } else {
                    // All fills hold writer.lock, so uncommitted directories here are abandoned.
                    std::fs::remove_dir_all(generation.path()).map_err(io)?;
                }
            }
        }
        generations.sort_by_key(|(age, _, _)| *age);
        for (_, path, manifest) in generations {
            if total.saturating_add(requested) <= self.options.max_disk_bytes {
                break;
            }
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path.join("lease"))
                .map_err(io)?;
            if file.try_lock().is_err() {
                continue;
            }
            let current = path.parent().unwrap().join("CURRENT");
            if std::fs::read_to_string(&current).is_ok_and(|v| v == manifest.generation) {
                std::fs::remove_file(current).map_err(io)?;
            }
            std::fs::remove_dir_all(&path).map_err(io)?;
            total = total.saturating_sub(manifest.disk_bytes);
            self.memory.lock().unwrap().retain(|generation, entry| {
                generation != &manifest.generation || Arc::strong_count(entry) > 1
            });
        }
        if total.saturating_add(requested) > self.options.max_disk_bytes {
            return Err(failure(
                "cache disk budget exhausted; active generations are pinned",
            ));
        }
        Ok(())
    }
}
struct Unpublished(Option<PathBuf>);
impl Drop for Unpublished {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
#[derive(Debug)]
struct DiskPartition {
    directory: PathBuf,
    schema: SchemaRef,
    manifest: Manifest,
    _lease: Arc<File>,
}
impl PartitionStream for DiskPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, task: Arc<TaskContext>) -> SendableRecordBatchStream {
        let directory = self.directory.clone();
        let manifest = self.manifest.clone();
        let schema = self.schema.clone();
        let lease = self._lease.clone();
        let stream = async_stream::try_stream! {
            use tokio::io::AsyncReadExt;
            let _lease = lease;
            let reservation = datafusion::execution::memory_pool::MemoryConsumer::new("materialization IPC").register(task.memory_pool());
            for i in 0..manifest.files {
                let mut file = tokio::fs::File::open(directory.join(format!("{i:08}.arrow"))).await.map_err(io)?;
                let mut decoder = IpcDecoder::new(manifest.decoded_bytes.saturating_add(4 * 1024 * 1024));
                let mut bytes = vec![0u8; 65536];
                loop {
                    if let Some(query) = QueryContext::from_task(&task) { query.check()?; }
                    let n = file.read(&mut bytes).await.map_err(io)?;
                    if n == 0 { break; }
                    reservation.try_resize(decoder.buffered_bytes().saturating_add(n).saturating_mul(2).saturating_add(decoder.retained_bytes().saturating_mul(2)))?;
                    decoder.push(&bytes[..n])?;
                    while let Some(batch) = decoder.next_batch_admitted(|size,retained| {
                        if let Some(query) = QueryContext::from_task(&task) { query.charge_decoded(size)?; }
                        reservation.try_resize(size.saturating_mul(4).saturating_add(retained.saturating_mul(2)).saturating_add(semantic_runtime::ipc::DECODER_SCRATCH_BYTES))
                    })? {
                        if batch.schema() != schema { Err(failure("cached schema mismatch"))?; }
                        yield batch;
                    }
                }
                if decoder.finish()? != schema { Err(failure("cached schema mismatch"))?; }
            }
        };
        Box::pin(RecordBatchStreamAdapter::new(self.schema.clone(), stream))
    }
}

#[derive(Debug)]
struct MemoryPartition {
    entry: Arc<MemoryEntry>,
    schema: SchemaRef,
}
impl PartitionStream for MemoryPartition {
    fn schema(&self) -> &SchemaRef {
        &self.schema
    }
    fn execute(&self, task: Arc<TaskContext>) -> SendableRecordBatchStream {
        let entry = self.entry.clone();
        let stream = async_stream::try_stream! {
            for batch in &entry.batches {
                if let Some(query) = QueryContext::from_task(&task) { query.charge_decoded(batch.get_array_memory_size())?; }
                yield batch.clone();
            }
        };
        Box::pin(RecordBatchStreamAdapter::new(self.schema.clone(), stream))
    }
}
