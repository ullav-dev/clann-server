use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::RecordId;

use crate::{
    db::Db,
    error::{AppError, ErrorResponse},
    models::{family_tree::FamilyTree, person::{CreatePerson, Person, TreeMembershipRequest, UpdatePerson}},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PersonFilter {
    /// When present, only return/allow access to persons with this `created_by` value.
    pub created_by: Option<String>,
    /// When present, only return persons belonging to this family tree.
    pub tree: Option<String>,
}

/// Returns true if `username` matches the configured admin username
/// (`ADMIN_USERNAME` env var, defaulting to `"theboss"`).
fn is_admin(username: &str) -> bool {
    let admin = std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "theboss".to_string());
    username == admin
}

/// Returns `AppError::NotFound` if `created_by` is set, is not the admin
/// username, and does not match the person's `created_by` field.
pub fn check_ownership(person: &Person, created_by: &Option<String>) -> Result<(), AppError> {
    if let Some(ref creator) = created_by {
        if !is_admin(creator) && person.created_by.as_deref() != Some(creator.as_str()) {
            return Err(AppError::NotFound);
        }
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/api/persons",
    request_body = CreatePerson,
    responses(
        (status = 201, description = "Person created", body = Person),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 422, description = "Invalid request body", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn create_person(
    State(db): State<Db>,
    Json(payload): Json<CreatePerson>,
) -> Result<(StatusCode, Json<Person>), AppError> {
    let db = db.lock().await;

    if payload.trees.is_empty() {
        return Err(AppError::BadRequest("At least one tree must be specified".to_string()));
    }

    // Validate all specified trees exist
    for tree_name in &payload.trees {
        let tree_exists: Option<FamilyTree> = db
            .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
            .bind(("name", tree_name.clone()))
            .await?
            .take(0)?;
        if tree_exists.is_none() {
            return Err(AppError::BadRequest(format!(
                "Family tree '{}' not found",
                tree_name
            )));
        }
    }

    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let person: Option<Person> = db.create("person").content(body).await?;
    let person = person.ok_or(AppError::BadRequest("Failed to create person".to_string()))?;
    Ok((StatusCode::CREATED, Json(person)))
}

#[utoipa::path(
    get,
    path = "/api/persons",
    params(PersonFilter),
    responses(
        (status = 200, description = "List of persons, optionally filtered", body = Vec<Person>),
    ),
    tag = "persons"
)]
pub async fn list_persons(
    State(db): State<Db>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<Vec<Person>>, AppError> {
    let db = db.lock().await;
    let creator_filter = filter.created_by.as_deref().filter(|c| !is_admin(c));

    let persons: Vec<Person> = match (creator_filter, filter.tree.as_deref()) {
        (Some(creator), Some(tree)) => db
            .query("SELECT * FROM person WHERE created_by = $creator AND $tree IN trees")
            .bind(("creator", creator.to_string()))
            .bind(("tree", tree.to_string()))
            .await?
            .take(0)?,
        (Some(creator), None) => db
            .query("SELECT * FROM person WHERE created_by = $creator")
            .bind(("creator", creator.to_string()))
            .await?
            .take(0)?,
        (None, Some(tree)) => db
            .query("SELECT * FROM person WHERE $tree IN trees")
            .bind(("tree", tree.to_string()))
            .await?
            .take(0)?,
        (None, None) => db.select("person").await?,
    };
    Ok(Json(persons))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Person found", body = Person),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn get_person(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<Person>, AppError> {
    let db = db.lock().await;
    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;
    Ok(Json(person))
}

#[utoipa::path(
    put,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = UpdatePerson,
    responses(
        (status = 200, description = "Person updated", body = Person),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn update_person(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<UpdatePerson>,
) -> Result<Json<Person>, AppError> {
    let db = db.lock().await;
    if filter.created_by.is_some() {
        let check: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&check.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let person: Option<Person> = db.update(("person", id.as_str())).merge(body).await?;
    let person = person.ok_or(AppError::NotFound)?;
    Ok(Json(person))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 204, description = "Person deleted"),
    ),
    tag = "persons"
)]
pub async fn delete_person(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    if filter.created_by.is_some() {
        let check: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&check.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    let _: Option<Person> = db.delete(("person", id.as_str())).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/trees",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = TreeMembershipRequest,
    responses(
        (status = 204, description = "Person added to tree"),
        (status = 400, description = "Tree not found", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn add_person_to_tree(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<TreeMembershipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    let tree_exists: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", payload.tree.clone()))
        .await?
        .take(0)?;
    if tree_exists.is_none() {
        return Err(AppError::BadRequest(format!(
            "Family tree '{}' not found",
            payload.tree
        )));
    }

    db.query("UPDATE $person SET trees = array::union(trees, [$tree])")
        .bind(("person", RecordId::new("person", id.as_str())))
        .bind(("tree", payload.tree))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/trees/{tree_name}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("tree_name" = String, Path, description = "Tree name (slug) to remove"),
    ),
    responses(
        (status = 204, description = "Person removed from tree"),
        (status = 400, description = "Cannot remove from last tree", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn remove_person_from_tree(
    State(db): State<Db>,
    Path((id, tree_name)): Path<(String, String)>,
    Query(filter): Query<PersonFilter>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    if person.trees.len() <= 1 {
        return Err(AppError::BadRequest(
            "Cannot remove person from their last tree".to_string(),
        ));
    }

    db.query("UPDATE $person SET trees -= $tree")
        .bind(("person", RecordId::new("person", id.as_str())))
        .bind(("tree", tree_name))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
