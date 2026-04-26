use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::research_folder::{CreateResearchFolder, RenameResearchFolder, ResearchFolder},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct FolderFilter {
    pub created_by: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/folders",
    params(FolderFilter),
    responses(
        (status = 200, body = Vec<ResearchFolder>),
        (status = 401, body = ErrorResponse),
    ),
    tag = "folders"
)]
pub async fn list_folders(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Query(filter): Query<FolderFilter>,
) -> Result<Json<Vec<ResearchFolder>>, AppError> {
    let db = db.lock().await;
    let folders: Vec<ResearchFolder> = if let Some(creator) = filter.created_by {
        db.query("SELECT * FROM research_folder WHERE created_by = $creator ORDER BY name ASC")
            .bind(("creator", creator))
            .await?
            .take(0)?
    } else {
        db.query("SELECT * FROM research_folder ORDER BY name ASC")
            .await?
            .take(0)?
    };
    Ok(Json(folders))
}

#[utoipa::path(
    post,
    path = "/api/folders",
    request_body = CreateResearchFolder,
    responses(
        (status = 201, body = ResearchFolder),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "folders"
)]
pub async fn create_folder(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Json(payload): Json<CreateResearchFolder>,
) -> Result<(StatusCode, Json<ResearchFolder>), AppError> {
    let db = db.lock().await;
    let folders: Vec<ResearchFolder> = db
        .query("CREATE research_folder SET name = $name, created_by = $creator")
        .bind(("name", payload.name))
        .bind(("creator", payload.created_by))
        .await?
        .take(0)?;
    folders
        .into_iter()
        .next()
        .map(|f| (StatusCode::CREATED, Json(f)))
        .ok_or_else(|| AppError::BadRequest("Failed to create folder".to_string()))
}

#[utoipa::path(
    patch,
    path = "/api/folders/{id}",
    params(("id" = String, Path, description = "Folder ULID")),
    request_body = RenameResearchFolder,
    responses(
        (status = 200, body = ResearchFolder),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "folders"
)]
pub async fn rename_folder(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(folder_id): Path<String>,
    Json(payload): Json<RenameResearchFolder>,
) -> Result<Json<ResearchFolder>, AppError> {
    use surrealdb::types::RecordId;
    let db = db.lock().await;
    let fid = RecordId::new("research_folder", folder_id.as_str());
    let existing: Option<ResearchFolder> = db.select(fid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    let updated: Vec<ResearchFolder> = db
        .query("UPDATE $fid SET name = $name")
        .bind(("fid", fid))
        .bind(("name", payload.name))
        .await?
        .take(0)?;
    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    delete,
    path = "/api/folders/{id}",
    params(("id" = String, Path, description = "Folder ULID")),
    responses(
        (status = 204, description = "Deleted; notes in this folder become unfiled"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "folders"
)]
pub async fn delete_folder(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(folder_id): Path<String>,
) -> Result<StatusCode, AppError> {
    use surrealdb::types::RecordId;
    let db = db.lock().await;
    let fid = RecordId::new("research_folder", folder_id.as_str());
    let existing: Option<ResearchFolder> = db.select(fid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    // Unfile all notes that belong to this folder before deleting it.
    let fid_str = format!("research_folder:{folder_id}");
    db.query("UPDATE research_note SET folder_id = NONE WHERE folder_id = $fid")
        .bind(("fid", fid_str))
        .await?;
    let _: Option<ResearchFolder> = db.delete(fid).await?;
    Ok(StatusCode::NO_CONTENT)
}
