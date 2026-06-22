use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
use utoipa::ToSchema;

use crate::models::person::serialize_record_id;

/// Agreed canonical field values supplied when accepting a proposal with conflicts.
/// Any field left as `None` is not overridden — the value from the surviving canonical is kept.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema, Clone, SurrealValue)]
pub struct MergeResolution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_birth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_of_birth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_death: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_of_death: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_cert_authority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_cert_number: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct MergeProposal {
    #[schema(value_type = String, example = "person_merge_proposal:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub from_proxy_id: RecordId,
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub to_proxy_id: RecordId,
    pub proposed_by: String,
    pub proposed_at: String,
    /// `pending`, `accepted`, or `rejected`.
    pub status: String,
    pub resolution: Option<MergeResolution>,
    pub responded_by: Option<String>,
    pub responded_at: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMergeProposalRequest {
    /// ID of the proxy in the proposing tree (`person_proxy:<ulid>`).
    pub from_proxy_id: String,
    /// ID of the proxy in the receiving tree (`person_proxy:<ulid>`).
    pub to_proxy_id: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcceptMergeProposalRequest {
    /// Required only when the two canonicals have conflicting hard fields.
    /// If omitted and conflicts exist, the handler returns 409 with the list of conflicts.
    pub resolution: Option<MergeResolution>,
}
