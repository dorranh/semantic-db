//! Authored business/fiscal calendar mapping contract. Catalog validation pins
//! identities and physical types; an executable join must separately verify
//! date uniqueness and coverage under the same read boundary as facts.

use std::collections::BTreeSet;

use crate::{CatalogSnapshot, DataType, ObjectRef, SourceRef, canonical_digest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BUSINESS_CALENDAR_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarSourceBasis {
    UtcInstantMicros,
    Date32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusinessCalendarRule {
    pub version: u32,
    pub id: String,
    pub source_relation: String,
    pub calendar_relation: String,
    pub source_date_field: String,
    pub calendar_date_field: String,
    pub fiscal_year_field: String,
    pub fiscal_period_field: String,
    pub business_day_field: String,
    pub source_basis: CalendarSourceBasis,
    /// IANA timezone used to interpret an instant as a business date. Date32
    /// mappings still name the calendar's timezone to prevent silent reuse.
    pub timezone: String,
    /// Authored mapping-data revision; a schema revision alone cannot pin rows.
    pub mapping_revision: String,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BusinessCalendarError {
    #[error("unsupported business calendar contract version")]
    Version,
    #[error("business calendar identity or mapping revision is missing")]
    Identity,
    #[error("business calendar timezone is not a recognized IANA zone")]
    Timezone,
    #[error("business calendar relation is missing")]
    Relation,
    #[error("business calendar field is missing or ambiguous")]
    Field,
    #[error("business calendar physical field type is unsupported")]
    Type,
    #[error("business calendar relation leaves the authorized scope")]
    Scope,
}

impl BusinessCalendarRule {
    pub fn reference(&self) -> ObjectRef {
        let mut canonical = self.clone();
        canonical.source_refs.clear();
        ObjectRef {
            id: self.id.clone(),
            revision: canonical_digest(
                &serde_json::to_value(canonical).expect("business calendar serializes"),
            ),
        }
    }

    pub fn validate(
        &self,
        snapshot: &CatalogSnapshot,
        allowed_relations: Option<&BTreeSet<String>>,
    ) -> Result<(), BusinessCalendarError> {
        if self.version != BUSINESS_CALENDAR_VERSION {
            return Err(BusinessCalendarError::Version);
        }
        if [
            &self.id,
            &self.source_relation,
            &self.calendar_relation,
            &self.source_date_field,
            &self.calendar_date_field,
            &self.fiscal_year_field,
            &self.fiscal_period_field,
            &self.business_day_field,
            &self.timezone,
            &self.mapping_revision,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(BusinessCalendarError::Identity);
        }
        if self.timezone.parse::<chrono_tz::Tz>().is_err() {
            return Err(BusinessCalendarError::Timezone);
        }
        if allowed_relations.is_some_and(|scope| {
            !scope.contains(&self.source_relation) || !scope.contains(&self.calendar_relation)
        }) {
            return Err(BusinessCalendarError::Scope);
        }
        let source = snapshot
            .relation(&self.source_relation)
            .ok_or(BusinessCalendarError::Relation)?;
        let calendar = snapshot
            .relation(&self.calendar_relation)
            .ok_or(BusinessCalendarError::Relation)?;
        let source_field = source
            .field(&self.source_date_field)
            .ok_or(BusinessCalendarError::Field)?;
        let expected = match self.source_basis {
            CalendarSourceBasis::UtcInstantMicros => {
                DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
            }
            CalendarSourceBasis::Date32 => DataType::Date32,
        };
        if source_field.data_type() != &expected {
            return Err(BusinessCalendarError::Type);
        }
        for (name, expected) in [
            (&self.calendar_date_field, DataType::Date32),
            (&self.fiscal_year_field, DataType::Int32),
            (&self.fiscal_period_field, DataType::Int16),
            (&self.business_day_field, DataType::Boolean),
        ] {
            let field = calendar.field(name).ok_or(BusinessCalendarError::Field)?;
            if field.data_type() != &expected || field.is_nullable() {
                return Err(BusinessCalendarError::Type);
            }
        }
        Ok(())
    }
}
