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
pub struct ResearchFolder {
    #[schema(value_type = String, example = "research_folder:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub name: String,
    pub created_by: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateResearchFolder {
    pub name: String,
    pub created_by: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RenameResearchFolder {
    pub name: String,
}
