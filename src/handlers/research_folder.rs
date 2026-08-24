//! `/api/folders/*` -- `research_folder` stays the source of truth in
//! SurrealDB (see `handlers::research_note`'s own module doc comment for
//! why: it's genuinely personal-per-user, no team/tree column at all, so it
//! has no 1:1 tack equivalent). What changes here for Phase 3: `rename`/
//! `delete` also touch any tack folder this Clann folder has been mapped to
//! (`tack_migration_state`, kind = "folder" -- see
//! `handlers::research_note::resolve_or_create_tack_folder`), so a rename
//! or delete doesn't leave a stale, orphaned tack folder behind.

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::{RecordId, SurrealValue};

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::research_folder::{CreateResearchFolder, RenameResearchFolder, ResearchFolder},
    tack_client::TackClient,
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
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreateResearchFolder>,
) -> Result<(StatusCode, Json<ResearchFolder>), AppError> {
    let db = db.lock().await;
    let folders: Vec<ResearchFolder> = db
        .query("CREATE research_folder SET name = $name, created_by = $creator")
        .bind(("name", payload.name))
        .bind(("creator", auth.username.clone()))
        .await?
        .take(0)?;
    folders
        .into_iter()
        .next()
        .map(|f| (StatusCode::CREATED, Json(f)))
        .ok_or_else(|| AppError::BadRequest("Failed to create folder".to_string()))
}

#[derive(Debug, SurrealValue)]
struct MigrationStateRow {
    tack_id: String,
}

/// Looks up this Clann folder's mapped tack folder, if any -- same
/// `tack_migration_state` row `handlers::research_note::resolve_or_create_
/// tack_folder` reads/writes (duplicated read-only query, not factored into
/// a shared helper -- small and stable enough that the duplication is
/// cheaper than the cross-module coupling; if the lookup query ever
/// changes, check that function too).
async fn find_mapped_tack_folder(db: &Db, clann_folder_id: &str) -> Result<Option<uuid::Uuid>, AppError> {
    let conn = db.lock().await;
    let rows: Vec<MigrationStateRow> = conn
        .query("SELECT tack_id FROM tack_migration_state WHERE surreal_id = $sid AND kind = 'folder'")
        .bind(("sid", clann_folder_id.to_string()))
        .await?
        .take(0)?;
    Ok(rows.into_iter().next().and_then(|r| uuid::Uuid::parse_str(&r.tack_id).ok()))
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
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(folder_id): Path<String>,
    Json(payload): Json<RenameResearchFolder>,
) -> Result<Json<ResearchFolder>, AppError> {
    let fid = RecordId::new("research_folder", folder_id.as_str());
    let existing: Option<ResearchFolder> = {
        let conn = db.lock().await;
        conn.select(fid.clone()).await?
    };
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    if let Some(tack_folder_id) = find_mapped_tack_folder(&db, &folder_id).await? {
        // Best-effort: a stale tack folder name is a cosmetic mismatch, not
        // worth failing the whole rename over if tack is unreachable or the
        // caller (e.g. a non-admin who no longer has edit rights on that
        // team's folder for some other reason) can't rename it there.
        let _ = tack.rename_note_folder(&auth.raw_authorization, tack_folder_id, &payload.name).await;
    }

    let updated: Vec<ResearchFolder> = {
        let conn = db.lock().await;
        conn.query("UPDATE $fid SET name = $name")
            .bind(("fid", fid))
            .bind(("name", payload.name))
            .await?
            .take(0)?
    };
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
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(folder_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let fid = RecordId::new("research_folder", folder_id.as_str());
    let existing: Option<ResearchFolder> = {
        let conn = db.lock().await;
        conn.select(fid.clone()).await?
    };
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    if let Some(tack_folder_id) = find_mapped_tack_folder(&db, &folder_id).await? {
        // tack-server's own delete_note_folder unfiles every note in it
        // server-side (see that endpoint's own doc comment) -- no need to
        // separately PATCH each note to unfile it here. Best-effort: don't
        // block the Clann-side delete on tack being reachable.
        let _ = tack.delete_note_folder(&auth.raw_authorization, tack_folder_id).await;
    }

    let conn = db.lock().await;
    // Clann-side bookkeeping tack has no knowledge of: clear every note's
    // pointer back to this folder id, and drop the now-stale mapping row.
    conn.query("UPDATE tack_note_meta SET legacy_folder_id = NONE WHERE legacy_folder_id = $fid")
        .bind(("fid", folder_id.clone()))
        .await?;
    conn.query("DELETE tack_migration_state WHERE surreal_id = $sid AND kind = 'folder'")
        .bind(("sid", folder_id))
        .await?;
    let _: Option<ResearchFolder> = conn.delete(fid).await?;
    Ok(StatusCode::NO_CONTENT)
}
