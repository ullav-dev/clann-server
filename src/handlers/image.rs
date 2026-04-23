use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{header, Response, StatusCode},
    response::IntoResponse,
    Extension,
};
use serde_json::json;
use tokio::fs;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::AppError,
    handlers::person::{PersonFilter, check_ownership},
    models::person::Person,
};

const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024; // 2 MB hard cap for profile pictures

/// Maps a MIME type to its canonical file extension.
/// Returns `None` for unsupported types.
fn ext_for_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "video/mp4" => Some("mp4"),
        "video/quicktime" => Some("mov"),
        "video/webm" => Some("webm"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/wav" | "audio/wave" | "audio/x-wav" => Some("wav"),
        "audio/ogg" => Some("ogg"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

/// Maps a file extension to its MIME type for serving.
fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Queries total media bytes used by `user_id` across all their persons.
async fn total_media_bytes(db: &crate::db::DbConn, user_id: &str) -> Result<i64, crate::error::AppError> {
    let result: Option<serde_json::Value> = db
        .query(
            "SELECT math::sum(array::flatten(SELECT [(image_bytes ?? 0), (life_image_bytes ?? 0)] \
             FROM person WHERE created_by = $uid)) AS total"
        )
        .bind(("uid", user_id.to_string()))
        .await?
        .take(0)?;

    Ok(result
        .and_then(|v| v.get("total").and_then(|t| t.as_i64()))
        .unwrap_or(0))
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/image",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body(
        content = String,
        content_type = "multipart/form-data",
        description = "Image file field named `image`. Accepts JPG or PNG, maximum 2 MB."
    ),
    responses(
        (status = 204, description = "Image uploaded and associated with the person"),
        (status = 400, description = "Invalid file type, size exceeded, or missing field", body = crate::error::ErrorResponse),
        (status = 403, description = "Storage quota exceeded", body = crate::error::ErrorResponse),
        (status = 404, description = "Person not found", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn upload_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    let person: Option<Person> = db.lock().await.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name != "image" {
            continue;
        }

        let content_type = field
            .content_type()
            .ok_or_else(|| AppError::BadRequest("Missing content-type on image field".to_string()))?
            .to_string();

        // Profile pictures are always image-only regardless of plan.
        let ext = match content_type.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/png" => "png",
            other => {
                return Err(AppError::BadRequest(format!(
                    "Unsupported image type `{other}`. Only JPG and PNG are accepted for profile pictures."
                )));
            }
        };

        let mut bytes: Vec<u8> = Vec::new();
        let mut stream = field;
        while let Some(chunk) = stream.chunk().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(AppError::BadRequest(format!(
                    "Image exceeds the 2 MB size limit ({} bytes received so far)",
                    bytes.len()
                )));
            }
        }

        if bytes.is_empty() {
            return Err(AppError::BadRequest("Image field is empty".to_string()));
        }

        let new_size = bytes.len() as i64;

        // Check storage quota: total used - old image_bytes + new_size
        if let Some(limit) = auth.storage_limit_bytes() {
            let db_conn = db.lock().await;
            let total = total_media_bytes(&db_conn, &auth.user_id).await?;
            let old_bytes = person.image_bytes.unwrap_or(0);
            if total - old_bytes + new_size > limit {
                let limit_mb = limit / (1024 * 1024);
                return Err(AppError::Forbidden(format!(
                    "Storage limit of {limit_mb} MB reached for your plan. Upgrade for more storage."
                )));
            }
        }

        fs::create_dir_all(&upload_dir).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create upload directory: {e}"))
        })?;

        for old_ext in ["jpg", "png"] {
            let old_path = format!("{upload_dir}/{id}.{old_ext}");
            let _ = fs::remove_file(&old_path).await;
        }

        let filename = format!("{id}.{ext}");
        let file_path = format!("{upload_dir}/{filename}");
        fs::write(&file_path, &bytes).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to write image: {e}"))
        })?;

        let _: Option<Person> = db
            .lock().await
            .update(("person", id.as_str()))
            .merge(json!({ "image_path": filename, "image_bytes": new_size }))
            .await?;

        return Ok(StatusCode::NO_CONTENT);
    }

    Err(AppError::BadRequest("No `image` field found in multipart body".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/image",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Image file (JPEG or PNG)", content_type = "image/jpeg"),
        (status = 404, description = "Person not found or no image uploaded", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn get_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let person: Option<Person> = db.lock().await.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;

    let filename = person.image_path.ok_or(AppError::NotFound)?;
    let ext = filename.rsplit('.').next().unwrap_or("jpg");
    let content_type = mime_for_ext(ext);

    let file_path = format!("{upload_dir}/{filename}");
    let bytes = fs::read(&file_path).await.map_err(|_| AppError::NotFound)?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap();

    Ok(response)
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/life-image",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body(
        content = String,
        content_type = "multipart/form-data",
        description = "Media file field named `image`. Individual/Family: JPG or PNG only. \
                       Professional/Enterprise: also accepts video (MP4, MOV, WebM), \
                       audio (MP3, WAV, OGG), and PDF."
    ),
    responses(
        (status = 204, description = "Life story media uploaded and associated with the person"),
        (status = 400, description = "Invalid file type, size exceeded, or missing field", body = crate::error::ErrorResponse),
        (status = 403, description = "File type not allowed on your plan, or storage quota exceeded", body = crate::error::ErrorResponse),
        (status = 404, description = "Person not found", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn upload_life_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    let person: Option<Person> = db.lock().await.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name != "image" {
            continue;
        }

        let content_type = field
            .content_type()
            .ok_or_else(|| AppError::BadRequest("Missing content-type on image field".to_string()))?
            .to_string();

        // Enforce plan-based media type restriction.
        if !auth.life_media_type_allowed(&content_type) {
            return Err(AppError::Forbidden(format!(
                "Your plan does not allow `{content_type}` uploads. \
                 Upgrade to Professional for video, audio, and document support."
            )));
        }

        let ext = ext_for_mime(&content_type).ok_or_else(|| {
            AppError::BadRequest(format!(
                "Unsupported media type `{content_type}`."
            ))
        })?;

        let mut bytes: Vec<u8> = Vec::new();
        let mut stream = field;
        while let Some(chunk) = stream.chunk().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
            bytes.extend_from_slice(&chunk);
        }

        if bytes.is_empty() {
            return Err(AppError::BadRequest("Image field is empty".to_string()));
        }

        let new_size = bytes.len() as i64;

        // Check storage quota: total used - old life_image_bytes + new_size
        if let Some(limit) = auth.storage_limit_bytes() {
            let db_conn = db.lock().await;
            let total = total_media_bytes(&db_conn, &auth.user_id).await?;
            let old_bytes = person.life_image_bytes.unwrap_or(0);
            if total - old_bytes + new_size > limit {
                let limit_mb = limit / (1024 * 1024);
                return Err(AppError::Forbidden(format!(
                    "Storage limit of {limit_mb} MB reached for your plan. Upgrade for more storage."
                )));
            }
        }

        fs::create_dir_all(&upload_dir).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create upload directory: {e}"))
        })?;

        // Remove any previous life media for this person (all known extensions).
        for old_ext in ["jpg", "jpeg", "png", "gif", "webp", "mp4", "mov", "webm", "mp3", "wav", "ogg", "pdf"] {
            let old_path = format!("{upload_dir}/{id}_life.{old_ext}");
            let _ = fs::remove_file(&old_path).await;
        }

        let filename = format!("{id}_life.{ext}");
        let file_path = format!("{upload_dir}/{filename}");
        fs::write(&file_path, &bytes).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to write life media: {e}"))
        })?;

        let _: Option<Person> = db
            .lock().await
            .update(("person", id.as_str()))
            .merge(json!({ "life_image_path": filename, "life_image_bytes": new_size }))
            .await?;

        return Ok(StatusCode::NO_CONTENT);
    }

    Err(AppError::BadRequest("No `image` field found in multipart body".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/life-image",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Life story media file", content_type = "image/jpeg"),
        (status = 404, description = "Person not found or no life story media uploaded", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn get_life_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let person: Option<Person> = db.lock().await.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;

    let filename = person.life_image_path.ok_or(AppError::NotFound)?;
    let ext = filename.rsplit('.').next().unwrap_or("jpg");
    let content_type = mime_for_ext(ext);

    let file_path = format!("{upload_dir}/{filename}");
    let bytes = fs::read(&file_path).await.map_err(|_| AppError::NotFound)?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap();

    Ok(response)
}
