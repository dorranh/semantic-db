//! Canonical JSON independent of serde_json's preserve_order feature unification.
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
struct Canonical<'a>(&'a Value);
impl Serialize for Canonical<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(values) => {
                let sorted: std::collections::BTreeMap<_, _> = values.iter().collect();
                let mut map = serializer.serialize_map(Some(sorted.len()))?;
                for (key, value) in sorted {
                    map.serialize_entry(key, &Canonical(value))?;
                }
                map.end()
            }
            Value::Array(values) => {
                let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    sequence.serialize_element(&Canonical(value))?;
                }
                sequence.end()
            }
            value => value.serialize(serializer),
        }
    }
}
pub fn canonical_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(&Canonical(value)).expect("JSON values serialize")
}
pub fn canonical_digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(canonical_json(value)))
}
