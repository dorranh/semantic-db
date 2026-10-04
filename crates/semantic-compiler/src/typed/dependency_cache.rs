//! Scoped, exact lookup dependencies for reusable compiler analysis. An entry
//! captures lookup *results*, including absence and competing candidates.
//! Executable artifacts remain pinned to their own catalog snapshot.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::{Arc, Mutex, Weak},
};

use semantic_catalog::{CatalogSnapshot, ObjectRef, SnapshotRelation, canonical_digest};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Concept,
    Metric,
    Relationship,
    Conversion,
    Allocation,
    CurrencyRate,
    BusinessCalendar,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "lookup", rename_all = "snake_case")]
pub enum LookupDependency {
    /// Conservative index/namespace dependency for ranked retrieval.
    Snapshot,
    Relation {
        name: String,
    },
    Field {
        relation: String,
        name: String,
    },
    Definition {
        relation: String,
        kind: String,
        name: String,
    },
    /// Exact lookup includes all matching names and aliases, even when empty.
    Candidates {
        relation: String,
        kind: CandidateKind,
        phrase: String,
    },
    PolicySet {
        relation: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyCaptureError {
    Empty,
    TooMany,
    InvalidLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencySet {
    results: Vec<(LookupDependency, Option<String>)>,
}

impl DependencySet {
    pub fn capture(
        snapshot: &CatalogSnapshot,
        lookups: impl IntoIterator<Item = LookupDependency>,
        max_lookups: usize,
    ) -> Result<Self, DependencyCaptureError> {
        if max_lookups == 0 {
            return Err(DependencyCaptureError::InvalidLimits);
        }
        let mut unique = BTreeSet::new();
        for lookup in lookups {
            unique.insert(lookup);
            if unique.len() > max_lookups {
                return Err(DependencyCaptureError::TooMany);
            }
        }
        if unique.is_empty() {
            return Err(DependencyCaptureError::Empty);
        }
        Ok(Self {
            results: unique
                .into_iter()
                .map(|lookup| {
                    let result = lookup_result(snapshot, &lookup);
                    (lookup, result)
                })
                .collect(),
        })
    }

    pub fn matches(&self, snapshot: &CatalogSnapshot) -> bool {
        self.results
            .iter()
            .all(|(lookup, result)| lookup_result(snapshot, lookup) == *result)
    }

    pub fn lookups(&self) -> impl Iterator<Item = &LookupDependency> {
        self.results.iter().map(|(lookup, _)| lookup)
    }
}

fn lookup_result(snapshot: &CatalogSnapshot, lookup: &LookupDependency) -> Option<String> {
    match lookup {
        LookupDependency::Snapshot => Some(snapshot.id().to_owned()),
        LookupDependency::Relation { name } => snapshot.relation(name).map(|relation| {
            canonical_digest(&serde_json::json!({
                "semantic": relation.semantic_revision(),
                "binding": relation.binding_revision(),
                "provenance": relation.reference().revision,
            }))
        }),
        LookupDependency::Field { relation, name } => {
            let relation = snapshot.relation(relation)?;
            let field = relation.field(name)?;
            Some(canonical_digest(&serde_json::json!({
                "field": field,
                "annotation": relation.definition().semantics.as_ref()
                    .and_then(|semantics| semantics.fields.get(name)),
            })))
        }
        LookupDependency::Definition {
            relation,
            kind,
            name,
        } => snapshot
            .relation(relation)
            .and_then(|relation| relation.definition_reference(kind, name))
            .map(|reference| canonical_digest(&serde_json::json!(reference))),
        LookupDependency::Candidates {
            relation,
            kind,
            phrase,
        } => snapshot
            .relation(relation)
            .map(|relation| candidates(relation, *kind, phrase)),
        LookupDependency::PolicySet { relation } => snapshot.relation(relation).map(|relation| {
            canonical_digest(&serde_json::json!(
                relation
                    .definition()
                    .semantics
                    .as_ref()
                    .map(|semantics| &semantics.row_policies)
            ))
        }),
    }
}

fn candidates(relation: &SnapshotRelation, kind: CandidateKind, phrase: &str) -> String {
    let Some(semantics) = &relation.definition().semantics else {
        return canonical_digest(&serde_json::json!([]));
    };
    let references: Vec<&ObjectRef> = match kind {
        CandidateKind::Concept => semantics
            .concepts
            .iter()
            .filter(|(name, definition)| {
                name.as_str() == phrase || definition.aliases.iter().any(|alias| alias == phrase)
            })
            .filter_map(|(name, _)| relation.definition_reference("concept", name))
            .collect(),
        CandidateKind::Metric => semantics
            .metrics
            .iter()
            .filter(|(name, definition)| {
                name.as_str() == phrase || definition.aliases.iter().any(|alias| alias == phrase)
            })
            .filter_map(|(name, _)| relation.definition_reference("metric", name))
            .collect(),
        CandidateKind::Relationship => semantics
            .relationships
            .iter()
            .filter(|(name, definition)| name.as_str() == phrase || definition.id == phrase)
            .filter_map(|(name, _)| relation.definition_reference("relationship", name))
            .collect(),
        CandidateKind::Conversion => semantics
            .conversions
            .iter()
            .filter(|(name, definition)| name.as_str() == phrase || definition.id == phrase)
            .filter_map(|(name, _)| relation.definition_reference("conversion", name))
            .collect(),
        CandidateKind::Allocation => semantics
            .allocations
            .iter()
            .filter(|(name, definition)| name.as_str() == phrase || definition.id == phrase)
            .filter_map(|(name, _)| relation.definition_reference("allocation", name))
            .collect(),
        CandidateKind::CurrencyRate => semantics
            .currency_rates
            .iter()
            .filter(|(name, definition)| name.as_str() == phrase || definition.id == phrase)
            .filter_map(|(name, _)| relation.definition_reference("currency_rate", name))
            .collect(),
        CandidateKind::BusinessCalendar => semantics
            .business_calendars
            .iter()
            .filter(|(name, definition)| name.as_str() == phrase || definition.id == phrase)
            .filter_map(|(name, _)| relation.definition_reference("business_calendar", name))
            .collect(),
    };
    canonical_digest(&serde_json::json!(references))
}

/// Every field must be supplied by the caller from exact input/configuration.
/// No fuzzy similarity or implicit tenant sharing is permitted.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCacheIdentity {
    pub stage: String,
    pub input_digest: String,
    pub access_scope_revision: String,
    pub parameter_digest: String,
    pub renderer_revision: String,
    pub function_revision: String,
    pub acceptance_revision: String,
}

#[derive(Debug, Clone)]
pub struct AnalysisCacheLimits {
    pub max_entries: usize,
    pub max_bytes: usize,
    pub max_lookups: usize,
    /// Maximum distinct identities with an in-flight builder or waiter.
    pub max_active_keys: usize,
}

impl Default for AnalysisCacheLimits {
    fn default() -> Self {
        Self {
            max_entries: 128,
            max_bytes: 16 * 1024 * 1024,
            max_lookups: 128,
            max_active_keys: 64,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
    pub retained_bytes: usize,
    pub active_keys: usize,
    pub admission_rejections: u64,
}

struct Entry<V> {
    value: Arc<V>,
    dependencies: DependencySet,
    bytes: usize,
    used: u64,
}

struct State<V> {
    entries: BTreeMap<AnalysisCacheIdentity, Entry<V>>,
    active: BTreeMap<AnalysisCacheIdentity, Weak<AsyncMutex<()>>>,
    clock: u64,
    stats: AnalysisCacheStats,
}

impl<V> Default for State<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            active: BTreeMap::new(),
            clock: 0,
            stats: AnalysisCacheStats::default(),
        }
    }
}

#[derive(Debug)]
pub enum AnalysisCacheError<E> {
    Dependencies(DependencyCaptureError),
    Build(E),
    Admission,
}

/// Single-flight within an exact input identity. The immutable snapshot is
/// supplied by the caller and held across construction, so publication races
/// cannot mix revisions inside one cached analysis.
pub struct AnalysisCache<V> {
    limits: AnalysisCacheLimits,
    state: Mutex<State<V>>,
}

impl<V> AnalysisCache<V> {
    pub fn new(limits: AnalysisCacheLimits) -> Result<Self, DependencyCaptureError> {
        if limits.max_entries == 0
            || limits.max_bytes == 0
            || limits.max_lookups == 0
            || limits.max_active_keys == 0
        {
            return Err(DependencyCaptureError::InvalidLimits);
        }
        Ok(Self {
            limits,
            state: Mutex::new(State::default()),
        })
    }

    pub fn stats(&self) -> AnalysisCacheStats {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.active.retain(|_, weak| weak.strong_count() > 0);
        state.stats.active_keys = state.active.len();
        state.stats.clone()
    }

    pub async fn get_or_build<E, F, Fut>(
        &self,
        snapshot: &CatalogSnapshot,
        identity: AnalysisCacheIdentity,
        build: F,
    ) -> Result<Arc<V>, AnalysisCacheError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(V, Vec<LookupDependency>, usize), E>>,
    {
        let lock = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            state.active.retain(|_, weak| weak.strong_count() > 0);
            // A retained valid result needs no build slot, even when unrelated
            // identities have saturated the active-build admission limit.
            state.clock += 1;
            let used = state.clock;
            if let Some(entry) = state.entries.get_mut(&identity)
                && entry.dependencies.matches(snapshot)
            {
                entry.used = used;
                let value = entry.value.clone();
                state.stats.hits += 1;
                state.stats.active_keys = state.active.len();
                return Ok(value);
            }
            match state.active.get(&identity).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    if state.active.len() >= self.limits.max_active_keys {
                        state.stats.admission_rejections += 1;
                        state.stats.active_keys = state.active.len();
                        return Err(AnalysisCacheError::Admission);
                    }
                    let lock = Arc::new(AsyncMutex::new(()));
                    state.active.insert(identity.clone(), Arc::downgrade(&lock));
                    state.stats.active_keys = state.active.len();
                    lock
                }
            }
        };
        let _guard = lock.lock().await;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            state.clock += 1;
            let used = state.clock;
            if let Some(entry) = state.entries.get_mut(&identity) {
                if entry.dependencies.matches(snapshot) {
                    entry.used = used;
                    let value = entry.value.clone();
                    state.stats.hits += 1;
                    return Ok(value);
                }
                let removed = state.entries.remove(&identity).expect("stale entry");
                state.stats.retained_bytes -= removed.bytes;
                state.stats.evictions += 1;
                state.stats.entries -= 1;
            }
            state.stats.misses += 1;
        }
        let (value, lookups, bytes) = build().await.map_err(AnalysisCacheError::Build)?;
        let dependencies = DependencySet::capture(snapshot, lookups, self.limits.max_lookups)
            .map_err(AnalysisCacheError::Dependencies)?;
        let value = Arc::new(value);
        if bytes > self.limits.max_bytes {
            return Ok(value);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while state.entries.len() >= self.limits.max_entries
            || bytes
                > self
                    .limits
                    .max_bytes
                    .saturating_sub(state.stats.retained_bytes)
        {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
                .expect("retention requires an old entry");
            let removed = state.entries.remove(&oldest).expect("old entry");
            state.stats.retained_bytes -= removed.bytes;
            state.stats.evictions += 1;
        }
        state.clock += 1;
        let used = state.clock;
        state.entries.insert(
            identity,
            Entry {
                value: value.clone(),
                dependencies,
                bytes,
                used,
            },
        );
        state.stats.retained_bytes += bytes;
        state.stats.entries = state.entries.len();
        Ok(value)
    }
}
