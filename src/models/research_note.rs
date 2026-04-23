use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};
use utoipa::ToSchema;

pub fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct ResearchNote {
    /// Record ID in the form `research_note:<ulid>`.
    #[schema(value_type = String, example = "research_note:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,

    pub title: String,
    pub description: Option<String>,
    /// Markdown body; may include embedded DAM images.
    pub body: Option<String>,
    /// Names of the family trees this note is linked to.
    pub trees: Vec<String>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateResearchNote {
    pub title: String,
    pub description: Option<String>,
    pub body: Option<String>,
    pub trees: Vec<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateResearchNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trees: Option<Vec<String>>,
}
