use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};
use utoipa::ToSchema;

/// Raw key only (e.g. `"01jd4a8xyz"`), not the old `"research_folder:<key>"`
/// form -- changed as part of Phase 3's cutover so a folder id is the same
/// bare string everywhere a client sees one, including a note's own
/// `folder_id` (which round-trips through tack-server's `tack_note_meta`
/// sidecar as whatever raw string the client sent -- see
/// `handlers::research_note::resolve_or_create_tack_folder`). The old
/// prefixed form meant `GET /api/folders` and a note's `folder_id` used
/// different formats for what should compare equal
/// (`note.folder_id === folder.id` is exactly how the frontend picks the
/// active folder), which broke the moment the note side stopped being a
/// direct SurrealDB round-trip.
fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&key_str)
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct ResearchFolder {
    #[schema(value_type = String, example = "01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub name: String,
    pub created_by: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateResearchFolder {
    pub name: String,
    // No `created_by` field, unlike the old request shape -- that let a
    // caller attribute a folder to any username with zero validation, same
    // gap `CreateResearchNote` had (see that model's own doc comment).
    // Always the authenticated caller (`ClannAuth::username` -- this
    // registry stays username-keyed, never unified with tack's UUID-keyed
    // `created_by` elsewhere in this API).
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RenameResearchFolder {
    pub name: String,
}
