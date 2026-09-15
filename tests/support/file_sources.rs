//! Shared generated fixtures exercised through the loader and both binary frontends.
#![allow(dead_code)]
use datafusion::arrow::{
    array::{Int64Array, StringArray},
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub struct Files(pub PathBuf);
impl Files {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "semantic-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        let fixture = Self(path);
        fixture.write("items.csv", "CODE,qty\n00123,2\n00456,3\n");
        fixture.write(
            "items.jsonl",
            "{\"CODE\":\"00123\",\"qty\":2}\n{\"CODE\":\"00456\",\"qty\":3}\n",
        );
        let schema = Arc::new(Schema::new(vec![
            Field::new("CODE", DataType::Utf8, true),
            Field::new("qty", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec!["00123", "00456"])),
                Arc::new(Int64Array::from(vec![2, 3])),
            ],
        )
        .unwrap();
        let mut parquet = datafusion::parquet::arrow::ArrowWriter::try_new(
            fs::File::create(fixture.0.join("items.parquet")).unwrap(),
            schema.clone(),
            None,
        )
        .unwrap();
        parquet.write(&batch).unwrap();
        parquet.close().unwrap();
        let mut arrow = datafusion::arrow::ipc::writer::FileWriter::try_new(
            fs::File::create(fixture.0.join("items.arrow")).unwrap(),
            &schema,
        )
        .unwrap();
        arrow.write(&batch).unwrap();
        arrow.finish().unwrap();
        let mut avro = arrow_avro::writer::AvroWriter::new(
            fs::File::create(fixture.0.join("items.avro")).unwrap(),
            schema.as_ref().clone(),
        )
        .unwrap();
        avro.write(&batch).unwrap();
        avro.finish().unwrap();
        for name in ["items.csv", "items.jsonl"] {
            let mut gzip = flate2::write::GzEncoder::new(
                fs::File::create(fixture.0.join(format!("{name}.gz"))).unwrap(),
                flate2::Compression::default(),
            );
            gzip.write_all(&fs::read(fixture.0.join(name)).unwrap())
                .unwrap();
            gzip.finish().unwrap();
        }
        fixture.model(Self::datasets());
        fixture
    }
    pub fn datasets() -> Value {
        json!([{"name":"products","source":"local.products","fields":[
            {"name":"code","datatype":"String","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"\"CODE\""}]}},
            {"name":"qty","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"qty"}]}}
        ]}])
    }
    pub fn model(&self, datasets: Value) {
        self.write(
            "model.json",
            &json!({"version":"0.2.0.dev0","semantic_model":[{"name":"test","datasets":datasets}]})
                .to_string(),
        );
    }
    pub fn write(&self, name: &str, text: &str) {
        fs::write(self.0.join(name), text).unwrap();
    }
    pub fn config(&self, source: Value) -> PathBuf {
        self.config_connector("file", source)
    }
    pub fn config_connector(&self, connector: &str, source: Value) -> PathBuf {
        let mut source = source;
        source["connection"] = json!("local");
        self.write("project.json", &json!({"ossie":"model.json","connections":{"local":{"connector":connector}},"sources":{"local.products":source},"views":{"selected_products":{"sql_file":"selected.sql","description":"Products with at least two units"}}}).to_string());
        self.write(
            "selected.sql",
            "SELECT code, qty FROM products WHERE qty >= 2",
        );
        self.0.join("project.json")
    }
    pub const FORMATS: [&'static str; 7] = [
        "items.csv",
        "items.jsonl",
        "items.parquet",
        "items.avro",
        "items.arrow",
        "items.csv.gz",
        "items.jsonl.gz",
    ];
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
