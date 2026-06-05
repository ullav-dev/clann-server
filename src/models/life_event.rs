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

/// Common life event types. Stored as a plain string so callers can supply
/// arbitrary values beyond this list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub enum EventType {
    Birth,
    Death,
    Marriage,
    Divorce,
    Graduation,
    Immigration,
    Emigration,
    Military,
    NameChange,
    Other,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct LifeEvent {
    /// Record ID in the form `life_event:<ulid>`.
    #[schema(value_type = String, example = "life_event:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,

    /// The person this event belongs to; serialised as `"person:<ulid>"` in JSON.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,

    pub name: String,
    pub date: Option<String>,
    /// Categorises the event. Stored as a plain string; the `EventType` enum
    /// lists common values but callers may supply any string.
    pub event_type: String,
    pub description: Option<String>,
    /// Long-form story written in Markdown; may include embedded DAM images.
    pub story: Option<String>,
    #[serde(default)]
    pub verified: bool,
    pub source_link: Option<String>,
    pub source_image: Option<String>,
    pub source_doc: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateLifeEvent {
    pub name: String,
    pub event_type: String,
    pub date: Option<String>,
    pub description: Option<String>,
    pub story: Option<String>,
    #[serde(default)]
    pub verified: bool,
    pub source_link: Option<String>,
    pub source_image: Option<String>,
    pub source_doc: Option<String>,
    pub created_by: Option<String>,
}

/// All fields optional; only supplied fields are updated (MERGE semantics).
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateLifeEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub story: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_doc: Option<String>,
}
