use axum::{
    extract::{Extension, State},
    http::StatusCode,
    Json,
};

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::user_ai_settings::{UpsertUserAiSettings, UserAiSettings},
};

#[utoipa::path(
    get,
    path = "/api/ai-settings",
    responses(
        (status = 200, body = UserAiSettings),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "ai-settings"
)]
pub async fn get_ai_settings(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
) -> Result<Json<UserAiSettings>, AppError> {
    let db = db.lock().await;
    let records: Vec<UserAiSettings> = db
        .query("SELECT * FROM user_ai_settings WHERE username = $username LIMIT 1")
        .bind(("username", auth.user_id))
        .await?
        .take(0)?;
    records.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    put,
    path = "/api/ai-settings",
    request_body = UpsertUserAiSettings,
    responses(
        (status = 200, body = UserAiSettings),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "ai-settings"
)]
pub async fn upsert_ai_settings(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<UpsertUserAiSettings>,
) -> Result<Json<UserAiSettings>, AppError> {
    let db = db.lock().await;
    let username = auth.user_id;

    let existing: Vec<UserAiSettings> = db
        .query("SELECT * FROM user_ai_settings WHERE username = $username LIMIT 1")
        .bind(("username", username.clone()))
        .await?
        .take(0)?;

    let result: Vec<UserAiSettings> = if let Some(record) = existing.into_iter().next() {
        db.query(
            "UPDATE $rid SET \
             provider      = $provider, \
             model         = $model, \
             ollama_url    = $ollama_url, \
             encrypted_key = $encrypted_key, \
             iv            = $iv, \
             auth_tag      = $auth_tag",
        )
        .bind(("rid", record.id))
        .bind(("provider", payload.provider))
        .bind(("model", payload.model))
        .bind(("ollama_url", payload.ollama_url))
        .bind(("encrypted_key", payload.encrypted_key))
        .bind(("iv", payload.iv))
        .bind(("auth_tag", payload.auth_tag))
        .await?
        .take(0)?
    } else {
        db.query(
            "CREATE user_ai_settings SET \
             username      = $username, \
             provider      = $provider, \
             model         = $model, \
             ollama_url    = $ollama_url, \
             encrypted_key = $encrypted_key, \
             iv            = $iv, \
             auth_tag      = $auth_tag",
        )
        .bind(("username", username))
        .bind(("provider", payload.provider))
        .bind(("model", payload.model))
        .bind(("ollama_url", payload.ollama_url))
        .bind(("encrypted_key", payload.encrypted_key))
        .bind(("iv", payload.iv))
        .bind(("auth_tag", payload.auth_tag))
        .await?
        .take(0)?
    };

    result
        .into_iter()
        .next()
        .map(Json)
        .ok_or_else(|| AppError::BadRequest("Failed to save AI settings".to_string()))
}

#[utoipa::path(
    delete,
    path = "/api/ai-settings",
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorResponse),
    ),
    tag = "ai-settings"
)]
pub async fn delete_ai_settings(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    let records: Vec<UserAiSettings> = db
        .query("SELECT * FROM user_ai_settings WHERE username = $username LIMIT 1")
        .bind(("username", auth.user_id))
        .await?
        .take(0)?;
    if let Some(record) = records.into_iter().next() {
        let _: Option<UserAiSettings> = db.delete(record.id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
