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
    db::Db,
    error::AppError,
    handlers::person::{PersonFilter, check_ownership},
    models::person::Person,
};

const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024; // 2 MB

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
        (status = 404, description = "Person not found", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn upload_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    // Verify person exists and caller has access.
    let person: Option<Person> = db.lock().await.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    // Read the multipart field named "image".
    while let Some(field) = multipart.next_field().await.map_err(|e| AppError::BadRequest(e.to_string()))? {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name != "image" {
            continue;
        }

        let content_type = field
            .content_type()
            .ok_or_else(|| AppError::BadRequest("Missing content-type on image field".to_string()))?
            .to_string();

        let ext = match content_type.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/png" => "png",
            other => {
                return Err(AppError::BadRequest(format!(
                    "Unsupported image type `{other}`. Only JPG and PNG are accepted."
                )));
            }
        };

        // Stream bytes, enforcing the size limit.
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

        // Write file, deleting any previous image for this person first.
        fs::create_dir_all(&upload_dir).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create upload directory: {e}"))
        })?;

        // Remove stale images for this person (any extension).
        for old_ext in ["jpg", "png"] {
            let old_path = format!("{upload_dir}/{id}.{old_ext}");
            let _ = fs::remove_file(&old_path).await; // ignore "not found" errors
        }

        let filename = format!("{id}.{ext}");
        let file_path = format!("{upload_dir}/{filename}");
        fs::write(&file_path, &bytes).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to write image: {e}"))
        })?;

        // Persist filename in DB.
        let _: Option<Person> = db
            .lock().await
            .update(("person", id.as_str()))
            .merge(json!({ "image_path": filename }))
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

    let content_type = if filename.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };

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
        description = "Image file field named `image`. Accepts JPG or PNG, maximum 2 MB."
    ),
    responses(
        (status = 204, description = "Life story image uploaded and associated with the person"),
        (status = 400, description = "Invalid file type, size exceeded, or missing field", body = crate::error::ErrorResponse),
        (status = 404, description = "Person not found", body = crate::error::ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn upload_life_image(
    State(db): State<Db>,
    Extension(upload_dir): Extension<String>,
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

        let ext = match content_type.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/png" => "png",
            other => {
                return Err(AppError::BadRequest(format!(
                    "Unsupported image type `{other}`. Only JPG and PNG are accepted."
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

        fs::create_dir_all(&upload_dir).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to create upload directory: {e}"))
        })?;

        for old_ext in ["jpg", "png"] {
            let old_path = format!("{upload_dir}/{id}_life.{old_ext}");
            let _ = fs::remove_file(&old_path).await;
        }

        let filename = format!("{id}_life.{ext}");
        let file_path = format!("{upload_dir}/{filename}");
        fs::write(&file_path, &bytes).await.map_err(|e| {
            AppError::BadRequest(format!("Failed to write image: {e}"))
        })?;

        let _: Option<Person> = db
            .lock().await
            .update(("person", id.as_str()))
            .merge(json!({ "life_image_path": filename }))
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
        (status = 200, description = "Life story image file (JPEG or PNG)", content_type = "image/jpeg"),
        (status = 404, description = "Person not found or no life story image uploaded", body = crate::error::ErrorResponse),
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

    let content_type = if filename.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };

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
