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
    models::{
        contact_request::{
            AppendContactMessage, CreateContactRequest,
            MergeContactRequest, UnreadContactCount,
        },
        person::PersonProxy,
    },
};

fn parse_proxy_id(s: &str) -> Result<RecordId, AppError> {
    let (tb, id) = s
        .split_once(':')
        .ok_or_else(|| AppError::BadRequest(format!("Expected 'table:id', got: {s}")))?;
    if tb != "person_proxy" {
        return Err(AppError::BadRequest(format!(
            "Expected person_proxy record ID, got table: {tb}"
        )));
    }
    Ok(RecordId::new(tb, id))
}

#[derive(Debug, Deserialize)]
pub struct ContactRequestFilter {
    pub role: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/contact-requests",
    request_body = CreateContactRequest,
    responses(
        (status = 201, description = "Contact requests created", body = Vec<MergeContactRequest>),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Proxy not found", body = ErrorResponse),
    ),
    tag = "contact-requests"
)]
pub async fn create_contact_requests(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreateContactRequest>,
) -> Result<(StatusCode, Json<Vec<MergeContactRequest>>), AppError> {
    if payload.to_users.is_empty() {
        return Err(AppError::BadRequest("to_users must not be empty".to_string()));
    }

    let proxy_rid = parse_proxy_id(&payload.from_proxy_id)?;
    let db = db.lock().await;

    let proxy: Option<PersonProxy> = db.select(proxy_rid.clone()).await?;
    let proxy = proxy.ok_or(AppError::NotFound)?;

    let caller = if auth.username.is_empty() {
        proxy.created_by.clone().unwrap_or_default()
    } else {
        auth.username.clone()
    };

    if let Some(owner) = &proxy.created_by {
        if !auth.username.is_empty() && owner != &auth.username {
            return Err(AppError::Forbidden(
                "You do not own this person proxy".to_string(),
            ));
        }
    }

    let mut created: Vec<MergeContactRequest> = Vec::new();

    for to_user in &payload.to_users {
        // Skip self-contact
        if to_user == &caller {
            continue;
        }

        // Reject duplicate pending request for same proxy + recipient
        let dup: Option<serde_json::Value> = db
            .query(
                "SELECT id FROM merge_contact_request \
                 WHERE status = 'pending' \
                 AND from_proxy_id = $proxy \
                 AND to_user = $to LIMIT 1",
            )
            .bind(("proxy", proxy_rid.clone()))
            .bind(("to", to_user.clone()))
            .await?
            .take(0)?;

        if dup.is_some() {
            continue; // silently skip — already have a pending request
        }

        let mut results: Vec<MergeContactRequest> = db
            .query(
                "CREATE merge_contact_request SET \
                 from_proxy_id = $proxy, \
                 from_user = $from, \
                 to_user = $to, \
                 initial_message = $msg",
            )
            .bind(("proxy", proxy_rid.clone()))
            .bind(("from", caller.clone()))
            .bind(("to", to_user.clone()))
            .bind(("msg", payload.message.clone()))
            .await?
            .take(0)?;

        created.append(&mut results);
    }

    Ok((StatusCode::CREATED, Json(created)))
}

#[utoipa::path(
    get,
    path = "/api/contact-requests",
    params(
        ("role" = Option<String>, Query, description = "Filter: `sent` or `received`. Omit for all.")
    ),
    responses(
        (status = 200, description = "Contact requests", body = Vec<MergeContactRequest>),
    ),
    tag = "contact-requests"
)]
pub async fn list_contact_requests(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Query(filter): Query<ContactRequestFilter>,
) -> Result<Json<Vec<MergeContactRequest>>, AppError> {
    let db = db.lock().await;
    let user = auth.username.clone();

    let mut query = match filter.role.as_deref() {
        Some("sent") => db
            .query(
                "SELECT * FROM merge_contact_request \
                 WHERE from_user = $user \
                 ORDER BY created_at DESC",
            )
            .bind(("user", user))
            .await?,
        Some("received") => db
            .query(
                "SELECT * FROM merge_contact_request \
                 WHERE to_user = $user \
                 ORDER BY created_at DESC",
            )
            .bind(("user", user))
            .await?,
        _ => db
            .query(
                "SELECT * FROM merge_contact_request \
                 WHERE from_user = $user OR to_user = $user \
                 ORDER BY created_at DESC",
            )
            .bind(("user", user))
            .await?,
    };

    let requests: Vec<MergeContactRequest> = query.take(0)?;
    Ok(Json(requests))
}

#[utoipa::path(
    get,
    path = "/api/contact-requests/pending-count",
    responses(
        (status = 200, description = "Count of pending incoming contact requests", body = UnreadContactCount),
    ),
    tag = "contact-requests"
)]
pub async fn get_pending_count(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
) -> Result<Json<UnreadContactCount>, AppError> {
    let db = db.lock().await;

    let mut res = db
        .query(
            "SELECT count() AS count FROM merge_contact_request \
             WHERE to_user = $user AND status = 'pending' \
             GROUP ALL",
        )
        .bind(("user", auth.username.clone()))
        .await?;

    let row: Option<serde_json::Value> = res.take(0)?;
    let count = row
        .as_ref()
        .and_then(|v| v.get("count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Ok(Json(UnreadContactCount { count }))
}

#[utoipa::path(
    patch,
    path = "/api/contact-requests/{id}/accept",
    params(("id" = String, Path, description = "Contact request ULID")),
    responses(
        (status = 200, description = "Accepted", body = MergeContactRequest),
        (status = 403, description = "Not the recipient", body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "contact-requests"
)]
pub async fn accept_contact_request(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<MergeContactRequest>, AppError> {
    let db = db.lock().await;
    let rid = RecordId::new("merge_contact_request", id.as_str());

    let req: Option<MergeContactRequest> = db.select(rid.clone()).await?;
    let req = req.ok_or(AppError::NotFound)?;

    if req.to_user != auth.username {
        return Err(AppError::Forbidden(
            "Only the recipient can accept this request".to_string(),
        ));
    }
    if req.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Request is already {}",
            req.status
        )));
    }

    let mut res = db
        .query(
            "UPDATE $id SET status = 'accepted', updated_at = <string>time::now()",
        )
        .bind(("id", rid))
        .await?;
    let updated: Vec<MergeContactRequest> = res.take(0)?;
    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    patch,
    path = "/api/contact-requests/{id}/ignore",
    params(("id" = String, Path, description = "Contact request ULID")),
    responses(
        (status = 200, description = "Ignored", body = MergeContactRequest),
        (status = 403, description = "Not the recipient", body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "contact-requests"
)]
pub async fn ignore_contact_request(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<MergeContactRequest>, AppError> {
    let db = db.lock().await;
    let rid = RecordId::new("merge_contact_request", id.as_str());

    let req: Option<MergeContactRequest> = db.select(rid.clone()).await?;
    let req = req.ok_or(AppError::NotFound)?;

    if req.to_user != auth.username {
        return Err(AppError::Forbidden(
            "Only the recipient can ignore this request".to_string(),
        ));
    }
    if req.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Request is already {}",
            req.status
        )));
    }

    let mut res = db
        .query(
            "UPDATE $id SET status = 'ignored', updated_at = <string>time::now()",
        )
        .bind(("id", rid))
        .await?;
    let updated: Vec<MergeContactRequest> = res.take(0)?;
    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    post,
    path = "/api/contact-requests/{id}/messages",
    params(("id" = String, Path, description = "Contact request ULID")),
    request_body = AppendContactMessage,
    responses(
        (status = 200, description = "Message appended", body = MergeContactRequest),
        (status = 400, description = "Request not accepted or empty text", body = ErrorResponse),
        (status = 403, description = "Not a participant", body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "contact-requests"
)]
pub async fn append_contact_message(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<AppendContactMessage>,
) -> Result<Json<MergeContactRequest>, AppError> {
    if payload.text.trim().is_empty() {
        return Err(AppError::BadRequest("Message text must not be empty".to_string()));
    }

    let db = db.lock().await;
    let rid = RecordId::new("merge_contact_request", id.as_str());

    let req: Option<MergeContactRequest> = db.select(rid.clone()).await?;
    let req = req.ok_or(AppError::NotFound)?;

    if req.from_user != auth.username && req.to_user != auth.username {
        return Err(AppError::Forbidden(
            "Only participants can send messages".to_string(),
        ));
    }
    if req.status != "accepted" {
        return Err(AppError::BadRequest(
            "Messages can only be sent on accepted requests".to_string(),
        ));
    }

    let mut res = db
        .query(
            "UPDATE $id SET \
             messages += [{from_user: $from, text: $text, sent_at: <string>time::now()}], \
             updated_at = <string>time::now()",
        )
        .bind(("id", rid))
        .bind(("from", auth.username.clone()))
        .bind(("text", payload.text))
        .await?;

    let updated: Vec<MergeContactRequest> = res.take(0)?;
    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}
