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
pub struct TreeEditor {
    #[schema(value_type = String)]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub tree_name: String,
    /// UUID of the user granted editor access (matches JWT `sub`).
    pub user_id: String,
    /// UUID of the user who granted this access.
    pub granted_by_user_id: String,
    pub granted_at: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AddTreeEditor {
    /// UUID of the team member to grant editor access (`TeamMember.user.id`).
    pub user_id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TreeAccessResponse {
    /// The caller's access level for this tree: `"owner"`, `"editor"`, or `"viewer"`.
    pub role: String,
}
