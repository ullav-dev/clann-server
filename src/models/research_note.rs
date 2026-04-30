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

pub fn serialize_opt_record_id<S: serde::Serializer>(
    id: &Option<RecordId>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match id {
        Some(rid) => {
            let key_str = match &rid.key {
                RecordIdKey::String(k) => k.clone(),
                other => format!("{other:?}"),
            };
            s.serialize_some(&format!("{}:{}", rid.table, key_str))
        }
        None => s.serialize_none(),
    }
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
    pub folder_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// When true, visible to all team members with access to the tree.
    #[serde(default)]
    pub is_shared: bool,
    /// Set on replies; None for top-level notes.
    #[schema(value_type = Option<String>, example = "research_note:01jd4a8xyz")]
    #[serde(
        default,
        serialize_with = "serialize_opt_record_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_id: Option<RecordId>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateResearchNote {
    pub title: String,
    pub description: Option<String>,
    pub body: Option<String>,
    pub trees: Vec<String>,
    pub folder_id: Option<String>,
    pub created_by: Option<String>,
    #[serde(default)]
    pub is_shared: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_shared: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetNoteFolderPayload {
    pub folder_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateNoteReply {
    pub body: String,
    pub created_by: Option<String>,
    #[serde(default)]
    pub trees: Vec<String>,
}
