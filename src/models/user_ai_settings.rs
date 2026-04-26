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

/// Stored AI settings for a user — the webapp encrypts API keys before sending;
/// clann-server stores the opaque encrypted blob and never sees the encryption key.
#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct UserAiSettings {
    #[schema(value_type = String, example = "user_ai_settings:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub username: String,
    pub provider: String,
    pub model: String,
    pub ollama_url: Option<String>,
    pub encrypted_key: Option<String>,
    pub iv: Option<String>,
    pub auth_tag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpsertUserAiSettings {
    pub provider: String,
    pub model: String,
    pub ollama_url: Option<String>,
    pub encrypted_key: Option<String>,
    pub iv: Option<String>,
    pub auth_tag: Option<String>,
}
