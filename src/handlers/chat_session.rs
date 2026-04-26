use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use surrealdb::types::RecordId;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::chat_session::{AppendMessage, ChatMessage, ChatSession, CreateChatSession},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SessionFilter {
    pub created_by: Option<String>,
    pub tree: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/chat/sessions",
    params(SessionFilter),
    responses(
        (status = 200, body = Vec<ChatSession>),
        (status = 401, body = ErrorResponse),
    ),
    tag = "chat"
)]
pub async fn list_sessions(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Query(filter): Query<SessionFilter>,
) -> Result<Json<Vec<ChatSession>>, AppError> {
    let db = db.lock().await;
    let sessions: Vec<ChatSession> = db
        .query(
            "SELECT * FROM chat_session WHERE created_by = $creator AND ($tree = NONE OR tree = $tree) ORDER BY updated_at DESC",
        )
        .bind(("creator", filter.created_by.unwrap_or_default()))
        .bind(("tree", filter.tree))
        .await?
        .take(0)?;
    Ok(Json(sessions))
}

#[utoipa::path(
    post,
    path = "/api/chat/sessions",
    request_body = CreateChatSession,
    responses(
        (status = 201, body = ChatSession),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "chat"
)]
pub async fn create_session(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Json(payload): Json<CreateChatSession>,
) -> Result<(StatusCode, Json<ChatSession>), AppError> {
    let db = db.lock().await;
    let sessions: Vec<ChatSession> = db
        .query("CREATE chat_session SET title = $title, created_by = $creator, tree = $tree")
        .bind(("title", payload.title))
        .bind(("creator", payload.created_by))
        .bind(("tree", payload.tree))
        .await?
        .take(0)?;
    sessions
        .into_iter()
        .next()
        .map(|s| (StatusCode::CREATED, Json(s)))
        .ok_or_else(|| AppError::BadRequest("Failed to create session".to_string()))
}

#[utoipa::path(
    delete,
    path = "/api/chat/sessions/{id}",
    params(("id" = String, Path, description = "Session ULID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "chat"
)]
pub async fn delete_session(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    let sid = RecordId::new("chat_session", session_id.as_str());
    let existing: Option<ChatSession> = db.select(sid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    // Delete all messages for this session first.
    db.query("DELETE chat_message WHERE session_id = $sid")
        .bind(("sid", sid.clone()))
        .await?;
    let _: Option<ChatSession> = db.delete(sid).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/chat/sessions/{id}/messages",
    params(("id" = String, Path, description = "Session ULID")),
    responses(
        (status = 200, body = Vec<ChatMessage>),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "chat"
)]
pub async fn list_session_messages(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<ChatMessage>>, AppError> {
    let db = db.lock().await;
    let sid = RecordId::new("chat_session", session_id.as_str());
    let existing: Option<ChatSession> = db.select(sid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    let messages: Vec<ChatMessage> = db
        .query("SELECT * FROM chat_message WHERE session_id = $sid ORDER BY created_at ASC")
        .bind(("sid", sid))
        .await?
        .take(0)?;
    Ok(Json(messages))
}

#[utoipa::path(
    post,
    path = "/api/chat/sessions/{id}/messages",
    params(("id" = String, Path, description = "Session ULID")),
    request_body = AppendMessage,
    responses(
        (status = 201, body = ChatMessage),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "chat"
)]
pub async fn append_message(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(session_id): Path<String>,
    Json(payload): Json<AppendMessage>,
) -> Result<(StatusCode, Json<ChatMessage>), AppError> {
    let db = db.lock().await;
    let sid = RecordId::new("chat_session", session_id.as_str());
    let existing: Option<ChatSession> = db.select(sid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    // Append the message and bump the session's updated_at in one go.
    let messages: Vec<ChatMessage> = db
        .query("CREATE chat_message SET session_id = $sid, role = $role, content = $content")
        .bind(("sid", sid.clone()))
        .bind(("role", payload.role))
        .bind(("content", payload.content))
        .await?
        .take(0)?;
    db.query("UPDATE $sid SET updated_at = <string>time::now()")
        .bind(("sid", sid))
        .await?;
    messages
        .into_iter()
        .next()
        .map(|m| (StatusCode::CREATED, Json(m)))
        .ok_or_else(|| AppError::BadRequest("Failed to append message".to_string()))
}
