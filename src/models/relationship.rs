use crate::models::person::{serialize_record_id, Sex};
use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
use utoipa::ToSchema;

/// Compact node used as the query projection type in relationship edge queries.
/// Effective names (preferred ?? canonical) are computed in SurrealQL before binding.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct PersonProxyNode {
    /// Proxy record ID (`person_proxy:<ulid>`).
    #[schema(value_type = String)]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Canonical person ID (`person:<ulid>`).
    #[schema(value_type = String)]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,
    pub tree: String,
    /// Effective family name: `preferred_family_name ?? canonical.family_name`.
    pub family_name: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
    pub biography: Option<String>,
    pub image_path: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    pub created_by: Option<String>,
}

/// SurrealQL expression that builds a `PersonProxyNode` sub-object for edge `out`.
/// Paste into a `SELECT {PROXY_NODE_OUT} AS person FROM has_* WHERE ...` query.
pub const PROXY_NODE_OUT: &str = "{ \
    id: out.id, \
    person_id: out.person_id, \
    tree: out.tree, \
    family_name: out.preferred_family_name ?? out.person_id.family_name, \
    first_name: out.preferred_first_name ?? out.person_id.first_name, \
    middle_name: out.preferred_middle_name ?? out.person_id.middle_name, \
    sex: out.person_id.sex, \
    date_of_birth: out.person_id.date_of_birth, \
    place_of_birth: out.person_id.place_of_birth, \
    date_of_death: out.person_id.date_of_death, \
    place_of_death: out.person_id.place_of_death, \
    biography: out.biography, \
    image_path: out.image_path, \
    is_private: out.is_private, \
    created_by: out.created_by \
}";

/// Same as `PROXY_NODE_OUT` but for `in` (child → parent edges, sibling incoming edges).
pub const PROXY_NODE_IN: &str = "{ \
    id: in.id, \
    person_id: in.person_id, \
    tree: in.tree, \
    family_name: in.preferred_family_name ?? in.person_id.family_name, \
    first_name: in.preferred_first_name ?? in.person_id.first_name, \
    middle_name: in.preferred_middle_name ?? in.person_id.middle_name, \
    sex: in.person_id.sex, \
    date_of_birth: in.person_id.date_of_birth, \
    place_of_birth: in.person_id.place_of_birth, \
    date_of_death: in.person_id.date_of_death, \
    place_of_death: in.person_id.place_of_death, \
    biography: in.biography, \
    image_path: in.image_path, \
    is_private: in.is_private, \
    created_by: in.created_by \
}";

/// The nature of a parent–child or sibling relationship.
///
/// Valid values for parent edges (`has_father`, `has_mother`): `birth`, `adopted`.
/// Valid values for sibling edges (`has_sibling`): `birth`, `half`, `adopted`.
///
/// Step and foster relationships are intentionally not stored: a step-parent is
/// already present in the tree as the spouse of a biological parent, and foster
/// relationships carry no genealogical information.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Pedigree {
    #[default]
    Birth,
    Adopted,
    Half,
}

impl Pedigree {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Pedigree::Birth => "birth",
            Pedigree::Adopted => "adopted",
            Pedigree::Half => "half",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "adopted" => Pedigree::Adopted,
            "half" => Pedigree::Half,
            _ => Pedigree::Birth,
        }
    }
}

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
    /// Full record ID of the related proxy, e.g. `person_proxy:01jd4a8xyz`.
    pub related_id: String,
    /// Required when `type` is `Sibling`.
    pub sibling_type: Option<SiblingType>,
    pub spouse_from: Option<String>,
    pub spouse_to: Option<String>,
    #[serde(default)]
    pub pedigree: Pedigree,
    /// For half/adopted siblings: the parent proxy ID through whose family the
    /// relationship is formed (`person_proxy:<ulid>`).
    #[serde(default)]
    pub via_parent_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRelationshipRequest {
    pub pedigree: Pedigree,
    pub via_parent_id: Option<String>,
}

/// A parent with the edge's pedigree qualifier.
#[derive(Debug, Serialize, ToSchema)]
pub struct ParentInfo {
    #[serde(flatten)]
    pub person: PersonProxyNode,
    pub pedigree: Pedigree,
}

/// A sibling with the edge's pedigree qualifier.
#[derive(Debug, Serialize, ToSchema)]
pub struct SiblingInfo {
    #[serde(flatten)]
    pub person: PersonProxyNode,
    pub pedigree: Pedigree,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_parent_id: Option<String>,
}

/// A spouse with the edge's date attributes.
#[derive(Debug, Serialize, ToSchema)]
pub struct SpouseInfo {
    #[serde(flatten)]
    pub person: PersonProxyNode,
    pub spouse_from: Option<String>,
    pub spouse_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSpouseDatesRequest {
    pub spouse_from: Option<String>,
    pub spouse_to: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RelationshipsResponse {
    pub father: Vec<ParentInfo>,
    pub mother: Vec<ParentInfo>,
    pub siblings: Vec<SiblingInfo>,
    pub spouse: Vec<SpouseInfo>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FamilyTreeNode {
    /// Proxy record ID (`person_proxy:<ulid>`).
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Canonical person ID (`person:<ulid>`).
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,
    pub tree: String,
    pub family_name: String,
    pub first_name: String,
    pub sex: Option<Sex>,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub biography: Option<String>,
    pub image_path: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pedigree: Option<Pedigree>,
    #[serde(default)]
    pub father: Vec<FamilyTreeNode>,
    #[serde(default)]
    pub mother: Vec<FamilyTreeNode>,
    #[serde(default)]
    pub children: Vec<FamilyTreeNode>,
    #[serde(default)]
    pub spouse: Vec<FamilyTreeNode>,
    #[serde(default)]
    pub siblings: Vec<FamilyTreeNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_parent_id: Option<String>,
}

impl FamilyTreeNode {
    pub fn from_proxy_node(node: PersonProxyNode) -> Self {
        FamilyTreeNode {
            id: node.id,
            person_id: node.person_id,
            tree: node.tree,
            family_name: node.family_name,
            first_name: node.first_name,
            sex: Some(node.sex),
            date_of_birth: node.date_of_birth,
            place_of_birth: node.place_of_birth,
            biography: node.biography,
            image_path: node.image_path,
            is_private: node.is_private,
            pedigree: None,
            father: vec![],
            mother: vec![],
            children: vec![],
            spouse: vec![],
            siblings: vec![],
            via_parent_id: None,
        }
    }
}
