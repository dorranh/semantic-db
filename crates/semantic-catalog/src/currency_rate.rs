//! Authored, dated currency conversion through a catalog rate relation.
//! Binding must execute the rate lookup under the same read boundary as facts;
//! catalog declarations alone cannot prove interval uniqueness or coverage.

use std::collections::BTreeSet;

use crate::{
    CatalogSnapshot, ConversionRounding, DataType, ObjectRef, SourceRef, canonical_digest,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENCY_RATE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateTimeBasis {
    UtcInstantMicros,
    BusinessDate { timezone: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyRateRule {
    pub version: u32,
    pub id: String,
    pub source_relation: String,
    pub rate_relation: String,
    pub source_amount_field: String,
    pub source_currency_field: String,
    pub source_time_field: String,
    pub to_currency: String,
    pub rate_from_currency_field: String,
    pub rate_to_currency_field: String,
    pub rate_valid_from_field: String,
    pub rate_valid_to_field: String,
    pub rate_numerator_field: String,
    pub rate_denominator_field: String,
    pub time_basis: RateTimeBasis,
    pub rounding: ConversionRounding,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CurrencyRateError {
    #[error("unsupported currency rate rule version")]
    Version,
    #[error("currency rate identity or time basis is invalid")]
    Identity,
    #[error("currency rate source or rate relation is missing")]
    Relation,
    #[error("currency rate field is missing or ambiguous")]
    Field,
    #[error("currency rate field physical type is unsupported")]
    Type,
    #[error("currency rate relation leaves the authorized scope")]
    Scope,
}

impl CurrencyRateRule {
    pub fn reference(&self) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: self.id.clone(),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("currency rate rule serializes"),
            ),
        }
    }

    pub fn validate(
        &self,
        snapshot: &CatalogSnapshot,
        allowed_relations: Option<&BTreeSet<String>>,
    ) -> Result<(), CurrencyRateError> {
        if self.version != CURRENCY_RATE_VERSION {
            return Err(CurrencyRateError::Version);
        }
        let names = [
            &self.id,
            &self.source_relation,
            &self.rate_relation,
            &self.source_amount_field,
            &self.source_currency_field,
            &self.source_time_field,
            &self.to_currency,
            &self.rate_from_currency_field,
            &self.rate_to_currency_field,
            &self.rate_valid_from_field,
            &self.rate_valid_to_field,
            &self.rate_numerator_field,
            &self.rate_denominator_field,
        ];
        if names.iter().any(|value| value.trim().is_empty())
            || matches!(&self.time_basis, RateTimeBasis::BusinessDate { timezone } if timezone.parse::<chrono_tz::Tz>().is_err())
        {
            return Err(CurrencyRateError::Identity);
        }
        if allowed_relations.is_some_and(|scope| {
            !scope.contains(&self.source_relation) || !scope.contains(&self.rate_relation)
        }) {
            return Err(CurrencyRateError::Scope);
        }
        let source = snapshot
            .relation(&self.source_relation)
            .ok_or(CurrencyRateError::Relation)?;
        let rate = snapshot
            .relation(&self.rate_relation)
            .ok_or(CurrencyRateError::Relation)?;
        let source_field = |name: &str| source.field(name).ok_or(CurrencyRateError::Field);
        let rate_field = |name: &str| rate.field(name).ok_or(CurrencyRateError::Field);
        let amount = source_field(&self.source_amount_field)?;
        let currency = source_field(&self.source_currency_field)?;
        let time = source_field(&self.source_time_field)?;
        if amount.data_type() != &DataType::Int64
            || currency.data_type() != &DataType::Utf8
            || currency.is_nullable()
            || time.is_nullable()
        {
            return Err(CurrencyRateError::Type);
        }
        let expected_time = match &self.time_basis {
            RateTimeBasis::UtcInstantMicros => {
                DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
            }
            RateTimeBasis::BusinessDate { .. } => DataType::Date32,
        };
        if time.data_type() != &expected_time {
            return Err(CurrencyRateError::Type);
        }
        for field in [&self.rate_from_currency_field, &self.rate_to_currency_field] {
            let field = rate_field(field)?;
            if field.data_type() != &DataType::Utf8 || field.is_nullable() {
                return Err(CurrencyRateError::Type);
            }
        }
        for field in [&self.rate_valid_from_field, &self.rate_valid_to_field] {
            let field = rate_field(field)?;
            if field.data_type() != &expected_time || field.is_nullable() {
                return Err(CurrencyRateError::Type);
            }
        }
        for field in [&self.rate_numerator_field, &self.rate_denominator_field] {
            let field = rate_field(field)?;
            if field.data_type() != &DataType::Int64 || field.is_nullable() {
                return Err(CurrencyRateError::Type);
            }
        }
        Ok(())
    }
}
