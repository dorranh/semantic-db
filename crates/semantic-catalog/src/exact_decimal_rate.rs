//! Exact decimal rates authored over genuine, dated source rows.
use crate::{CatalogSnapshot, DataType, ObjectRef, SourceRef, canonical_digest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Exact quantization modes; legacy rational conversions retain their own enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecimalRounding {
    Truncate,
    HalfEven,
    HalfAwayFromZero,
}
/// Missing rate values cannot be silently replaced by parity or zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullRatePolicy {
    Unavailable,
}
/// Versioned source-rate meaning, independent of requested amounts and answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactDecimalRateRule {
    pub version: u32,
    pub id: String,
    pub rate_relation: String,
    pub source_currency_field: String,
    pub date_field: String,
    pub rate_field: String,
    pub target_currency: String,
    pub positive_only: bool,
    pub null_rate: NullRatePolicy,
    #[serde(default)]
    pub source_refs: Vec<SourceRef>,
}
impl ExactDecimalRateRule {
    /// Validate the closed identity and policy before any source is loaded.
    pub fn validate_contract(&self) -> Result<(), &'static str> {
        if self.version != 1
            || !self.positive_only
            || self.id.trim().is_empty()
            || self.id.len() > 256
            || [
                &self.rate_relation,
                &self.source_currency_field,
                &self.date_field,
                &self.rate_field,
            ]
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 256)
            || self.target_currency.len() != 3
            || !self.target_currency.bytes().all(|c| c.is_ascii_uppercase())
        {
            return Err("invalid exact decimal rate identity or policy");
        }
        Ok(())
    }
    /// Validate exact physical types and the authorized rate relation.
    pub fn validate(
        &self,
        snapshot: &CatalogSnapshot,
        scope: Option<&BTreeSet<String>>,
    ) -> Result<(), &'static str> {
        self.validate_contract()?;
        if scope.is_some_and(|s| !s.contains(&self.rate_relation)) {
            return Err("exact decimal rate leaves the authorized scope");
        }
        let relation = snapshot
            .relation(&self.rate_relation)
            .ok_or("exact decimal rate relation is missing")?;
        let currency = relation
            .field(&self.source_currency_field)
            .ok_or("exact decimal rate currency field is missing")?;
        let date = relation
            .field(&self.date_field)
            .ok_or("exact decimal rate date field is missing")?;
        let rate = relation
            .field(&self.rate_field)
            .ok_or("exact decimal rate value field is missing")?;
        if currency.is_nullable()
            || date.is_nullable()
            || currency.data_type() != &DataType::Utf8
            || date.data_type() != &DataType::Date32
            || !matches!(rate.data_type(),DataType::Decimal128(p,s) if *p>0 && *p<=38 && *s>=0 && *s<=*p as i8)
        {
            return Err(
                "exact decimal rate requires Utf8 currency, Date32 date and exact Decimal128 rate",
            );
        }
        Ok(())
    }
    /// Stable contract identity; provenance remains attached but is not executable meaning.
    pub fn reference(&self) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: self.id.clone(),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("rate contract serializes"),
            ),
        }
    }
}
