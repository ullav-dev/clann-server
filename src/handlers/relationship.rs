use std::{future::Future, pin::Pin};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::{RecordId, SurrealValue};

use crate::{
    db::Db,
    error::{AppError, ErrorResponse},
    models::{
        person::Person,
        relationship::{AddRelationshipRequest, FamilyTreeNode, RelationshipType, RelationshipsResponse, SiblingType},
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
        .bind(("id", person_id))
        .await?;
    let siblings: Vec<Person> = sib_res
        .take::<Vec<SiblingsRow>>(0)?
        .into_iter()
        .flat_map(|r| r.out_siblings.into_iter().chain(r.in_siblings))
        .collect();

    Ok(Json(RelationshipsResponse { father, mother, siblings }))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("rel_type" = String, Path, description = "Relationship table: `has_father`, `has_mother`, or `has_sibling`"),
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
    let _valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&related_id)?;

    let query = format!("DELETE {} WHERE in = $from AND out = $to", rel_type);
    db.query(query)
        .bind(("from", from))
        .bind(("to", to))
        .await?;

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
                           father: [node…], mother: [node…] }`"
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

/// Recursively build a `FamilyTreeNode` up to `depth` generations using simple
/// per-relation queries rather than a single nested SurrealQL projection.
/// `include_children` is true only for the root call — it fetches people who
/// have this person as their father or mother and attaches them as leaf nodes.
fn build_tree_node(
    db: Db,
    person: Person,
    depth: u8,
    include_children: bool,
) -> Pin<Box<dyn Future<Output = Result<FamilyTreeNode, AppError>> + Send>> {
    Box::pin(async move {
        let children = if include_children {
            let child_persons = fetch_children(&db, &person.id).await?;
            let mut nodes = Vec::new();
            for p in child_persons {
                nodes.push(build_tree_node(db.clone(), p, 0, false).await?);
            }
            nodes
        } else {
            vec![]
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

    // Deduplicate by ULID key in case a person appears via both has_father and has_mother.
    let mut seen = std::collections::HashSet::new();
    all.retain(|p| {
        let key = match &p.id.key {
            surrealdb::types::RecordIdKey::String(k) => k.clone(),
            other => format!("{other:?}"),
        };
        seen.insert(key)
    });

    Ok(all)
}
