use serde::{Deserialize, Serialize};
use surrealdb::types::{Kind, KindLiteral, RecordId, SurrealValue, Value};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub enum Sex {
    Male,
    Female,
}

impl SurrealValue for Sex {
    fn kind_of() -> Kind {
        Kind::Either(vec![
            Kind::Literal(KindLiteral::String("Male".to_string())),
            Kind::Literal(KindLiteral::String("Female".to_string())),
        ])
    }

    fn into_value(self) -> Value {
        Value::String(match self {
            Sex::Male => "Male".to_string(),
            Sex::Female => "Female".to_string(),
        })
    }

    fn from_value(value: Value) -> Result<Self, surrealdb::types::Error> {
        let Value::String(s) = value else {
            return Err(surrealdb::types::Error::internal(
                "Failed to decode Sex: expected a string".to_string(),
            ));
        };
        match s.as_str() {
            "Male" => Ok(Sex::Male),
            "Female" => Ok(Sex::Female),
            other => Err(surrealdb::types::Error::internal(format!(
                "Failed to decode Sex, no variants matched: `{other}`"
            ))),
        }
    }
}

pub fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    use surrealdb::types::RecordIdKey;
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

/// Canonical person record — verifiable identity hub.
/// Fields: birth certificate name, biological sex, vital dates/places, birth cert reference.
/// Designed to accumulate verifiable facts (DNA, medical, documents) in future versions.
///
/// Enrichment (biography, images, preferred display names) lives on `PersonProxy`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct Person {
    /// Record ID in the form `person:<ulid>`.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Birth certificate family name.
    pub family_name: String,
    /// Birth certificate first name.
    pub first_name: String,
    pub middle_name: Option<String>,
    /// Biological sex — immutable canonical fact.
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
    /// Issuing authority of the birth certificate (e.g. "GRO Ireland").
    pub birth_cert_authority: Option<String>,
    /// Unique number on the birth certificate within the issuing authority.
    pub birth_cert_number: Option<String>,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
}

/// Tree-local view of a canonical person.
/// Holds preferred display name overrides, biography, images, and privacy flag.
/// Relationships (has_father, has_mother, etc.) are edges between proxies.
#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct PersonProxy {
    /// Record ID in the form `person_proxy:<ulid>`.
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Canonical person this proxy represents.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,
    pub tree: String,
    /// Optional preferred display name overrides (null = show canonical name).
    pub preferred_family_name: Option<String>,
    pub preferred_first_name: Option<String>,
    pub preferred_middle_name: Option<String>,
    pub nickname: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub verified: bool,
    pub biography: Option<String>,
    pub image_path: Option<String>,
    pub life_image_path: Option<String>,
    pub image_bytes: Option<i64>,
    pub life_image_bytes: Option<i64>,
    /// When true, non-members see only a stub (sex + existence) for graph structure.
    #[serde(default)]
    pub is_private: bool,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
}

/// Heterogeneous person response — full data or privacy stub.
/// Used by endpoints that may return either depending on the caller's access.
#[derive(Debug, Serialize, ToSchema)]
#[serde(untagged)]
pub enum PersonResponse {
    Full(PersonProxyResponse),
    Stub(PersonProxyStub),
}

/// Flattened API response combining proxy and canonical data.
/// Constructed in Rust via `build_proxy_response` — never deserialized from DB directly.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PersonProxyResponse {
    /// Proxy record ID (`person_proxy:<ulid>`).
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    /// Canonical person ID (`person:<ulid>`).
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,
    pub tree: String,
    /// Effective display name: `preferred_family_name ?? canonical.family_name`.
    pub family_name: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    /// Explicit preferred overrides (null = use canonical).
    pub preferred_family_name: Option<String>,
    pub preferred_first_name: Option<String>,
    pub preferred_middle_name: Option<String>,
    /// Canonical facts surfaced at top level for convenience.
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
    /// Proxy-local enrichment.
    pub nickname: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub verified: bool,
    pub biography: Option<String>,
    pub image_path: Option<String>,
    pub life_image_path: Option<String>,
    pub image_bytes: Option<i64>,
    pub life_image_bytes: Option<i64>,
    #[serde(default)]
    pub is_private: bool,
    pub created_by: Option<String>,
    pub created_at: Option<String>,
    /// Full canonical record for the detail view.
    pub canonical: Person,
}

/// Privacy-redacted stub returned to non-members for private cross-tree nodes.
/// Only sex and existence are exposed (enough to render a gendered placeholder in the tree UI).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PersonProxyStub {
    #[schema(value_type = String, example = "person_proxy:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub person_id: RecordId,
    pub tree: String,
    pub sex: Sex,
    pub is_private: bool,
}

/// Build a `PersonProxyResponse` from its constituent parts.
pub fn build_proxy_response(proxy: PersonProxy, canonical: Person) -> PersonProxyResponse {
    let family_name = proxy.preferred_family_name.clone()
        .unwrap_or_else(|| canonical.family_name.clone());
    let first_name = proxy.preferred_first_name.clone()
        .unwrap_or_else(|| canonical.first_name.clone());
    let middle_name = proxy.preferred_middle_name.clone()
        .or_else(|| canonical.middle_name.clone());
    let sex = canonical.sex.clone();
    let date_of_birth = canonical.date_of_birth.clone();
    let place_of_birth = canonical.place_of_birth.clone();
    let date_of_death = canonical.date_of_death.clone();
    let place_of_death = canonical.place_of_death.clone();
    PersonProxyResponse {
        id: proxy.id,
        person_id: proxy.person_id,
        tree: proxy.tree,
        family_name,
        first_name,
        middle_name,
        preferred_family_name: proxy.preferred_family_name,
        preferred_first_name: proxy.preferred_first_name,
        preferred_middle_name: proxy.preferred_middle_name,
        sex,
        date_of_birth,
        place_of_birth,
        date_of_death,
        place_of_death,
        nickname: proxy.nickname,
        username: proxy.username,
        email: proxy.email,
        verified: proxy.verified,
        biography: proxy.biography,
        image_path: proxy.image_path,
        life_image_path: proxy.life_image_path,
        image_bytes: proxy.image_bytes,
        life_image_bytes: proxy.life_image_bytes,
        is_private: proxy.is_private,
        created_by: proxy.created_by,
        created_at: proxy.created_at,
        canonical,
    }
}

/// Request body for `POST /api/persons`.
/// Contains both canonical identity fields and proxy enrichment fields.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePerson {
    // Canonical required
    pub family_name: String,
    pub first_name: String,
    pub sex: Sex,
    // Canonical optional
    pub middle_name: Option<String>,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
    pub birth_cert_authority: Option<String>,
    pub birth_cert_number: Option<String>,
    // Proxy required
    pub tree: String,
    // Proxy optional
    pub preferred_family_name: Option<String>,
    pub preferred_first_name: Option<String>,
    pub preferred_middle_name: Option<String>,
    pub nickname: Option<String>,
    pub username: Option<String>,
    pub email: Option<String>,
    pub biography: Option<String>,
    pub is_private: Option<bool>,
    pub created_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TreeMembershipRequest {
    /// Tree name (slug) to add or remove.
    pub tree: String,
}

/// Proxy fields only — does NOT update canonical identity fields.
/// Use `PATCH /api/persons/{id}/canonical` for canonical updates.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct UpdatePersonProxy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_family_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_middle_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biography: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

/// Canonical identity fields — all optional (MERGE semantics).
/// Any tree proxy owner may update canonical facts.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateCanonicalPerson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<Sex>,
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
