use serde::{Deserialize, Serialize};
use surrealdb::types::{RecordId, SurrealValue};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, SurrealValue)]
pub enum Sex {
    Male,
    Female,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, SurrealValue)]
pub struct Person {
    /// Record ID in the form `person:<ulid>`.
    #[schema(value_type = String, example = "person:01jd4a8xyz")]
    pub id: RecordId,
    pub family_name: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, SurrealValue)]
pub struct CreatePerson {
    pub family_name: String,
    pub first_name: String,
    pub middle_name: Option<String>,
    pub sex: Sex,
    pub date_of_birth: Option<String>,
    pub place_of_birth: Option<String>,
    pub date_of_death: Option<String>,
    pub place_of_death: Option<String>,
}

impl Serialize for CreatePerson {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("CreatePerson", 8)?;
        s.serialize_field("family_name", &self.family_name)?;
        s.serialize_field("first_name", &self.first_name)?;
        s.serialize_field("middle_name", &self.middle_name)?;
        s.serialize_field("sex", &self.sex)?;
        s.serialize_field("date_of_birth", &self.date_of_birth)?;
        s.serialize_field("place_of_birth", &self.place_of_birth)?;
        s.serialize_field("date_of_death", &self.date_of_death)?;
        s.serialize_field("place_of_death", &self.place_of_death)?;
        s.end()
    }
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
