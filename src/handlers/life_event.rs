use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::RecordId;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::life_event::{CreateLifeEvent, LifeEvent, UpdateLifeEvent},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LifeEventFilter {
    /// Filter by the user who created the events.
    pub created_by: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/life-events",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = CreateLifeEvent,
    responses(
        (status = 201, description = "Life event created", body = LifeEvent),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn create_life_event(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(person_id): Path<String>,
    Json(payload): Json<CreateLifeEvent>,
) -> Result<(StatusCode, Json<LifeEvent>), AppError> {
    let db = db.lock().await;

    let person_rid = RecordId::new("person", person_id.as_str());

    // Verify the person exists
    let person_exists: Option<serde_json::Value> = db
        .query("SELECT id FROM person WHERE id = $pid LIMIT 1")
        .bind(("pid", person_rid.clone()))
        .await?
        .take(0)?;
    if person_exists.is_none() {
        return Err(AppError::NotFound);
    }

    // Build the content map, injecting person_id from the path
    let mut body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    body["person_id"] = serde_json::json!(format!("person:{}", person_id));

    let event: Option<LifeEvent> = db.create("life_event").content(body).await?;
    event
        .map(|e| (StatusCode::CREATED, Json(e)))
        .ok_or_else(|| AppError::BadRequest("Failed to create life event".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/life-events",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        LifeEventFilter
    ),
    responses(
        (status = 200, description = "List of life events for the person", body = Vec<LifeEvent>),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn list_life_events(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(person_id): Path<String>,
    Query(filter): Query<LifeEventFilter>,
) -> Result<Json<Vec<LifeEvent>>, AppError> {
    let db = db.lock().await;

    let person_rid = RecordId::new("person", person_id.as_str());

    // Verify person exists
    let person_exists: Option<serde_json::Value> = db
        .query("SELECT id FROM person WHERE id = $pid LIMIT 1")
        .bind(("pid", person_rid.clone()))
        .await?
        .take(0)?;
    if person_exists.is_none() {
        return Err(AppError::NotFound);
    }

    let events: Vec<LifeEvent> = if let Some(creator) = filter.created_by {
        db.query(
            "SELECT * FROM life_event WHERE person_id = $pid AND created_by = $creator ORDER BY date ASC",
        )
        .bind(("pid", person_rid))
        .bind(("creator", creator))
        .await?
        .take(0)?
    } else {
        db.query("SELECT * FROM life_event WHERE person_id = $pid ORDER BY date ASC")
            .bind(("pid", person_rid))
            .await?
            .take(0)?
    };

    Ok(Json(events))
}

#[utoipa::path(
    get,
    path = "/api/life-events/{event_id}",
    params(
        ("event_id" = String, Path, description = "Life event ID (without the `life_event:` prefix)")
    ),
    responses(
        (status = 200, description = "Life event", body = LifeEvent),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn get_life_event(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
) -> Result<Json<LifeEvent>, AppError> {
    let db = db.lock().await;

    let eid = RecordId::new("life_event", event_id.as_str());
    let event: Option<LifeEvent> = db.select(eid).await?;
    event.map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    put,
    path = "/api/life-events/{event_id}",
    params(
        ("event_id" = String, Path, description = "Life event ID (without the `life_event:` prefix)")
    ),
    request_body = UpdateLifeEvent,
    responses(
        (status = 200, description = "Updated life event", body = LifeEvent),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn update_life_event(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
    Json(payload): Json<UpdateLifeEvent>,
) -> Result<Json<LifeEvent>, AppError> {
    let db = db.lock().await;

    let eid = RecordId::new("life_event", event_id.as_str());

    // Check it exists first
    let existing: Option<LifeEvent> = db.select(eid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let updated: Option<LifeEvent> = db.update(eid).merge(body).await?;
    updated.map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    delete,
    path = "/api/life-events/{event_id}",
    params(
        ("event_id" = String, Path, description = "Life event ID (without the `life_event:` prefix)")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn delete_life_event(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let eid = RecordId::new("life_event", event_id.as_str());
    let existing: Option<LifeEvent> = db.select(eid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    let _: Option<LifeEvent> = db.delete(eid).await?;
    Ok(StatusCode::NO_CONTENT)
}
