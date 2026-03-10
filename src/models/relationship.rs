use serde::{Deserialize, Serialize};
use surrealdb::types::RecordId;
use utoipa::ToSchema;

use super::person::Person;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RelationshipType {
    Father,
    Mother,
    Sibling,
}

impl RelationshipType {
    pub fn table_name(&self) -> &'static str {
        match self {
            RelationshipType::Father => "has_father",
            RelationshipType::Mother => "has_mother",
            RelationshipType::Sibling => "has_sibling",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "has_father" => Some(Self::Father),
            "has_mother" => Some(Self::Mother),
            "has_sibling" => Some(Self::Sibling),
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
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipsResponse {
    pub father: Vec<Person>,
    pub mother: Vec<Person>,
    pub siblings: Vec<Person>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FamilyTreeNode {
    /// Record ID in the form `person:<ulid>`.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    pub id: RecordId,
    pub family_name: String,
    pub first_name: String,
    /// Father's node with their ancestors (2 generations deep).
    #[serde(default)]
    pub father: Vec<FamilyTreeNode>,
    /// Mother's node with their ancestors (2 generations deep).
    #[serde(default)]
    pub mother: Vec<FamilyTreeNode>,
}
