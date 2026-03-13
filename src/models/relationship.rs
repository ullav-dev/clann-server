use crate::models::person::Sex;
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, RecordIdKey};
use utoipa::ToSchema;

fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

use super::person::Person;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RelationshipType {
    Father,
    Mother,
    Sibling,
    Spouse,
}

impl RelationshipType {
    pub fn table_name(&self) -> &'static str {
        match self {
            RelationshipType::Father => "has_father",
            RelationshipType::Mother => "has_mother",
            RelationshipType::Sibling => "has_sibling",
            RelationshipType::Spouse => "has_spouse",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "has_father" => Some(Self::Father),
            "has_mother" => Some(Self::Mother),
            "has_sibling" => Some(Self::Sibling),
            "has_spouse" => Some(Self::Spouse),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum SiblingType {
    Brother,
    Sister,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddRelationshipRequest {
    #[serde(rename = "type")]
    pub rel_type: RelationshipType,
    /// Full record ID of the related person, e.g. `person:01jd4a8xyz`.
    pub related_id: String,
    /// Required when `type` is `Sibling`.
    pub sibling_type: Option<SiblingType>,
    /// Optional marriage/partnership start date (ISO 8601 or free-form string).
    pub spouse_from: Option<String>,
    /// Optional marriage/partnership end date.
    pub spouse_to: Option<String>,
}

/// A spouse with the edge's date attributes included.
#[derive(Debug, Serialize, ToSchema)]
pub struct SpouseInfo {
    #[serde(flatten)]
    pub person: Person,
    pub spouse_from: Option<String>,
    pub spouse_to: Option<String>,
}

/// Payload for updating spouse date attributes on an existing relationship.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSpouseDatesRequest {
    pub spouse_from: Option<String>,
    pub spouse_to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipsResponse {
    pub father: Vec<Person>,
    pub mother: Vec<Person>,
    pub siblings: Vec<Person>,
    pub spouse: Vec<SpouseInfo>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FamilyTreeNode {
    /// Record ID in the form `person:<ulid>`.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub family_name: String,
    pub first_name: String,
    pub sex: Option<Sex>,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub biography: Option<String>,
    pub image_path: Option<String>,
    /// Father's node with their ancestors (2 generations deep).
    #[serde(default)]
    pub father: Vec<FamilyTreeNode>,
    /// Mother's node with their ancestors (2 generations deep).
    #[serde(default)]
    pub mother: Vec<FamilyTreeNode>,
    /// Direct children (people for whom this node is father or mother).
    /// Only populated for the root node.
    #[serde(default)]
    pub children: Vec<FamilyTreeNode>,
    /// Spouses. Only populated for the root node.
    #[serde(default)]
    pub spouse: Vec<FamilyTreeNode>,
    /// Siblings. Only populated for the root node.
    #[serde(default)]
    pub siblings: Vec<FamilyTreeNode>,
}
