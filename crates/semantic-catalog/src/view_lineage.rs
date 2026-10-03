//! Exact direct-projection lineage for one-source views.
//! The canonical SQL equality is the proof boundary: no expression, filter,
//! join, or implicit cast can be silently described as direct lineage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{ObjectRef, Schema, SourceRef, canonical_digest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewOutputLineage {
    /// Exact source relation revision in the same catalog snapshot.
    pub source: ObjectRef,
    /// Every output column maps to one physical source column.
    pub columns: BTreeMap<String, String>,
    pub source_refs: Vec<SourceRef>,
}

impl ViewOutputLineage {
    pub fn reference(&self, relation: &str) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: format!("view_lineage/{relation}"),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("view lineage serializes"),
            ),
        }
    }

    /// This deliberately accepts only an unquoted, lower-case identifier
    /// profile and exactly the output fields in schema order.
    pub fn canonical_sql(&self, output: &Schema) -> Option<String> {
        if !identifier(&self.source.id)
            || self.source.revision.is_empty()
            || output.fields().is_empty()
            || output.fields().len() > 64
            || self.columns.len() != output.fields().len()
        {
            return None;
        }
        let mut projections = Vec::with_capacity(output.fields().len());
        for field in output.fields() {
            let name = field.name();
            let source = self.columns.get(name.as_str())?;
            if !identifier(name) || !identifier(source) {
                return None;
            }
            projections.push(format!("{source} AS {name}"));
        }
        Some(format!(
            "SELECT {} FROM {}",
            projections.join(", "),
            self.source.id
        ))
    }
}

fn identifier(name: &str) -> bool {
    name.len() <= 128
        && name
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || *byte == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
