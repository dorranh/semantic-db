use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Manifest contract for artifact format version 1.
pub struct Manifest {
    pub format_version: u32,
    pub id: String,
    pub version: String,
    pub project: PathBuf,
    pub cases: PathBuf,
    pub required_paired_cases: usize,
    #[serde(default)]
    pub required_companion_cases: usize,
    #[serde(default)]
    pub public_cases: Vec<String>,
    pub context: Context,
    #[serde(default)]
    pub execution: Option<ExecutionLimits>,
    #[serde(default)]
    pub environment: Option<Environment>,
    #[serde(default)]
    pub fixtures: Vec<Fixture>,
    pub schemas: Option<PathBuf>,
    pub canonical_data: Option<PathBuf>,
}
/// Optional artifact execution capacity; output collection limits remain run-owned.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    pub max_requests: Option<usize>,
    pub max_decoded_bytes: Option<usize>,
    pub max_remote_bytes: Option<usize>,
}
impl ExecutionLimits {
    pub(crate) fn apply(&self, budgets: &mut semantic_engine::QueryOptions) -> Result<()> {
        if self.max_requests.is_some_and(|n| n == 0 || n > 1_000_000)
            || [self.max_decoded_bytes, self.max_remote_bytes]
                .into_iter()
                .flatten()
                .any(|n| n == 0 || n as u128 > (1u128 << 40))
        {
            return Err("execution capacity must be positive, at most 1000000 requests and 1 TiB per byte limit".into());
        }
        if let Some(n) = self.max_requests {
            budgets.max_remote_requests = n;
        }
        if let Some(n) = self.max_decoded_bytes {
            budgets.max_decoded_bytes = n;
        }
        if let Some(n) = self.max_remote_bytes {
            budgets.max_remote_bytes = n;
        }
        budgets.validate()?;
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Context contract for artifact format version 1.
pub struct Context {
    pub reference_time: String,
    pub timezone: String,
    #[serde(default)]
    pub allowed_relations: Option<BTreeSet<String>>,
    #[serde(default = "gregorian")]
    pub calendar: String,
}
fn gregorian() -> String {
    "gregorian".into()
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Environment contract for artifact format version 1.
pub struct Environment {
    pub compose_files: Vec<PathBuf>,
    pub services: Vec<String>,
    pub bootstrap_service: Option<String>,
    pub startup_timeout_seconds: u64,
    pub bootstrap_timeout_seconds: u64,
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Binding contract for artifact format version 1.
pub struct Binding {
    pub service: String,
    pub port: u16,
    pub template: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Fixture contract for artifact format version 1.
pub struct Fixture {
    pub sql: String,
    pub expected: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Case contract for artifact format version 1.
pub struct Case {
    pub id: String,
    pub question: String,
    pub sql: Option<String>,
    pub expected: PathBuf,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub requirements: Vec<String>,
    #[serde(default)]
    pub comparison: Comparison,
    pub context: Option<Context>,
    pub primary_group: Option<String>,
    pub oracle_reasoning: Option<String>,
    #[serde(default)]
    pub distinguishing_rows: Vec<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Comparison contract for artifact format version 1.
pub struct Comparison {
    #[serde(default)]
    pub ordered: bool,
    #[serde(default)]
    pub assert_names: bool,
    #[serde(default)]
    pub assert_physical_types: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
/// Authored expected result or semantic outcome.
pub enum Expected {
    Result {
        columns: Vec<Column>,
        rows: Vec<Vec<Value>>,
    },
    NeedsClarification {
        diagnostic_contains: Option<String>,
    },
    Unsupported {
        diagnostic_contains: Option<String>,
    },
    Rejected {
        diagnostic_contains: Option<String>,
    },
    ExecutionError {
        diagnostic_contains: Option<String>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Column contract for artifact format version 1.
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub tolerance: Option<Tolerance>,
    #[serde(default = "yes")]
    pub nullable: bool,
    /// Optional assertion on provider schema metadata; nullable describes data values.
    pub physical_nullable: Option<bool>,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
/// Tolerance contract for artifact format version 1.
pub struct Tolerance {
    pub absolute: f64,
    pub relative: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
/// Complete schema and losslessly encoded rows, independent of Arrow batch boundaries.
pub struct TypedResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}
/// A validated standalone artifact bundle with manifest-relative paths.
pub struct Dataset {
    pub manifest: Manifest,
    pub root: PathBuf,
    pub cases: Vec<Case>,
    pub expectations: BTreeMap<String, Expected>,
    pub digest: String,
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
impl Dataset {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = std::fs::canonicalize(path)?;
        let root = path.parent().ok_or("manifest has no parent")?.to_owned();
        let manifest: Manifest = read(&path)?;
        if let Some(limits) = &manifest.execution {
            limits.apply(&mut semantic_engine::QueryOptions::default())?;
        }
        if manifest.format_version != 1
            || !valid_id(&manifest.id)
            || manifest.version.trim().is_empty()
        {
            return Err("invalid manifest version, identity or required count".into());
        }
        validate_context(&manifest.context)?;
        let cases: Vec<Case> = read(&resolve(&root, &manifest.cases)?)?;
        let mut expectations = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for c in &cases {
            if !valid_id(&c.id)
                || !ids.insert(c.id.clone())
                || c.question.trim().is_empty()
                || c.sql.as_ref().is_some_and(|s| s.trim().is_empty())
            {
                return Err(format!("invalid or duplicate case {}", c.id).into());
            }
            if let Some(ctx) = &c.context {
                validate_context(ctx)?;
            }
            let e: Expected = read(&resolve(&root, &c.expected)?)?;
            validate_expected(&e)?;
            expectations.insert(c.id.clone(), e);
        }
        if cases.is_empty()
            || cases
                .iter()
                .filter(|c| !matches!(expectations[&c.id], Expected::Result { .. }))
                .count()
                != manifest.required_companion_cases
            || cases
                .iter()
                .filter(|c| {
                    c.sql.is_some() && matches!(expectations[&c.id], Expected::Result { .. })
                })
                .count()
                != manifest.required_paired_cases
        {
            return Err("paired case count differs from manifest requirement".into());
        }
        for id in &manifest.public_cases {
            if !cases.iter().any(|c| {
                &c.id == id
                    && c.sql.is_some()
                    && matches!(expectations[id], Expected::Result { .. })
            }) {
                return Err("public case must name a paired result case".into());
            }
        }
        resolve(&root, &manifest.project)?;
        for p in [&manifest.schemas, &manifest.canonical_data]
            .into_iter()
            .flatten()
        {
            resolve(&root, p)?;
        }
        for f in &manifest.fixtures {
            let e: Expected = read(&resolve(&root, &f.expected)?)?;
            validate_expected(&e)?;
            if !matches!(e, Expected::Result { .. }) {
                return Err("fixture must expect result".into());
            }
        }
        if let Some(env) = &manifest.environment {
            if env.compose_files.is_empty()
                || env.services.is_empty()
                || env.startup_timeout_seconds == 0
                || env.startup_timeout_seconds > 86400
                || env.bootstrap_timeout_seconds == 0
                || env.bootstrap_timeout_seconds > 86400
            {
                return Err("invalid lifecycle contract".into());
            }
            for p in &env.compose_files {
                resolve(&root, p)?;
            }
            for (name, b) in &env.bindings {
                if !valid_id(name)
                    || !env.services.contains(&b.service)
                    || b.port == 0
                    || !b.template.contains("{host}")
                    || !b.template.contains("{port}")
                {
                    return Err("invalid endpoint binding".into());
                }
            }
        }
        let digest = bundle_digest(&root)?;
        Ok(Self {
            manifest,
            root,
            cases,
            expectations,
            digest,
        })
    }
    /// Reject artifact changes since loading so reports cannot mix dataset revisions.
    pub fn verify_digest(&self) -> Result<()> {
        if bundle_digest(&self.root)? != self.digest {
            return Err("dataset artifact changed since validation".into());
        }
        Ok(())
    }
    /// Inspect the real project and Ossie model without starting services or resolving secrets.
    pub fn validate_project(&self) -> Result<()> {
        semantic_sources::Project::from_path(self.resolve(&self.manifest.project)?)?
            .inspect_project(&semantic_sources::Registry::standard())?;
        Ok(())
    }
    pub fn resolve(&self, p: &Path) -> Result<PathBuf> {
        resolve(&self.root, p)
    }
}
pub(crate) fn resolve(root: &Path, p: &Path) -> Result<PathBuf> {
    if p.is_absolute() {
        return Err("artifact paths must be relative".into());
    }
    let resolved = std::fs::canonicalize(root.join(p))?;
    if !resolved.starts_with(root) {
        return Err("artifact path escapes manifest bundle".into());
    }
    Ok(resolved)
}
fn bundle_digest(root: &Path) -> Result<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            if p.is_symlink() {
                return Err("bundle symlinks are not supported".into());
            }
            if p.is_dir() {
                walk(root, &p, out)?
            } else {
                out.push(p.strip_prefix(root)?.to_owned())
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, root, &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for p in files {
        let data = std::fs::read(root.join(&p))?;
        h.update(p.to_string_lossy().as_bytes());
        h.update([0]);
        h.update((data.len() as u64).to_le_bytes());
        h.update(data);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub(crate) fn validate_context(c: &Context) -> Result<()> {
    if c.calendar != "gregorian" {
        return Err("unsupported request calendar".into());
    }
    let time = chrono::DateTime::parse_from_rfc3339(&c.reference_time)?;
    semantic_compiler::typed::RequestContext {
        reference_unix_millis: time.timestamp_millis(),
        timezone: c.timezone.clone(),
        calendar: semantic_compiler::typed::Calendar::Gregorian,
        origin: semantic_compiler::typed::ContextOrigin::Caller,
    }
    .validate()
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn validate_expected(e: &Expected) -> Result<()> {
    if let Expected::Result { columns, rows } = e {
        crate::compare::validate_result(columns, rows)?
    }
    Ok(())
}
