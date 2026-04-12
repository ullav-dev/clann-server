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

/// SELECT projection that casts the record-typed person_id to a string so the
/// SurrealDB client can deserialise it into `LifeEvent.person_id: String`.
const SELECT_FIELDS: &str =
    "SELECT *, <string>person_id AS person_id FROM life_event";

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

    // CREATE and immediately re-SELECT with the string cast.
    // We use a LET + RETURN so we get back the new record ID for the re-fetch.
    let mut res = db
        .query(
            "LET $new = (CREATE life_event CONTENT $data)[0]; \
             RETURN SELECT *, <string>person_id AS person_id FROM life_event WHERE id = $new.id;",
        )
        .bind(("data", serde_json::json!({
            "person_id": person_rid,
            "name": payload.name,
            "date": payload.date,
            "event_type": payload.event_type,
            "description": payload.description,
            "story": payload.story,
            "verified": payload.verified,
            "source_link": payload.source_link,
            "source_image": payload.source_image,
            "source_doc": payload.source_doc,
            "created_by": payload.created_by,
        })))
        .await?;

    let events: Vec<LifeEvent> = res.take(1)?;
    events
        .into_iter()
        .next()
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
            &format!("{SELECT_FIELDS} WHERE person_id = $pid AND created_by = $creator ORDER BY date ASC"),
        )
        .bind(("pid", person_rid))
        .bind(("creator", creator))
        .await?
        .take(0)?
    } else {
        db.query(&format!("{SELECT_FIELDS} WHERE person_id = $pid ORDER BY date ASC"))
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
    let mut res = db
        .query(&format!("{SELECT_FIELDS} WHERE id = $eid LIMIT 1"))
        .bind(("eid", eid))
        .await?;
    let events: Vec<LifeEvent> = res.take(0)?;
    events.into_iter().next().map(Json).ok_or(AppError::NotFound)
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

    // Check existence
    let exists: Vec<serde_json::Value> = db
        .query("SELECT id FROM life_event WHERE id = $eid LIMIT 1")
        .bind(("eid", eid.clone()))
        .await?
        .take(0)?;
    if exists.is_empty() {
        return Err(AppError::NotFound);
    }

    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;

    // MERGE then re-SELECT with cast
    let mut res = db
        .query(
            "UPDATE $eid MERGE $data; \
             SELECT *, <string>person_id AS person_id FROM life_event WHERE id = $eid LIMIT 1;",
        )
        .bind(("eid", eid))
        .bind(("data", body))
        .await?;

    let events: Vec<LifeEvent> = res.take(1)?;
    events.into_iter().next().map(Json).ok_or(AppError::NotFound)
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

    // Check existence
    let exists: Vec<serde_json::Value> = db
        .query("SELECT id FROM life_event WHERE id = $eid LIMIT 1")
        .bind(("eid", eid.clone()))
        .await?
        .take(0)?;
    if exists.is_empty() {
        return Err(AppError::NotFound);
    }

    let _: Option<serde_json::Value> = db.delete(eid).await?;
    Ok(StatusCode::NO_CONTENT)
}
