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
    handlers::person::can_write_to_person,
    models::{
        life_event::{CreateLifeEvent, LifeEvent, UpdateLifeEvent},
        person::Person,
    },
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LifeEventFilter {
    pub created_by: Option<String>,
}

fn record_id_key(id: &RecordId) -> String {
    use surrealdb::types::RecordIdKey;
    match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    }
}

/// Check person exists using count() — avoids returning a record-typed `id`
/// field into serde_json::Value (which would fail with "Expected any, got record").
async fn person_exists(db: &crate::db::DbConn, person_rid: RecordId) -> Result<bool, AppError> {
    let result: Option<serde_json::Value> = db
        .query("SELECT count() AS n FROM person WHERE id = $pid GROUP ALL")
        .bind(("pid", person_rid))
        .await?
        .take(0)?;
    Ok(result
        .and_then(|v| v.get("n").and_then(|x| x.as_u64()))
        .unwrap_or(0) > 0)
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/life-events",
    params(("id" = String, Path, description = "Person ULID")),
    request_body = CreateLifeEvent,
    responses(
        (status = 201, body = LifeEvent),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn create_life_event(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(person_id): Path<String>,
    Json(payload): Json<CreateLifeEvent>,
) -> Result<(StatusCode, Json<LifeEvent>), AppError> {
    let db = db.lock().await;
    let person_rid = RecordId::new("person", person_id.as_str());

    if !person_exists(&db, person_rid.clone()).await? {
        return Err(AppError::NotFound);
    }

    let person: Option<Person> = db.select(("person", person_id.as_str())).await?;
    can_write_to_person(&person.ok_or(AppError::NotFound)?, &auth, &db).await?;

    // Use a query string so we can bind person_rid as RecordId (keeping the
    // record<person> type in the DB) and get Vec<LifeEvent> back via SurrealValue.
    let events: Vec<LifeEvent> = db
        .query(
            "CREATE life_event SET \
             person_id   = $pid, \
             name        = $name, \
             date        = $date, \
             event_type  = $et, \
             description = $desc, \
             story       = $story, \
             verified    = $verified, \
             source_link = $sl, \
             source_image = $si, \
             source_doc  = $sd, \
             created_by  = $creator",
        )
        .bind(("pid",     person_rid))
        .bind(("name",    payload.name))
        .bind(("date",    payload.date))
        .bind(("et",      payload.event_type))
        .bind(("desc",    payload.description))
        .bind(("story",   payload.story))
        .bind(("verified",payload.verified))
        .bind(("sl",      payload.source_link))
        .bind(("si",      payload.source_image))
        .bind(("sd",      payload.source_doc))
        .bind(("creator", payload.created_by))
        .await?
        .take(0)?;

    events
        .into_iter()
        .next()
        .map(|e| (StatusCode::CREATED, Json(e)))
        .ok_or_else(|| AppError::BadRequest("Failed to create life event".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/life-events",
    params(("id" = String, Path, description = "Person ULID"), LifeEventFilter),
    responses(
        (status = 200, body = Vec<LifeEvent>),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
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

    if !person_exists(&db, person_rid.clone()).await? {
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
    params(("event_id" = String, Path, description = "Life event ULID")),
    responses(
        (status = 200, body = LifeEvent),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
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
    params(("event_id" = String, Path, description = "Life event ULID")),
    request_body = UpdateLifeEvent,
    responses(
        (status = 200, body = LifeEvent),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn update_life_event(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
    Json(payload): Json<UpdateLifeEvent>,
) -> Result<Json<LifeEvent>, AppError> {
    let db = db.lock().await;
    let eid = RecordId::new("life_event", event_id.as_str());

    let existing: Option<LifeEvent> = db.select(eid.clone()).await?;
    let existing = existing.ok_or(AppError::NotFound)?;

    let person_key = record_id_key(&existing.person_id);
    let person: Option<Person> = db.select(("person", person_key.as_str())).await?;
    can_write_to_person(&person.ok_or(AppError::NotFound)?, &auth, &db).await?;

    // Use an explicit UPDATE SET with bound parameters rather than MERGE+JSON.
    // The SDK maps Option::None bindings to SurrealDB NONE (not NULL), which is
    // what option<string> schema fields require to clear a value.  MERGE via
    // serde_json would silently skip None fields (skip_serializing_if) instead
    // of clearing them.
    let updated: Vec<LifeEvent> = db
        .query(
            "UPDATE $eid SET \
             name         = $name, \
             date         = $date, \
             event_type   = $et, \
             description  = $desc, \
             story        = $story, \
             verified     = $verified, \
             source_link  = $sl, \
             source_image = $si, \
             source_doc   = $sd",
        )
        .bind(("eid",      eid))
        .bind(("name",     payload.name))
        .bind(("date",     payload.date))
        .bind(("et",       payload.event_type))
        .bind(("desc",     payload.description))
        .bind(("story",    payload.story))
        .bind(("verified", payload.verified))
        .bind(("sl",       payload.source_link))
        .bind(("si",       payload.source_image))
        .bind(("sd",       payload.source_doc))
        .await?
        .take(0)?;

    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    delete,
    path = "/api/life-events/{event_id}",
    params(("event_id" = String, Path, description = "Life event ULID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn delete_life_event(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    let eid = RecordId::new("life_event", event_id.as_str());
    let existing: Option<LifeEvent> = db.select(eid.clone()).await?;
    let existing = existing.ok_or(AppError::NotFound)?;

    let person_key = record_id_key(&existing.person_id);
    let person: Option<Person> = db.select(("person", person_key.as_str())).await?;
    can_write_to_person(&person.ok_or(AppError::NotFound)?, &auth, &db).await?;

    let _: Option<LifeEvent> = db.delete(eid).await?;
    Ok(StatusCode::NO_CONTENT)
}
