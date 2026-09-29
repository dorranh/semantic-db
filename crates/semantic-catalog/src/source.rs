//! Source fidelity is separate from executable semantic authority.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, sync::Arc};

/// Retained bytes are deliberately excluded from Debug and ordinary catalog serialization.
#[derive(Clone)]
pub struct SourceArchive {
    bytes: Arc<[u8]>,
    revision: String,
    pub format: String,
    pub specification: String,
    pub adapter: String,
}
impl SourceArchive {
    pub fn new(
        bytes: impl Into<Arc<[u8]>>,
        format: impl Into<String>,
        specification: impl Into<String>,
        adapter: impl Into<String>,
    ) -> Self {
        let bytes = bytes.into();
        Self {
            revision: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
            format: format.into(),
            specification: specification.into(),
            adapter: adapter.into(),
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
}
impl fmt::Debug for SourceArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceArchive")
            .field("revision", &self.revision)
            .field("bytes", &self.bytes.len())
            .field("format", &self.format)
            .finish()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: usize,
    pub column: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub artifact_revision: String,
    /// RFC 6901 pointer. An empty string identifies the document root.
    pub path: String,
    pub span: Option<SourceSpan>,
}

/// Source-language numbers retain their exact spelling and explicit tag. They
/// cannot enter executable arithmetic until a typed adapter validates them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SourceValue {
    Null,
    Boolean(bool),
    Number(String),
    String(String),
    Sequence(Vec<SourceNode>),
    Mapping(BTreeMap<String, SourceNode>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceNode {
    pub value: SourceValue,
    pub tag: Option<String>,
    pub origins: Vec<SourceRef>,
}
impl SourceNode {
    /// Absence is None; explicit null is Some(SourceValue::Null).
    pub fn pointer(&self, pointer: &str) -> Option<&Self> {
        if pointer.is_empty() {
            return Some(self);
        }
        let mut node = self;
        for token in pointer.strip_prefix('/')?.split('/') {
            let token = token.replace("~1", "/").replace("~0", "~");
            node = match &node.value {
                SourceValue::Mapping(values) => values.get(&token)?,
                SourceValue::Sequence(values) => values.get(token.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        Some(node)
    }
    /// Locations and source formatting do not invalidate semantic content. Number
    /// spelling remains significant until an adapter defines canonical arithmetic.
    pub fn semantic_value(&self) -> serde_json::Value {
        let value = match &self.value {
            SourceValue::Mapping(values) => {
                serde_json::json!({"mapping": values.iter().map(|(k,v)| (k.clone(), v.semantic_value())).collect::<BTreeMap<_,_>>()})
            }
            SourceValue::Sequence(values) => {
                serde_json::json!({"sequence": values.iter().map(Self::semantic_value).collect::<Vec<_>>()})
            }
            value => serde_json::to_value(value).expect("source value serializes"),
        };
        serde_json::json!({"value": value, "tag": self.tag})
    }
    pub fn semantic_revision(&self) -> String {
        crate::canonical_digest(&self.semantic_value())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Presence<T> {
    Missing,
    Null,
    Value(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Capability {
    Executable { profile: String, revision: String },
    DescriptiveOnly { reason: String },
    Blocked { diagnostics: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    Authored,
    SourceConstraint,
    Verified,
    Derived,
    Proposed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact<T> {
    pub id: String,
    pub scope: String,
    pub value: T,
    pub authority: Authority,
    pub origins: Vec<SourceRef>,
    /// Evidence is revisioned and scoped; historical observations are not enforcement.
    pub evidence: Vec<VerificationEvidence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub rule: String,
    pub revision: String,
    pub scope: String,
    pub execution_snapshot: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FactResolution<T> {
    Unknown,
    Known {
        value: T,
        contributors: Vec<Fact<T>>,
    },
    Conflicting {
        alternatives: Vec<Fact<T>>,
    },
}
/// No implicit authority ranking: disagreement remains visible, including when
/// one contributor is a proposal. Adapters must implement explicit promotion rules.
pub fn resolve_facts<T: Clone + Eq>(facts: impl IntoIterator<Item = Fact<T>>) -> FactResolution<T> {
    let mut facts: Vec<_> = facts.into_iter().collect();
    facts.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.id.cmp(&b.id)));
    let Some(first) = facts.first() else {
        return FactResolution::Unknown;
    };
    if facts
        .iter()
        .any(|fact| fact.value != first.value || fact.scope != first.scope)
    {
        FactResolution::Conflicting {
            alternatives: facts,
        }
    } else {
        FactResolution::Known {
            value: first.value.clone(),
            contributors: facts,
        }
    }
}
