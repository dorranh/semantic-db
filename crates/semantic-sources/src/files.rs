//! Local DataFusion readers. Model guidance changes parsing, never casts parsed data.
mod json;
use super::*;
use datafusion::common::GetExt;
use datafusion::execution::options::ArrowReadOptions;
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema, TimeUnit},
    datasource::file_format::file_compression_type::FileCompressionType,
    prelude::{
        AvroReadOptions, CsvReadOptions, JsonReadOptions, ParquetReadOptions, SessionContext,
    },
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileFormat {
    Csv,
    Json,
    Parquet,
    Avro,
    Arrow,
}

/// Use None for extension-based format selection; explicit formats also support directories.
pub struct FileConnector {
    format: Option<FileFormat>,
}
impl FileConnector {
    pub fn new(format: Option<FileFormat>) -> Self {
        Self { format }
    }
}
/// Compatibility with existing applications registering the CSV factory directly.
pub struct CsvConnector;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionOptions {}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FileOptions {
    path: PathBuf,
    #[serde(default)]
    format: Option<FileFormat>,
    #[serde(default)]
    compression: Option<String>,
    #[serde(default)]
    extension: Option<String>,
    #[serde(default)]
    has_header: Option<bool>,
    #[serde(default)]
    delimiter: Option<String>,
    #[serde(default)]
    quote: Option<String>,
    #[serde(default)]
    escape: Option<String>,
    #[serde(default)]
    null_regex: Option<String>,
    #[serde(default)]
    schema_infer_max_records: Option<usize>,
    /// Arrow serde types, only needed for physical details absent from Ossie (e.g. decimal scale).
    #[serde(default)]
    physical_types: BTreeMap<String, DataType>,
}

fn err(message: impl Into<String>) -> SourceError {
    SourceError::configuration("file_options", "/", message)
}
fn parse(options: &Options) -> Result<FileOptions> {
    super::builtin::options(options)
}
fn byte(value: &Option<String>, default: u8) -> Result<u8> {
    match value {
        None => Ok(default),
        Some(s) if s.len() == 1 && s.is_ascii() => Ok(s.as_bytes()[0]),
        _ => Err(err(
            "CSV delimiter, quote, and escape must be one ASCII character",
        )),
    }
}

impl FileOptions {
    fn resolved(
        &self,
        fixed: Option<FileFormat>,
    ) -> Result<(FileFormat, FileCompressionType, String)> {
        if self.path.as_os_str().is_empty() {
            let kind = if fixed == Some(FileFormat::Csv) {
                "CSV"
            } else {
                "file"
            };
            return Err(err(format!("{kind} path must be nonempty")));
        }
        let path = self
            .path
            .to_str()
            .ok_or_else(|| err("file path must be UTF-8"))?;
        if path.contains("://") {
            return Err(err(
                "only local files are supported; remote URLs require a separate storage connector",
            ));
        }
        let name = self.path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let lower = name.to_ascii_lowercase();
        let (stem, inferred_compression, _suffix) = [
            (".gz", "gzip"),
            (".bz2", "bzip2"),
            (".xz", "xz"),
            (".zst", "zstd"),
            (".zstd", "zstd"),
        ]
        .iter()
        .find_map(|(ext, codec)| lower.strip_suffix(ext).map(|s| (s, *codec, *ext)))
        .unwrap_or((&lower, "uncompressed", ""));
        let inferred_format = stem.rsplit_once('.').and_then(|(_, ext)| match ext {
            "csv" | "tsv" => Some(FileFormat::Csv),
            "json" | "jsonl" | "ndjson" => Some(FileFormat::Json),
            "parquet" => Some(FileFormat::Parquet),
            "avro" => Some(FileFormat::Avro),
            "arrow" | "ipc" => Some(FileFormat::Arrow),
            _ => None,
        });
        if fixed.zip(self.format).is_some_and(|(a, b)| a != b) {
            return Err(err("format conflicts with the selected connector"));
        }
        let format = fixed.or(self.format).or(inferred_format).ok_or_else(|| {
            err("cannot infer file format; set format for directories or extensionless files")
        })?;
        let compression = self
            .compression
            .as_deref()
            .unwrap_or(inferred_compression)
            .parse::<FileCompressionType>()?;
        if !matches!(format, FileFormat::Csv | FileFormat::Json)
            && compression != FileCompressionType::UNCOMPRESSED
        {
            return Err(err(
                "whole-file compression is supported only for CSV and NDJSON; other formats use internal codecs",
            ));
        }
        if !matches!(format, FileFormat::Csv | FileFormat::Json)
            && (!self.physical_types.is_empty() || self.schema_infer_max_records.is_some())
        {
            return Err(err(
                "physical_types and schema_infer_max_records apply only to CSV and NDJSON; embedded schemas are preserved",
            ));
        }
        if format != FileFormat::Csv
            && (self.has_header.is_some()
                || self.delimiter.is_some()
                || self.quote.is_some()
                || self.escape.is_some()
                || self.null_regex.is_some())
        {
            return Err(err("CSV parsing options require a CSV source"));
        }
        byte(&self.delimiter, b',')?;
        byte(&self.quote, b'"')?;
        byte(&self.escape, b'\\')?;
        if self.schema_infer_max_records == Some(0) {
            return Err(err("schema_infer_max_records must be positive"));
        }
        let default_ext = match format {
            FileFormat::Csv => ".csv",
            FileFormat::Json => ".json",
            FileFormat::Parquet => ".parquet",
            FileFormat::Avro => ".avro",
            FileFormat::Arrow => ".arrow",
        };
        let extension = self.extension.clone().unwrap_or_else(|| {
            // Preserve filename casing and aliases such as .jsonl. Directories normally use format defaults.
            if inferred_format == Some(format) {
                let start = stem.rfind('.').unwrap();
                name[start..].into()
            } else {
                format!("{default_ext}{}", compression.get_ext())
            }
        });
        if extension.is_empty()
            || !extension.starts_with('.')
            || extension.contains('/')
            || extension.contains('\\')
        {
            return Err(err(
                "extension must be a nonempty filename suffix starting with '.'",
            ));
        }
        Ok((format, compression, extension))
    }
}

fn prepare(
    options: &Options,
    model: &ModelInspection,
    source: &str,
    fixed: Option<FileFormat>,
) -> Result<Options> {
    let mut options = parse(options)?;
    let (format, _, _) = options.resolved(fixed)?;
    if !matches!(format, FileFormat::Csv | FileFormat::Json) {
        return Ok(serde_json::to_value(options)
            .unwrap()
            .as_object()
            .unwrap()
            .clone());
    }
    let mut declarations: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for dataset in model.datasets.iter().filter(|d| d.source == source) {
        for field in &dataset.fields {
            let Some(logical) = field.datatype.as_deref() else {
                continue;
            };
            if let Some((previous, location)) =
                declarations.insert(&field.source_column, (logical, &dataset.path))
                && previous != logical
            {
                return Err(err(format!(
                    "conflicting Ossie types for {}: {previous} at {location}, {logical} at {}",
                    field.source_column, dataset.path
                )));
            }
            if let Some(physical) = options.physical_types.get(&field.source_column) {
                if !compatible(logical, physical) {
                    return Err(err(format!(
                        "physical_types for {} conflicts with Ossie {logical}",
                        field.source_column
                    )));
                }
                continue;
            }
            let datatype = match logical {
                "String" => DataType::Utf8,
                "Integer" => DataType::Int64,
                "Float" => DataType::Float64,
                "Boolean" => DataType::Boolean,
                "Date" => DataType::Date32,
                "Time" => DataType::Time64(TimeUnit::Nanosecond),
                "DateTime" => DataType::Timestamp(TimeUnit::Nanosecond, None),
                "DateTimeTz" => DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
                "Decimal" => {
                    return Err(err(format!(
                        "Ossie Decimal does not specify precision/scale; provide physical_types for {} (e.g. {{Decimal128: [18, 2]}})",
                        field.source_column
                    )));
                }
                other => {
                    return Err(err(format!(
                        "no file parsing mapping for Ossie type {other}"
                    )));
                }
            };
            options
                .physical_types
                .insert(field.source_column.clone(), datatype);
        }
    }
    Ok(serde_json::to_value(options)
        .unwrap()
        .as_object()
        .unwrap()
        .clone())
}
fn compatible(logical: &str, physical: &DataType) -> bool {
    use DataType::*;
    match logical {
        "String" => matches!(physical, Utf8 | LargeUtf8 | Utf8View),
        "Integer" => matches!(
            physical,
            Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64
        ),
        "Float" => matches!(physical, Float16 | Float32 | Float64),
        "Decimal" => matches!(
            physical,
            Decimal32(..) | Decimal64(..) | Decimal128(..) | Decimal256(..)
        ),
        "Boolean" => matches!(physical, Boolean),
        "Date" => matches!(physical, Date32 | Date64),
        "Time" => matches!(physical, Time32(_) | Time64(_)),
        "DateTime" => matches!(physical, Timestamp(_, None)),
        "DateTimeTz" => matches!(physical, Timestamp(_, Some(_))),
        _ => false,
    }
}

impl ConnectorFactory for FileConnector {
    fn prepare_source(
        &self,
        options: &Options,
        model: &ModelInspection,
        source: &str,
    ) -> Result<Options> {
        prepare(options, model, source, self.format)
    }
    fn validate_connection(&self, options: &Options) -> Result<()> {
        super::builtin::options::<ConnectionOptions>(options).map(|_| ())
    }
    fn validate_source(&self, options: &Options) -> Result<()> {
        parse(options)?.resolved(self.format).map(|_| ())
    }
    fn connect<'a>(
        &'a self,
        _: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(
            async move { Ok(Arc::new(FileConnection(self.format)) as Arc<dyn SourceConnection>) },
        )
    }
}
impl ConnectorFactory for CsvConnector {
    fn prepare_source(
        &self,
        options: &Options,
        model: &ModelInspection,
        source: &str,
    ) -> Result<Options> {
        prepare(options, model, source, Some(FileFormat::Csv))
    }
    fn validate_connection(&self, options: &Options) -> Result<()> {
        super::builtin::options::<ConnectionOptions>(options).map(|_| ())
    }
    fn validate_source(&self, options: &Options) -> Result<()> {
        parse(options)?.resolved(Some(FileFormat::Csv)).map(|_| ())
    }
    fn connect<'a>(
        &'a self,
        _: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async {
            Ok(Arc::new(FileConnection(Some(FileFormat::Csv))) as Arc<dyn SourceConnection>)
        })
    }
}

struct FileConnection(Option<FileFormat>);
impl SourceConnection for FileConnection {
    fn authorization_scope(&self) -> Option<String> {
        Some("local-files".into())
    }
    fn table<'a>(
        &'a self,
        options: &'a Options,
        base_dir: &'a Path,
    ) -> BoxFuture<'a, Result<Arc<dyn TableProvider>>> {
        Box::pin(async move {
            let options = parse(options)?;
            let (format, compression, mut extension) = options.resolved(self.0)?;
            let path = base_dir.join(&options.path);
            // A single explicitly typed file may have no conventional suffix.
            if options.extension.is_none()
                && path.is_file()
                && !path.to_string_lossy().ends_with(&extension)
            {
                extension.clear();
            }
            let path = path
                .to_str()
                .ok_or_else(|| err("file path must be UTF-8"))?;
            let ctx = SessionContext::new();
            let mut csv = CsvReadOptions::new();
            csv.has_header = options.has_header.unwrap_or(true);
            csv.delimiter = byte(
                &options.delimiter,
                if extension.to_ascii_lowercase().starts_with(".tsv") {
                    b'\t'
                } else {
                    b','
                },
            )?;
            csv.quote = byte(&options.quote, b'"')?;
            csv.escape = options.escape.as_ref().map(|s| s.as_bytes()[0]);
            csv.null_regex = options.null_regex.clone();
            csv.file_extension = &extension;
            csv.file_compression_type = compression;
            csv.schema_infer_max_records = options.schema_infer_max_records.unwrap_or(1000);
            let json = JsonReadOptions::default()
                .file_extension(&extension)
                .file_compression_type(compression)
                .schema_infer_max_records(options.schema_infer_max_records.unwrap_or(1000));
            let inferred = match format {
                FileFormat::Csv => ctx.read_csv(path, csv.clone()).await?,
                FileFormat::Json => ctx.read_json(path, json.clone()).await?,
                FileFormat::Parquet => {
                    ctx.read_parquet(
                        path,
                        ParquetReadOptions::default().file_extension(&extension),
                    )
                    .await?
                }
                FileFormat::Avro => {
                    ctx.read_avro(
                        path,
                        AvroReadOptions {
                            file_extension: &extension,
                            ..Default::default()
                        },
                    )
                    .await?
                }
                FileFormat::Arrow => {
                    ctx.read_arrow(
                        path,
                        ArrowReadOptions {
                            file_extension: &extension,
                            ..Default::default()
                        },
                    )
                    .await?
                }
            };
            if options.physical_types.is_empty() && format != FileFormat::Json {
                return Ok(inferred.into_view());
            }
            let schema = inferred.schema().as_arrow();
            for name in options.physical_types.keys() {
                if schema.field_with_name(name).is_err() {
                    return Err(err(format!(
                        "declared physical column {name:?} is missing from the file schema"
                    )));
                }
            }
            let fields = schema
                .fields()
                .iter()
                .map(|field| match options.physical_types.get(field.name()) {
                    Some(datatype) => {
                        Field::new(field.name(), datatype.clone(), field.is_nullable())
                            .with_metadata(field.metadata().clone())
                    }
                    None => field.as_ref().clone(),
                })
                .collect::<Vec<_>>();
            let schema = Schema::new_with_metadata(fields, schema.metadata().clone());
            // Reopen with the guided schema: no values are taken from the inferred frame.
            let frame = match format {
                FileFormat::Csv => ctx.read_csv(path, csv.schema(&schema)).await?,
                FileFormat::Json => {
                    return Ok(json::table(
                        Path::new(path),
                        &extension,
                        Arc::new(schema),
                        compression,
                    )?);
                }
                _ => unreachable!("physical overrides rejected for embedded schemas"),
            };
            Ok(frame.into_view())
        })
    }
}
