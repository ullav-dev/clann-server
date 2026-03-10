use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use crate::{
    db::Db,
    error::{AppError, ErrorResponse},
    models::person::{CreatePerson, Person, UpdatePerson},
};

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
    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let person: Option<Person> = db.create("person").content(body).await?;
    let person = person.ok_or(AppError::BadRequest("Failed to create person".to_string()))?;
    Ok((StatusCode::CREATED, Json(person)))
}

#[utoipa::path(
    get,
    path = "/api/persons",
    responses(
        (status = 200, description = "List of all persons", body = Vec<Person>),
    ),
    tag = "persons"
)]
pub async fn list_persons(State(db): State<Db>) -> Result<Json<Vec<Person>>, AppError> {
    let persons: Vec<Person> = db.select("person").await?;
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
) -> Result<Json<Person>, AppError> {
    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
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
    Json(payload): Json<UpdatePerson>,
) -> Result<Json<Person>, AppError> {
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
) -> Result<StatusCode, AppError> {
    let _: Option<Person> = db.delete(("person", id.as_str())).await?;
    Ok(StatusCode::NO_CONTENT)
}
