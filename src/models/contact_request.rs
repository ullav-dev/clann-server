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

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateSearchResult {
    /// Number of possible duplicate records found across all other trees.
    pub count: u64,
    /// Usernames of users who own records that may be duplicates.
    pub owners: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadContactCount {
    pub count: u64,
}
