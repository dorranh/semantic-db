//! Rebuildable discovery index. Scores never confer semantic authority.
use crate::{AiContext, CatalogSnapshot};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SearchObject {
    pub relation: Arc<str>,
    pub field: Option<Arc<str>>,
}
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub object: SearchObject,
    pub score: u32,
    pub exact: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct SearchReport {
    pub hits: Vec<SearchHit>,
    pub postings_visited: usize,
    pub exhausted: bool,
    pub truncated_candidates: bool,
    /// Includes misses: a future publication can invalidate a negative lookup.
    pub lookup_fingerprints: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub max_postings: usize,
    pub max_candidates: usize,
    pub max_terms: usize,
    pub relation: Option<String>,
    pub allowed_relations: Option<BTreeSet<String>>,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            max_postings: 20_000,
            max_candidates: 64,
            max_terms: 64,
            relation: None,
            allowed_relations: None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct SearchIndex {
    snapshot_id: String,
    exact: crate::tree::Root<Postings>,
    lexical: crate::tree::Root<Postings>,
    footprints: crate::tree::Root<Footprint>,
    objects_indexed: usize,
    pub fields_indexed: usize,
}
#[derive(Debug)]
struct Postings {
    values: BTreeMap<SearchObject, u32>,
    revision: String,
}
impl Postings {
    fn new(values: BTreeMap<SearchObject, u32>) -> Self {
        let revision =
            crate::canonical_digest(&serde_json::json!(values.iter().collect::<Vec<_>>()));
        Self { values, revision }
    }
}
impl crate::tree::Revisioned for Postings {
    fn revision(&self) -> &str {
        &self.revision
    }
}
#[derive(Debug, Default, Serialize)]
struct Footprint {
    exact: BTreeSet<String>,
    lexical: BTreeSet<String>,
    objects: usize,
    fields: usize,
    #[serde(skip)]
    revision: String,
}
impl crate::tree::Revisioned for Footprint {
    fn revision(&self) -> &str {
        &self.revision
    }
}
#[derive(Debug)]
struct IndexBuilder {
    objects: Vec<SearchObject>,
    exact: BTreeMap<String, Vec<usize>>,
    lexical: BTreeMap<String, Vec<(usize, u8)>>,
    pub fields_indexed: usize,
    field_counts: BTreeMap<String, usize>,
}
impl IndexBuilder {
    fn build<'a, E>(
        entries: impl Iterator<Item = &'a crate::SnapshotRelation>,
        mut check: impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<Self, E> {
        let mut index = Self {
            objects: Vec::new(),
            exact: BTreeMap::new(),
            lexical: BTreeMap::new(),
            fields_indexed: 0,
            field_counts: BTreeMap::new(),
        };
        for entry in entries {
            check(1, 0)?;
            let relation = entry.definition();
            index
                .field_counts
                .insert(relation.name.clone(), relation.schema.fields().len());
            let name: Arc<str> = relation.name.as_str().into();
            let mut relation_text =
                vec![relation.description.as_deref(), relation.grain.as_deref()];
            if let Some(semantics) = &relation.semantics {
                relation_text.push(semantics.model_description.as_deref());
            }
            let contexts = relation
                .semantics
                .as_ref()
                .map(|s| [s.ai_context.as_ref(), s.model_ai_context.as_ref()])
                .unwrap_or([None, None]);
            index.insert(
                SearchObject {
                    relation: name.clone(),
                    field: None,
                },
                &relation.name,
                relation_text,
                &contexts,
                &[],
                &mut check,
            )?;
            if let Some(semantics) = &relation.semantics {
                for (mapping_name, mapping) in &semantics.value_mappings {
                    check(1, 0)?;
                    let object = SearchObject {
                        relation: name.clone(),
                        field: Some(mapping.field.as_str().into()),
                    };
                    index.insert(
                        object.clone(),
                        mapping_name,
                        vec![Some(&mapping.description)],
                        &[],
                        &[],
                        &mut check,
                    )?;
                    for phrase in mapping.codes.keys() {
                        check(1, 0)?;
                        index.insert(object.clone(), phrase, vec![], &[], &[], &mut check)?;
                    }
                }
                for (name_key, relationship) in &semantics.relationships {
                    check(1, 0)?;
                    index.insert(
                        SearchObject {
                            relation: name.clone(),
                            field: None,
                        },
                        name_key,
                        vec![Some(&relationship.role)],
                        &[relationship.ai_context.as_ref()],
                        &[],
                        &mut check,
                    )?;
                }
                for (ratio_name, ratio) in &semantics.ratio_metrics {
                    check(1, 0)?;
                    index.insert(
                        SearchObject {
                            relation: name.clone(),
                            field: None,
                        },
                        ratio_name,
                        vec![Some(&ratio.description)],
                        &[],
                        &ratio.aliases,
                        &mut check,
                    )?;
                }
                for (metric_name, metric) in &semantics.metrics {
                    check(1, 0)?;
                    index.insert(
                        SearchObject {
                            relation: name.clone(),
                            field: None,
                        },
                        metric_name,
                        vec![Some(&metric.description)],
                        &[],
                        &metric.aliases,
                        &mut check,
                    )?;
                }
            }
            for field in relation.schema.fields() {
                check(1, 0)?;
                let semantics = relation
                    .semantics
                    .as_ref()
                    .and_then(|s| s.fields.get(field.name()));
                let text = semantics
                    .map(|s| {
                        vec![
                            s.description.as_deref(),
                            s.label.as_deref(),
                            s.logical_type.as_deref(),
                        ]
                    })
                    .unwrap_or_default();
                let context = semantics.and_then(|s| s.ai_context.as_ref());
                index.insert(
                    SearchObject {
                        relation: name.clone(),
                        field: Some(field.name().as_str().into()),
                    },
                    field.name(),
                    text,
                    &[context],
                    &[],
                    &mut check,
                )?;
                index.fields_indexed += 1;
            }
        }
        Ok(index)
    }
    fn insert<E>(
        &mut self,
        object: SearchObject,
        name: &str,
        text: Vec<Option<&str>>,
        contexts: &[Option<&AiContext>],
        aliases: &[String],
        check: &mut impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<(), E> {
        // Check source bytes before normalization, cloning, or tokenization.
        // Also charge qualified names, which repeat the relation name per field.
        check(0, name.len())?;
        if let Some(field) = &object.field {
            check(
                0,
                object
                    .relation
                    .len()
                    .saturating_add(field.len())
                    .saturating_add(1),
            )?;
        }
        for value in aliases
            .iter()
            .map(String::as_str)
            .chain(text.iter().copied().flatten())
            .chain(
                contexts
                    .iter()
                    .flatten()
                    .flat_map(|context| {
                        context
                            .synonyms
                            .iter()
                            .chain(&context.instructions)
                            .chain(&context.examples)
                    })
                    .map(String::as_str),
            )
        {
            check(0, value.len())?;
        }
        let id = self.objects.len();
        let mut names = BTreeSet::from([normalize(name)]);
        names.extend(aliases.iter().map(|value| normalize(value)));
        let mut tokens = BTreeMap::new();
        for context in contexts.iter().flatten() {
            for alias in &context.synonyms {
                names.insert(normalize(alias));
            }
            // Examples/instructions aid discovery but are never made definitions.
            for text in context.instructions.iter().chain(context.examples.iter()) {
                for term in terms(text) {
                    tokens.entry(term).or_insert(1);
                }
            }
        }
        for name in &names {
            for term in terms(name) {
                tokens.insert(term, 4);
            }
        }
        for text in text.into_iter().flatten() {
            for term in terms(text) {
                tokens.entry(term).or_insert(1);
            }
        }
        // Qualified names are exact handles, not lexical field content. Adding
        // the parent name to every field's postings makes relation-only queries
        // enumerate the full width of a table.
        if let Some(field) = &object.field {
            names.insert(normalize(&format!("{}.{}", object.relation, field)));
        }
        for name in names {
            self.exact.entry(name).or_default().push(id);
        }
        for (term, weight) in tokens {
            self.lexical.entry(term).or_default().push((id, weight));
        }
        self.objects.push(object);
        Ok(())
    }
    fn finish<E>(
        self,
        snapshot_id: &str,
        check: &mut impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<SearchIndex, E> {
        let mut footprints: BTreeMap<String, Footprint> = BTreeMap::new();
        for object in &self.objects {
            let footprint = footprints.entry(object.relation.to_string()).or_default();
            footprint.objects += 1;
            footprint.fields = self.field_counts[object.relation.as_ref()];
        }
        let mut exact = crate::tree::Root::default();
        let mut lexical = crate::tree::Root::default();
        for (term, ids) in self.exact {
            let mut values = BTreeMap::new();
            for id in ids {
                check(0, 0)?;
                let object = self.objects[id].clone();
                footprints
                    .get_mut(object.relation.as_ref())
                    .expect("indexed footprint")
                    .exact
                    .insert(term.clone());
                let weight = values.entry(object).or_insert(0u32);
                *weight = weight.saturating_add(100);
            }
            exact = exact.insert(term, Arc::new(Postings::new(values)));
        }
        for (term, ids) in self.lexical {
            let mut values = BTreeMap::new();
            for (id, weight) in ids {
                check(0, 0)?;
                let object = self.objects[id].clone();
                footprints
                    .get_mut(object.relation.as_ref())
                    .expect("indexed footprint")
                    .lexical
                    .insert(term.clone());
                let value = values.entry(object).or_insert(0u32);
                *value = value.saturating_add(u32::from(weight));
            }
            lexical = lexical.insert(term, Arc::new(Postings::new(values)));
        }
        let mut root = crate::tree::Root::default();
        for (name, mut footprint) in footprints {
            footprint.revision = crate::canonical_digest(&serde_json::json!(footprint));
            root = root.insert(name, Arc::new(footprint));
        }
        Ok(SearchIndex {
            snapshot_id: snapshot_id.into(),
            exact,
            lexical,
            footprints: root,
            objects_indexed: self.objects.len(),
            fields_indexed: self.fields_indexed,
        })
    }
}
impl SearchIndex {
    pub(crate) fn build<E>(
        snapshot: &CatalogSnapshot,
        mut check: impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<Self, E> {
        IndexBuilder::build(snapshot.relations(), &mut check)?.finish(snapshot.id(), &mut check)
    }
    pub(crate) fn update<E>(
        &self,
        snapshot: &CatalogSnapshot,
        changed: &BTreeSet<String>,
        mut check: impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<Self, E> {
        let replacement = IndexBuilder::build(
            changed.iter().filter_map(|name| snapshot.relation(name)),
            &mut check,
        )?
        .finish(snapshot.id(), &mut check)?;
        let mut updated = self.clone();
        updated.snapshot_id = snapshot.id().into();
        let mut exact = BTreeSet::new();
        let mut lexical = BTreeSet::new();
        for name in changed {
            check(1, name.len())?;
            for footprint in self
                .footprints
                .get(name)
                .into_iter()
                .chain(replacement.footprints.get(name))
            {
                for term in &footprint.exact {
                    check(0, term.len())?;
                    exact.insert(term.clone());
                }
                for term in &footprint.lexical {
                    check(0, term.len())?;
                    lexical.insert(term.clone());
                }
            }
            if let Some(old) = self.footprints.get(name) {
                updated.objects_indexed -= old.objects;
                updated.fields_indexed -= old.fields;
            }
            if let Some(new) = replacement.footprints.get(name) {
                updated.objects_indexed += new.objects;
                updated.fields_indexed += new.fields;
                updated.footprints = updated.footprints.insert(name.clone(), new.clone());
            } else {
                updated.footprints = updated.footprints.remove(name);
            }
        }
        updated.exact =
            update_postings(&self.exact, &replacement.exact, exact, changed, &mut check)?;
        updated.lexical = update_postings(
            &self.lexical,
            &replacement.lexical,
            lexical,
            changed,
            &mut check,
        )?;
        Ok(updated)
    }
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    pub fn objects_indexed(&self) -> usize {
        self.objects_indexed
    }
    pub fn search<E>(
        &self,
        request: &str,
        options: &SearchOptions,
        mut check: impl FnMut() -> Result<(), E>,
    ) -> Result<SearchReport, E> {
        let mut lookup_terms = vec![format!("exact:{}", normalize(request))];
        let mut query_terms = BTreeSet::new();
        let mut exhausted = true;
        for term in terms(request) {
            if query_terms.len() >= options.max_terms && !query_terms.contains(&term) {
                exhausted = false;
                break;
            }
            query_terms.insert(term);
        }
        lookup_terms.extend(query_terms.iter().map(|term| format!("lexical:{term}")));
        let mut scores: BTreeMap<SearchObject, (u32, bool)> = BTreeMap::new();
        let mut visited = 0;
        let eligible = |object: &SearchObject| {
            options
                .relation
                .as_ref()
                .is_none_or(|r| r.as_str() == object.relation.as_ref())
                && options
                    .allowed_relations
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(object.relation.as_ref()))
        };
        for (id, weight, exact) in self
            .exact
            .get(&normalize(request))
            .into_iter()
            .flat_map(|postings| &postings.values)
            .map(|(object, weight)| (object, *weight, true))
            .chain(query_terms.iter().flat_map(|term| {
                self.lexical
                    .get(term)
                    .into_iter()
                    .flat_map(|postings| &postings.values)
                    .map(|(object, weight)| (object, *weight, false))
            }))
        {
            check()?;
            if visited >= options.max_postings {
                exhausted = false;
                break;
            }
            visited += 1;
            if eligible(id) {
                let score = scores.entry(id.clone()).or_default();
                score.0 = score.0.saturating_add(weight);
                score.1 |= exact;
            }
        }
        let mut hits: Vec<_> = scores
            .into_iter()
            .map(|(id, (score, exact))| SearchHit {
                object: id,
                score,
                exact,
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.object.cmp(&b.object)));
        let truncated_candidates = hits.len() > options.max_candidates;
        hits.truncate(options.max_candidates);
        Ok(SearchReport {
            hits,
            postings_visited: visited,
            exhausted,
            truncated_candidates,
            lookup_fingerprints: lookup_terms
                .iter()
                .map(|term| format!("{:x}", Sha256::digest(term.as_bytes())))
                .collect(),
        })
    }
}
fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}
fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
}

fn update_postings<E>(
    old: &crate::tree::Root<Postings>,
    replacement: &crate::tree::Root<Postings>,
    terms: BTreeSet<String>,
    changed: &BTreeSet<String>,
    check: &mut impl FnMut(usize, usize) -> Result<(), E>,
) -> Result<crate::tree::Root<Postings>, E> {
    let mut root = old.clone();
    for term in terms {
        // Compare just the changed relations' ranges first. An annotation edit
        // must not clone every common-field posting list on the relation.
        let old_postings = old.get(&term);
        let new_postings = replacement.get(&term);
        let mut before = changed.iter().flat_map(|name| {
            old_postings.into_iter().flat_map(move |postings| {
                postings
                    .values
                    .range(
                        SearchObject {
                            relation: name.as_str().into(),
                            field: None,
                        }..,
                    )
                    .take_while(move |(object, _)| object.relation.as_ref() == name)
            })
        });
        let mut after = new_postings
            .into_iter()
            .flat_map(|postings| postings.values.iter());
        let same = loop {
            check(0, 0)?;
            match (before.next(), after.next()) {
                (None, None) => break true,
                (Some(a), Some(b)) if a == b => {}
                _ => break false,
            }
        };
        if same {
            continue;
        }
        let mut values = BTreeMap::new();
        if let Some(postings) = old.get(&term) {
            for (object, weight) in &postings.values {
                check(
                    1,
                    object
                        .relation
                        .len()
                        .saturating_add(object.field.as_ref().map_or(0, |f| f.len())),
                )?;
                if !changed.contains(object.relation.as_ref()) {
                    values.insert(object.clone(), *weight);
                }
            }
        }
        if let Some(postings) = replacement.get(&term) {
            values.extend(
                postings
                    .values
                    .iter()
                    .map(|(object, weight)| (object.clone(), *weight)),
            );
        }
        root = if values.is_empty() {
            root.remove(&term)
        } else {
            root.insert(term, Arc::new(Postings::new(values)))
        };
    }
    Ok(root)
}
