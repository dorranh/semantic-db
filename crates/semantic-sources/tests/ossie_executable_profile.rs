use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use arrow_schema::{DataType, Field, Schema};
use futures::future::BoxFuture;
use semantic_catalog::FactResolution;
use semantic_ossie::OssieDocument;
use semantic_sources::{
    ConnectorFactory, Options, Project, Registry, Result, SecretResolver, SourceConnection,
    SourceError,
};
use serde_json::{Value, json};

struct OfflineFactory(Arc<AtomicUsize>);
impl ConnectorFactory for OfflineFactory {
    fn validate_connection(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn validate_source(&self, _: &Options) -> Result<()> {
        Ok(())
    }
    fn connect<'a>(
        &'a self,
        _: &'a Options,
        _: &'a SecretResolver<'_>,
    ) -> BoxFuture<'a, Result<Arc<dyn SourceConnection>>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(SourceError::configuration(
                "unexpected_connect",
                "/connections",
                "offline import opened a connection",
            ))
        })
    }
}

fn extension(data: Value) -> Value {
    json!({"vendor_name":"SEMANTIC_DB","data":data.to_string()})
}

fn model() -> Value {
    json!({"version":"0.2.0.dev0","semantic_model":[{
        "name":"shop", "datasets":[{"name":"items","source":"fixture.items","fields":[
            {"name":"id","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"id"}]}},
            {"name":"amount","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"amount"}]}},
            {"name":"state","datatype":"String","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":"state"}]}}
        ]}],
        "metrics":[{"name":"total_amount","datatype":"Integer","expression":{"dialects":[{"dialect":"ANSI_SQL","expression":" SUM(amount) "}]},
            "custom_extensions":[extension(json!({"kind":"metric","dataset":"items","source_grain":{"entity":null,"keys":[{"relation":"items","field":"id"}]},"dimensions":["state"],"unit":{"kind":"currency","code":"USD"},"empty":"null"}))]}],
        "custom_extensions":[extension(json!({"kind":"concept","dataset":"items","name":"active","description":"Open items",
            "predicate":{"kind":"compare","field":"state","operator":"eq","value":{"type":"utf8","value":"open"}}}))]
    }]})
}

fn project(value: Value) -> Project {
    let schema = Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
    ]);
    let config = json!({
        "ossie":"unused.yaml",
        "connections":{"fixture":{"connector":"offline"}},
        "sources":{"fixture.items":{"connection":"fixture","recorded_schema":schema}}
    });
    Project::new(
        serde_json::from_value(config).unwrap(),
        OssieDocument::parse(&value.to_string()).unwrap(),
        PathBuf::from("."),
    )
    .unwrap()
}

fn registry() -> (Arc<Registry>, Arc<AtomicUsize>) {
    let connects = Arc::new(AtomicUsize::new(0));
    let mut registry = Registry::new();
    registry
        .register("offline", OfflineFactory(connects.clone()))
        .unwrap();
    (Arc::new(registry), connects)
}

#[tokio::test]
async fn configured_deferred_import_preserves_executable_profile_without_connecting() {
    let (registry, connects) = registry();
    let loaded = project(model())
        .load_deferred_read_only(
            registry,
            Arc::new(|_| None),
            semantic_engine::DeferredOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(connects.load(Ordering::SeqCst), 0);
    assert_eq!(loaded.provider_report().resolutions, 0);
    let relation = loaded.engine.catalog().relation("items").unwrap();
    let semantics = relation.semantics.as_ref().unwrap();
    assert_eq!(
        semantics.metrics["total_amount"].source_grain,
        semantic_catalog::SourceGrain {
            entity: None,
            keys: vec![semantic_catalog::GrainKey {
                relation: "items".into(),
                field: "id".into(),
            }],
        }
    );
    assert_eq!(semantics.concepts["active"].description, "Open items");
    let FactResolution::Known {
        value,
        contributors,
    } = &semantics.facts["metric_expression/total_amount"]
    else {
        panic!("authored expression fact missing");
    };
    assert_eq!(value["expression"], " SUM(amount) ");
    assert!(contributors[0].origins.iter().any(|origin| {
        origin
            .path
            .ends_with("/metrics/0/expression/dialects/0/expression")
    }));
}

#[tokio::test]
async fn unknown_vendor_extension_rejects_before_source_resolution() {
    let mut value = model();
    value["semantic_model"][0]["metrics"][0]["custom_extensions"][0]["vendor_name"] =
        json!("OTHER");
    let (registry, connects) = registry();
    let error = project(value)
        .load_deferred_read_only(
            registry,
            Arc::new(|_| None),
            semantic_engine::DeferredOptions::default(),
        )
        .await
        .err()
        .expect("unknown vendor must fail");
    assert!(error.to_string().contains("unsupported_feature"), "{error}");
    assert_eq!(connects.load(Ordering::SeqCst), 0);
}
