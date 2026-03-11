use std::{future::Future, pin::Pin};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};

use crate::{
    db::Db,
    error::{AppError, ErrorResponse},
    models::{
        person::Person,
        relationship::{
            AddRelationshipRequest, FamilyTreeNode, RelationshipType, RelationshipsResponse,
            SiblingType, SpouseInfo, UpdateSpouseDatesRequest,
        },
    },
};

/// Parse a "table:id" string into a SurrealDB `RecordId`.
/// Binding RecordId directly avoids needing `type::thing()` in SurrealQL,
/// which is unsupported in the embedded engine.
fn parse_record_id(s: &str) -> Result<RecordId, AppError> {
    let (tb, id) = s
        .split_once(':')
        .ok_or_else(|| AppError::BadRequest(format!("Expected 'table:id', got: {s}")))?;
    Ok(RecordId::new(tb, id))
}

fn record_id_key(id: &RecordId) -> String {
    match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    }
}

#[derive(Deserialize, SurrealValue)]
struct PersonsRow {
    persons: Vec<Person>,
}

#[derive(Deserialize, SurrealValue)]
struct SiblingsRow {
    out_siblings: Vec<Person>,
    in_siblings: Vec<Person>,
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/relationships",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = AddRelationshipRequest,
    responses(
        (status = 201, description = "Relationship added"),
        (status = 400, description = "Bad request — e.g. missing sibling_type", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn add_relationship(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(payload): Json<AddRelationshipRequest>,
) -> Result<StatusCode, AppError> {
    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&payload.related_id)?;

    match payload.rel_type {
        RelationshipType::Father => {
            db.query("RELATE $from->has_father->$to")
                .bind(("from", from))
                .bind(("to", to))
                .await?;
        }
        RelationshipType::Mother => {
            db.query("RELATE $from->has_mother->$to")
                .bind(("from", from))
                .bind(("to", to))
                .await?;
        }
        RelationshipType::Sibling => {
            let sibling_type = payload.sibling_type.ok_or_else(|| {
                AppError::BadRequest("sibling_type required for Sibling relationship".to_string())
            })?;
            let st = match sibling_type {
                SiblingType::Brother => "Brother",
                SiblingType::Sister => "Sister",
            };
            db.query("RELATE $from->has_sibling->$to CONTENT { sibling_type: $st }")
                .bind(("from", from))
                .bind(("to", to))
                .bind(("st", st))
                .await?;
        }
        RelationshipType::Spouse => {
            // Add both directions so either person can find the other via a forward query.
            let sp_from = payload.spouse_from.as_deref().unwrap_or_default().to_string();
            let sp_to = payload.spouse_to.as_deref().unwrap_or_default().to_string();
            db.query("RELATE $from->has_spouse->$to CONTENT { spouse_from: $sf, spouse_to: $st }")
                .bind(("from", from.clone()))
                .bind(("to", to.clone()))
                .bind(("sf", sp_from.clone()))
                .bind(("st", sp_to.clone()))
                .await?;
            db.query("RELATE $to->has_spouse->$from CONTENT { spouse_from: $sf, spouse_to: $st }")
                .bind(("from", from))
                .bind(("to", to))
                .bind(("sf", sp_from))
                .bind(("st", sp_to))
                .await?;
        }
    }

    Ok(StatusCode::CREATED)
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/relationships",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Grouped relationships", body = RelationshipsResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_relationships(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<RelationshipsResponse>, AppError> {
    let person_id = RecordId::new("person", id.as_str());

    let mut father_res = db
        .query("SELECT ->has_father->person.* AS persons FROM $id")
        .bind(("id", person_id.clone()))
        .await?;
    let father: Vec<Person> = father_res
        .take::<Vec<PersonsRow>>(0)?
        .into_iter()
        .flat_map(|r| r.persons)
        .collect();

    let mut mother_res = db
        .query("SELECT ->has_mother->person.* AS persons FROM $id")
        .bind(("id", person_id.clone()))
        .await?;
    let mother: Vec<Person> = mother_res
        .take::<Vec<PersonsRow>>(0)?
        .into_iter()
        .flat_map(|r| r.persons)
        .collect();

    // Siblings: query both directions to cover symmetric relationships.
    let mut sib_res = db
        .query(
            "SELECT ->has_sibling->person.* AS out_siblings, <-has_sibling<-person.* AS in_siblings FROM $id",
        )
        .bind(("id", person_id.clone()))
        .await?;
    let siblings: Vec<Person> = sib_res
        .take::<Vec<SiblingsRow>>(0)?
        .into_iter()
        .flat_map(|r| r.out_siblings.into_iter().chain(r.in_siblings))
        .collect();

    // Spouses: bidirectional edges exist, so forward query is sufficient.
    let spouse = fetch_spouses(&db, &person_id).await?;

    Ok(Json(RelationshipsResponse { father, mother, siblings, spouse }))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("rel_type" = String, Path, description = "Relationship table: `has_father`, `has_mother`, `has_sibling`, or `has_spouse`"),
        ("related_id" = String, Path, description = "Full related record ID, e.g. `person:01jd4a8xyz`"),
    ),
    responses(
        (status = 204, description = "Relationship deleted"),
        (status = 400, description = "Invalid relationship type", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn delete_relationship(
    State(db): State<Db>,
    Path((id, rel_type, related_id)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    // Validate rel_type against the whitelist before embedding it in the query string.
    let valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&related_id)?;

    if matches!(valid, RelationshipType::Spouse) {
        // Spouse edges are bidirectional — delete both directions.
        let q = format!(
            "DELETE {rel_type} WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)"
        );
        db.query(q).bind(("from", from)).bind(("to", to)).await?;
    } else {
        let query = format!("DELETE {} WHERE in = $from AND out = $to", rel_type);
        db.query(query).bind(("from", from)).bind(("to", to)).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/family-tree",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (
            status = 200,
            description = "Family tree up to 2 generations deep. \
                           Each node: `{ id, family_name, first_name, \
                           father: [node…], mother: [node…], children: [node…], spouse: [node…] }`"
        ),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_family_tree(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<FamilyTreeNode>, AppError> {
    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;

    let node = build_tree_node(db, person, 2, true).await?;
    Ok(Json(node))
}

/// Recursively build a `FamilyTreeNode` up to `depth` generations.
/// `include_root_extras` is true only for the root call — fetches children and spouses.
fn build_tree_node(
    db: Db,
    person: Person,
    depth: u8,
    include_root_extras: bool,
) -> Pin<Box<dyn Future<Output = Result<FamilyTreeNode, AppError>> + Send>> {
    Box::pin(async move {
        let (children, spouse) = if include_root_extras {
            let child_persons = fetch_children(&db, &person.id).await?;
            let spouse_persons = fetch_spouses(&db, &person.id).await?;

            let mut child_nodes = Vec::new();
            for p in child_persons {
                child_nodes.push(build_tree_node(db.clone(), p, 0, false).await?);
            }
            let mut spouse_nodes = Vec::new();
            for s in spouse_persons {
                spouse_nodes.push(build_tree_node(db.clone(), s.person, 0, false).await?);
            }
            (child_nodes, spouse_nodes)
        } else {
            (vec![], vec![])
        };

        if depth == 0 {
            return Ok(FamilyTreeNode {
                id: person.id,
                family_name: person.family_name,
                first_name: person.first_name,
                image_path: person.image_path,
                father: vec![],
                mother: vec![],
                children,
                spouse,
            });
        }

        let father_persons = fetch_relatives(&db, &person.id, "has_father").await?;
        let mother_persons = fetch_relatives(&db, &person.id, "has_mother").await?;

        let mut father = Vec::new();
        for p in father_persons {
            father.push(build_tree_node(db.clone(), p, depth - 1, false).await?);
        }

        let mut mother = Vec::new();
        for p in mother_persons {
            mother.push(build_tree_node(db.clone(), p, depth - 1, false).await?);
        }

        Ok(FamilyTreeNode {
            id: person.id,
            family_name: person.family_name,
            first_name: person.first_name,
            image_path: person.image_path,
            father,
            mother,
            children,
            spouse,
        })
    })
}

async fn fetch_relatives(db: &Db, from: &RecordId, rel: &str) -> Result<Vec<Person>, AppError> {
    let query = format!("SELECT ->{}->person.* AS persons FROM $id", rel);
    let mut res = db.query(query).bind(("id", from.clone())).await?;
    let rows: Vec<PersonsRow> = res.take(0)?;
    Ok(rows.into_iter().flat_map(|r| r.persons).collect())
}

/// Fetch all people who have `parent_id` as their father or mother.
async fn fetch_children(db: &Db, parent_id: &RecordId) -> Result<Vec<Person>, AppError> {
    let mut father_res = db
        .query("SELECT <-has_father<-person.* AS persons FROM $id")
        .bind(("id", parent_id.clone()))
        .await?;
    let mut mother_res = db
        .query("SELECT <-has_mother<-person.* AS persons FROM $id")
        .bind(("id", parent_id.clone()))
        .await?;

    let mut all: Vec<Person> = father_res
        .take::<Vec<PersonsRow>>(0)?
        .into_iter()
        .flat_map(|r| r.persons)
        .chain(
            mother_res
                .take::<Vec<PersonsRow>>(0)?
                .into_iter()
                .flat_map(|r| r.persons),
        )
        .collect();

    // Deduplicate by ULID key.
    let mut seen = std::collections::HashSet::new();
    all.retain(|p| seen.insert(record_id_key(&p.id)));

    Ok(all)
}

/// Fetch spouses with edge date attributes.
/// Because `has_spouse` edges are added bidirectionally, a forward query is sufficient.
async fn fetch_spouses(db: &Db, person_id: &RecordId) -> Result<Vec<SpouseInfo>, AppError> {
    #[derive(serde::Deserialize, surrealdb::types::SurrealValue)]
    struct SpouseEdgeRow {
        person: Person,
        spouse_from: Option<String>,
        spouse_to: Option<String>,
    }

    // Build a sub-object for the person to avoid field name collisions with the edge's own `id`.
    let query = "SELECT \
        { id: out.id, family_name: out.family_name, first_name: out.first_name, \
          middle_name: out.middle_name, sex: out.sex, date_of_birth: out.date_of_birth, \
          place_of_birth: out.place_of_birth, date_of_death: out.date_of_death, \
          place_of_death: out.place_of_death, image_path: out.image_path } AS person, \
        spouse_from, spouse_to \
        FROM has_spouse WHERE in = $id";

    let mut res = db.query(query).bind(("id", person_id.clone())).await?;
    let rows: Vec<SpouseEdgeRow> = res.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| SpouseInfo { person: r.person, spouse_from: r.spouse_from, spouse_to: r.spouse_to })
        .collect())
}

#[utoipa::path(
    patch,
    path = "/api/persons/{id}/spouse-dates/{related_id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("related_id" = String, Path, description = "Full related record ID, e.g. `person:01jd4a8xyz`"),
    ),
    request_body = UpdateSpouseDatesRequest,
    responses(
        (status = 204, description = "Spouse dates updated"),
        (status = 400, description = "Bad request", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn update_spouse_dates(
    State(db): State<Db>,
    Path((id, related_id)): Path<(String, String)>,
    Json(payload): Json<UpdateSpouseDatesRequest>,
) -> Result<StatusCode, AppError> {
    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&related_id)?;
    db.query(
        "UPDATE has_spouse SET spouse_from = $sf, spouse_to = $st \
         WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)",
    )
    .bind(("from", from))
    .bind(("to", to))
    .bind(("sf", payload.spouse_from))
    .bind(("st", payload.spouse_to))
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
