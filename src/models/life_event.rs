use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};
use utoipa::ToSchema;

fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

fn serialize_opt_record_id<S: serde::Serializer>(
    id: &Option<RecordId>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match id {
        Some(rid) => serialize_record_id(rid, s),
        None => s.serialize_none(),
    }
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
    Other,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::Birth => "Birth",
            EventType::Death => "Death",
            EventType::Marriage => "Marriage",
            EventType::Divorce => "Divorce",
            EventType::Graduation => "Graduation",
            EventType::Immigration => "Immigration",
            EventType::Emigration => "Emigration",
            EventType::Military => "Military",
            EventType::Other => "Other",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct LifeEvent {
    /// Record ID in the form `life_event:<ulid>`.
    #[schema(value_type = String, example = "life_event:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,

    /// The person this event belongs to (`person:<ulid>`).
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,

    /// Short name or title for the event (e.g. "Birth", "Wedding", "Graduated from TCD").
    pub name: String,

    /// Fuzzy date string — free-form, e.g. "1920", "circa 1920", "June 1920", "14 June 1920".
    pub date: Option<String>,

    /// Categorises the event. Stored as a plain string; the `EventType` enum
    /// lists common values but callers may supply any string.
    pub event_type: String,

    /// Narrative description (plain text, short).
    pub description: Option<String>,

    /// Long-form story written in Markdown; may include embedded DAM images.
    pub story: Option<String>,

    /// Whether this event has been verified against primary sources.
    #[serde(default)]
    pub verified: bool,

    /// External URL pointing to a source document or webpage.
    pub source_link: Option<String>,

    /// DAM asset path/URL for a source image (e.g. scan of a birth certificate).
    pub source_image: Option<String>,

    /// DAM asset path/URL for a source document (e.g. a PDF scan).
    pub source_doc: Option<String>,

    /// The user who created this record.
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateLifeEvent {
    /// The person this event belongs to (`person:<ulid>`).
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(
        serialize_with = "serialize_opt_record_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub person_id: Option<RecordId>,

    pub name: String,
    pub date: Option<String>,
    /// Event type string. Use one of the `EventType` enum values or any custom string.
    pub event_type: String,
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
