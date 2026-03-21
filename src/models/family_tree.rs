use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
use utoipa::ToSchema;

fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    use surrealdb::types::RecordIdKey;
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct FamilyTree {
    /// Record ID in the form `family_tree:<ulid>`.
    #[schema(value_type = String, example = "family_tree:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Unique slug identifier for this tree (e.g. `"smith-family"`). Globally unique.
    pub name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Username of the tree owner.
    pub owner: String,
    /// Whether this is the owner's primary tree.
    #[serde(default)]
    pub is_primary: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct CreateFamilyTree {
    /// Unique slug identifier for this tree (e.g. `"smith-family"`). Must be globally unique.
    pub name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Username of the tree owner.
    pub owner: String,
    /// Set to `true` to make this the owner's primary tree (clears `is_primary` on all others).
    #[serde(default)]
    pub is_primary: bool,
}
