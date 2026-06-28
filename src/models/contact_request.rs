use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
use utoipa::ToSchema;

use crate::models::person::serialize_record_id;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, ToSchema)]
pub struct ContactMessage {
    pub from_user: String,
    pub text: String,
    pub sent_at: String,
}

#[derive(Debug, Serialize, Deserialize, SurrealValue, ToSchema)]
pub struct MergeContactRequest {
    #[schema(value_type = String, example = "merge_contact_request:01jd4xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    #[schema(value_type = String, example = "person_proxy:01jd4xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub from_proxy_id: RecordId,
    pub from_user: String,
    pub to_user: String,
    pub initial_message: Option<String>,
    /// `pending`, `accepted`, or `ignored`
    pub status: String,
    #[serde(default)]
    pub messages: Vec<ContactMessage>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateContactRequest {
    /// Proxy ID of the person in the requester's tree (`person_proxy:<ulid>`).
    pub from_proxy_id: String,
    /// Usernames of the target tree owners to contact.
    pub to_users: Vec<String>,
    /// Optional opening message to the recipients.
    pub message: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AppendContactMessage {
    pub text: String,
}

/// A single candidate duplicate found by the scored matching algorithm.
#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateMatch {
    /// Full proxy record ID: `person_proxy:<ulid>`.
    pub proxy_id: String,
    /// Full canonical record ID: `person:<ulid>`.
    pub canonical_id: String,
    pub tree: String,
    /// Username of the tree owner.
    pub owner: String,
    pub family_name: String,
    pub first_name: String,
    pub sex: Option<String>,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    /// Confidence score: sex(+3) + dob_year(+2) + place(+2) = max 7, min 0.
    pub score: u32,
    /// `"strong"` (≥4), `"likely"` (2–3), or `"possible"` (0–1).
    pub confidence: String,
    /// True when the match is in a tree owned by the same user (no contact request needed).
    pub is_own: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateSearchResult {
    /// Total number of candidate matches.
    pub count: u64,
    /// Distinct owner usernames across all matches (kept for backward compat).
    pub owners: Vec<String>,
    /// Scored candidate matches, highest confidence first.
    pub matches: Vec<DuplicateMatch>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadContactCount {
    pub count: u64,
}
