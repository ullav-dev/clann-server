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

fn serialize_record_id<S: serde::Serializer>(id: &RecordId, s: S) -> Result<S::Ok, S::Error> {
    use surrealdb::types::RecordIdKey;
    let key_str = match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    };
    s.serialize_str(&format!("{}:{}", id.table, key_str))
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct Person {
    /// Record ID in the form `person:<ulid>`.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    #[serde(serialize_with = "serialize_record_id")]
    pub id: RecordId,
    pub family_name: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
    pub image_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct CreatePerson {
    pub family_name: String,
    pub first_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_name: Option<String>,
    pub sex: Sex,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_birth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_of_birth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_death: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place_of_death: Option<String>,
}

/// All fields are optional; only supplied fields are updated (MERGE semantics).
#[derive(Debug, Default, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct UpdatePerson {
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
}
