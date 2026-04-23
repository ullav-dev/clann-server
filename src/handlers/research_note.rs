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
    models::research_note::{CreateResearchNote, ResearchNote, UpdateResearchNote},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct NoteFilter {
    pub tree: Option<String>,
    pub created_by: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/notes",
    request_body = CreateResearchNote,
    responses(
        (status = 201, body = ResearchNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn create_research_note(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Json(payload): Json<CreateResearchNote>,
) -> Result<(StatusCode, Json<ResearchNote>), AppError> {
    let db = db.lock().await;

    let notes: Vec<ResearchNote> = db
        .query(
            "CREATE research_note SET \
             title       = $title, \
             description = $desc, \
             body        = $body, \
             trees       = $trees, \
             created_by  = $creator",
        )
        .bind(("title",   payload.title))
        .bind(("desc",    payload.description))
        .bind(("body",    payload.body))
        .bind(("trees",   payload.trees))
        .bind(("creator", payload.created_by))
        .await?
        .take(0)?;

    notes
        .into_iter()
        .next()
        .map(|n| (StatusCode::CREATED, Json(n)))
        .ok_or_else(|| AppError::BadRequest("Failed to create research note".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/notes",
    params(NoteFilter),
    responses(
        (status = 200, body = Vec<ResearchNote>),
        (status = 401, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn list_research_notes(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Query(filter): Query<NoteFilter>,
) -> Result<Json<Vec<ResearchNote>>, AppError> {
    let db = db.lock().await;

    let notes: Vec<ResearchNote> = match (filter.tree, filter.created_by) {
        (Some(tree), Some(creator)) => db
            .query("SELECT * FROM research_note WHERE $tree INSIDE trees AND created_by = $creator ORDER BY updated_at DESC")
            .bind(("tree", tree))
            .bind(("creator", creator))
            .await?
            .take(0)?,
        (Some(tree), None) => db
            .query("SELECT * FROM research_note WHERE $tree INSIDE trees ORDER BY updated_at DESC")
            .bind(("tree", tree))
            .await?
            .take(0)?,
        (None, Some(creator)) => db
            .query("SELECT * FROM research_note WHERE created_by = $creator ORDER BY updated_at DESC")
            .bind(("creator", creator))
            .await?
            .take(0)?,
        (None, None) => db
            .query("SELECT * FROM research_note ORDER BY updated_at DESC")
            .await?
            .take(0)?,
    };

    Ok(Json(notes))
}

#[utoipa::path(
    get,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "Research note ULID")),
    responses(
        (status = 200, body = ResearchNote),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn get_research_note(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
) -> Result<Json<ResearchNote>, AppError> {
    use surrealdb::types::RecordId;
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());
    let note: Option<ResearchNote> = db.select(nid).await?;
    note.map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    put,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "Research note ULID")),
    request_body = UpdateResearchNote,
    responses(
        (status = 200, body = ResearchNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn update_research_note(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
    Json(payload): Json<UpdateResearchNote>,
) -> Result<Json<ResearchNote>, AppError> {
    use surrealdb::types::RecordId;
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());

    let existing: Option<ResearchNote> = db.select(nid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    let updated: Vec<ResearchNote> = db
        .query(
            "UPDATE $nid SET \
             title       = $title, \
             description = $desc, \
             body        = $body, \
             trees       = $trees, \
             updated_at  = <string>time::now()",
        )
        .bind(("nid",   nid))
        .bind(("title", payload.title))
        .bind(("desc",  payload.description))
        .bind(("body",  payload.body))
        .bind(("trees", payload.trees))
        .await?
        .take(0)?;

    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    delete,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "Research note ULID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn delete_research_note(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
) -> Result<StatusCode, AppError> {
    use surrealdb::types::RecordId;
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());
    let existing: Option<ResearchNote> = db.select(nid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    let _: Option<ResearchNote> = db.delete(nid).await?;
    Ok(StatusCode::NO_CONTENT)
}
