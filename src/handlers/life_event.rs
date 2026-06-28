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
    handlers::person::{can_write_to_proxy, fetch_proxy_and_canonical},
    models::life_event::{CreateLifeEvent, LifeEvent, UpdateLifeEvent},
};

fn record_id_key(id: &RecordId) -> String {
    use surrealdb::types::RecordIdKey;
    match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    }
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/life-events",
    params(("id" = String, Path, description = "Proxy ULID")),
    request_body = CreateLifeEvent,
    responses(
        (status = 201, body = LifeEvent),
        (status = 400, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn create_life_event(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(proxy_id): Path<String>,
    Json(payload): Json<CreateLifeEvent>,
) -> Result<(StatusCode, Json<LifeEvent>), AppError> {
    let db = db.lock().await;
    let (proxy, _) = fetch_proxy_and_canonical(&db, &proxy_id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let proxy_rid = RecordId::new("person_proxy", proxy_id.as_str());
    let is_canonical = payload.is_canonical;
    let contributed_tree = proxy.tree.clone();

    let events: Vec<LifeEvent> = db
        .query(
            "CREATE life_event SET \
             person_proxy_id      = $pid, \
             contributed_by_tree  = $tree, \
             is_canonical         = $is_canonical, \
             name                 = $name, \
             date                 = $date, \
             event_type           = $et, \
             description          = $desc, \
             story                = $story, \
             verified             = $verified, \
             source_link          = $sl, \
             source_image         = $si, \
             source_doc           = $sd, \
             created_by           = $creator",
        )
        .bind(("pid",          proxy_rid))
        .bind(("tree",         contributed_tree))
        .bind(("is_canonical", is_canonical))
        .bind(("name",         payload.name))
        .bind(("date",         payload.date))
        .bind(("et",           payload.event_type))
        .bind(("desc",         payload.description))
        .bind(("story",        payload.story))
        .bind(("verified",     payload.verified))
        .bind(("sl",           payload.source_link))
        .bind(("si",           payload.source_image))
        .bind(("sd",           payload.source_doc))
        .bind(("creator",      payload.created_by))
        .await?
        .take(0)?;

    events
        .into_iter()
        .next()
        .map(|e| (StatusCode::CREATED, Json(e)))
        .ok_or_else(|| AppError::BadRequest("Failed to create life event".to_string()))
}

#[derive(Debug, Deserialize)]
pub struct LifeEventFilter {
    pub created_by: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/life-events",
    params(("id" = String, Path, description = "Proxy ULID")),
    responses(
        (status = 200, body = Vec<LifeEvent>),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn list_life_events(
    State(db): State<Db>,
    Path(proxy_id): Path<String>,
    Query(filter): Query<LifeEventFilter>,
) -> Result<Json<Vec<LifeEvent>>, AppError> {
    let db = db.lock().await;
    let proxy: Option<crate::models::person::PersonProxy> =
        db.select(("person_proxy", proxy_id.as_str())).await?;
    let proxy = proxy.ok_or(AppError::NotFound)?;

    let proxy_rid = RecordId::new("person_proxy", proxy_id.as_str());
    let canonical_id = proxy.person_id.clone();

    // Own events + canonical events from sibling proxies (SurrealDB has no UNION).
    // Uses two separate queries and deduplicates by ID in Rust.
    let mut own_events: Vec<LifeEvent> = if let Some(ref cb) = filter.created_by {
        db.query("SELECT * FROM life_event WHERE person_proxy_id = $pid AND created_by = $cb ORDER BY date ASC")
            .bind(("pid", proxy_rid.clone()))
            .bind(("cb", cb.clone()))
            .await?
            .take(0)?
    } else {
        db.query("SELECT * FROM life_event WHERE person_proxy_id = $pid ORDER BY date ASC")
            .bind(("pid", proxy_rid))
            .await?
            .take(0)?
    };

    let canonical_events: Vec<LifeEvent> = if let Some(ref cb) = filter.created_by {
        db.query(
            "SELECT * FROM life_event \
             WHERE is_canonical = true \
             AND created_by = $cb \
             AND person_proxy_id IN (SELECT VALUE id FROM person_proxy WHERE person_id = $cid) \
             AND person_proxy_id != $self_pid \
             ORDER BY date ASC",
        )
        .bind(("cid",      canonical_id))
        .bind(("self_pid", RecordId::new("person_proxy", proxy_id.as_str())))
        .bind(("cb",       cb.clone()))
        .await?
        .take(0)?
    } else {
        db.query(
            "SELECT * FROM life_event \
             WHERE is_canonical = true \
             AND person_proxy_id IN (SELECT VALUE id FROM person_proxy WHERE person_id = $cid) \
             AND person_proxy_id != $self_pid \
             ORDER BY date ASC",
        )
        .bind(("cid",      canonical_id))
        .bind(("self_pid", RecordId::new("person_proxy", proxy_id.as_str())))
        .await?
        .take(0)?
    };

    // Merge and deduplicate.
    let mut seen = std::collections::HashSet::new();
    for e in &own_events {
        seen.insert(record_id_key(&e.id));
    }
    for e in canonical_events {
        if seen.insert(record_id_key(&e.id)) {
            own_events.push(e);
        }
    }
    own_events.sort_by(|a, b| a.date.cmp(&b.date));

    Ok(Json(own_events))
}

#[utoipa::path(
    get,
    path = "/api/life-events/{event_id}",
    params(("event_id" = String, Path, description = "Life event ULID")),
    responses(
        (status = 200, body = LifeEvent),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn get_life_event(
    State(db): State<Db>,
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

    let proxy_key = record_id_key(&existing.person_proxy_id);
    let proxy: Option<crate::models::person::PersonProxy> =
        db.select(("person_proxy", proxy_key.as_str())).await?;
    can_write_to_proxy(&proxy.ok_or(AppError::NotFound)?, &auth, &db).await?;

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

    let proxy_key = record_id_key(&existing.person_proxy_id);
    let proxy: Option<crate::models::person::PersonProxy> =
        db.select(("person_proxy", proxy_key.as_str())).await?;
    can_write_to_proxy(&proxy.ok_or(AppError::NotFound)?, &auth, &db).await?;

    let _: Option<LifeEvent> = db.delete(eid).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Promote a life event to canonical visibility (shared across all proxies for the same person).
#[utoipa::path(
    patch,
    path = "/api/life-events/{event_id}/promote",
    params(("event_id" = String, Path, description = "Life event ULID")),
    responses(
        (status = 200, description = "Promoted", body = LifeEvent),
        (status = 404, body = ErrorResponse),
    ),
    tag = "life-events"
)]
pub async fn promote_life_event(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(event_id): Path<String>,
) -> Result<Json<LifeEvent>, AppError> {
    let db = db.lock().await;
    let eid = RecordId::new("life_event", event_id.as_str());

    let existing: Option<LifeEvent> = db.select(eid.clone()).await?;
    let existing = existing.ok_or(AppError::NotFound)?;

    let proxy_key = record_id_key(&existing.person_proxy_id);
    let proxy: Option<crate::models::person::PersonProxy> =
        db.select(("person_proxy", proxy_key.as_str())).await?;
    can_write_to_proxy(&proxy.ok_or(AppError::NotFound)?, &auth, &db).await?;

    let updated: Vec<LifeEvent> = db
        .query("UPDATE $eid SET is_canonical = true")
        .bind(("eid", eid))
        .await?
        .take(0)?;

    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}
