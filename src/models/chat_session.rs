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

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct ChatSession {
    #[schema(value_type = String, example = "chat_session:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub title: String,
    pub created_by: String,
    pub tree: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateChatSession {
    pub title: String,
    pub created_by: String,
    pub tree: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct ChatMessage {
    #[schema(value_type = String, example = "chat_message:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    #[schema(value_type = String, example = "chat_session:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub session_id: RecordId,
    pub role: String,
    pub content: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AppendMessage {
    pub role: String,
    pub content: String,
}
