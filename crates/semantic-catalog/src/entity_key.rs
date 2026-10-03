//! Relation-scoped business identity and the authority of its key evidence.
//! A declaration describes intended row identity; it never proves that a
//! provider currently enforces or satisfies uniqueness.

use crate::{EntityId, FactResolution, SourceGrain, SourceRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyEvidence {
    /// Authored key declaration. This is not an enforced uniqueness guarantee.
    AuthoredDeclaration,
    /// Reserved for a trusted source-constraint attestation path.
    SourceConstraint {
        constraint_id: String,
        source_revision: String,
    },
    /// Reserved for a bounded verification over one execution snapshot.
    RuntimeVerification {
        verification_id: String,
        execution_snapshot: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityIdentity {
    pub id: EntityId,
    pub relation: String,
    pub source_grain: SourceGrain,
    pub key_evidence: FactResolution<KeyEvidence>,
    pub source_refs: Vec<SourceRef>,
}
