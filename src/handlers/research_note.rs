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
    models::research_note::{
        CreateNoteReply, CreateResearchNote, ResearchNote, SetNoteFolderPayload, UpdateResearchNote,
    },
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
             folder_id   = $folder_id, \
             created_by  = $creator, \
             is_shared   = $is_shared",
        )
        .bind(("title",     payload.title))
        .bind(("desc",      payload.description))
        .bind(("body",      payload.body))
        .bind(("trees",     payload.trees))
        .bind(("folder_id", payload.folder_id))
        .bind(("creator",   payload.created_by))
        .bind(("is_shared", payload.is_shared))
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

    // Always exclude replies (parent_id IS NOT NONE) from the list endpoint.
    // When created_by is omitted (team-tree reads), return notes the requester
    // owns OR that are explicitly shared — the frontend omits created_by for
    // team-linked trees so all members see shared notes.
    let notes: Vec<ResearchNote> = match (filter.tree, filter.created_by) {
        (Some(tree), Some(creator)) => db
            .query(
                "SELECT * FROM research_note \
                 WHERE $tree INSIDE trees \
                 AND (created_by = $creator OR is_shared = true) \
                 AND parent_id IS NONE \
                 ORDER BY updated_at DESC",
            )
            .bind(("tree", tree))
            .bind(("creator", creator))
            .await?
            .take(0)?,
        (Some(tree), None) => db
            .query(
                "SELECT * FROM research_note \
                 WHERE $tree INSIDE trees \
                 AND parent_id IS NONE \
                 ORDER BY updated_at DESC",
            )
            .bind(("tree", tree))
            .await?
            .take(0)?,
        (None, Some(creator)) => db
            .query(
                "SELECT * FROM research_note \
                 WHERE created_by = $creator \
                 AND parent_id IS NONE \
                 ORDER BY updated_at DESC",
            )
            .bind(("creator", creator))
            .await?
            .take(0)?,
        (None, None) => db
            .query(
                "SELECT * FROM research_note \
                 WHERE parent_id IS NONE \
                 ORDER BY updated_at DESC",
            )
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
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());

    let existing: Option<ResearchNote> = db.select(nid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }

    // Build the SET clause dynamically so omitted fields are not overwritten.
    let mut sets = vec![
        "updated_at = <string>time::now()",
    ];
    if payload.title.is_some()     { sets.push("title = $title"); }
    if payload.description.is_some() { sets.push("description = $desc"); }
    if payload.body.is_some()      { sets.push("body = $body"); }
    if payload.trees.is_some()     { sets.push("trees = $trees"); }
    if payload.is_shared.is_some() { sets.push("is_shared = $is_shared"); }

    let query = format!("UPDATE $nid SET {}", sets.join(", "));

    let updated: Vec<ResearchNote> = db
        .query(query)
        .bind(("nid",       nid))
        .bind(("title",     payload.title))
        .bind(("desc",      payload.description))
        .bind(("body",      payload.body))
        .bind(("trees",     payload.trees))
        .bind(("is_shared", payload.is_shared))
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
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());
    let existing: Option<ResearchNote> = db.select(nid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    // Delete all replies first, then the note itself.
    db.query("DELETE research_note WHERE parent_id = $nid")
        .bind(("nid", nid.clone()))
        .await?;
    let _: Option<ResearchNote> = db.delete(nid).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/notes/{note_id}/folder",
    params(("note_id" = String, Path, description = "Research note ULID")),
    request_body = SetNoteFolderPayload,
    responses(
        (status = 200, body = ResearchNote),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn set_note_folder(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
    Json(payload): Json<SetNoteFolderPayload>,
) -> Result<Json<ResearchNote>, AppError> {
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());
    let existing: Option<ResearchNote> = db.select(nid.clone()).await?;
    if existing.is_none() {
        return Err(AppError::NotFound);
    }
    let updated: Vec<ResearchNote> = db
        .query("UPDATE $nid SET folder_id = $folder_id, updated_at = <string>time::now()")
        .bind(("nid",       nid))
        .bind(("folder_id", payload.folder_id))
        .await?
        .take(0)?;
    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    get,
    path = "/api/notes/{note_id}/replies",
    params(("note_id" = String, Path, description = "Research note ULID")),
    responses(
        (status = 200, body = Vec<ResearchNote>),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn list_note_replies(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
) -> Result<Json<Vec<ResearchNote>>, AppError> {
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());

    let parent: Option<ResearchNote> = db.select(nid.clone()).await?;
    if parent.is_none() {
        return Err(AppError::NotFound);
    }

    let replies: Vec<ResearchNote> = db
        .query("SELECT * FROM research_note WHERE parent_id = $nid ORDER BY created_at ASC")
        .bind(("nid", nid))
        .await?
        .take(0)?;

    Ok(Json(replies))
}

#[utoipa::path(
    post,
    path = "/api/notes/{note_id}/replies",
    params(("note_id" = String, Path, description = "Research note ULID")),
    request_body = CreateNoteReply,
    responses(
        (status = 201, body = ResearchNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn create_note_reply(
    State(db): State<Db>,
    Extension(_auth): Extension<ClannAuth>,
    Path(note_id): Path<String>,
    Json(payload): Json<CreateNoteReply>,
) -> Result<(StatusCode, Json<ResearchNote>), AppError> {
    let db = db.lock().await;
    let nid = RecordId::new("research_note", note_id.as_str());

    let parent: Option<ResearchNote> = db.select(nid.clone()).await?;
    match parent {
        None => return Err(AppError::NotFound),
        Some(p) if !p.is_shared => {
            return Err(AppError::BadRequest(
                "Cannot reply to a private note".to_string(),
            ))
        }
        _ => {}
    }

    let replies: Vec<ResearchNote> = db
        .query(
            "CREATE research_note SET \
             title      = $title, \
             body       = $body, \
             trees      = $trees, \
             created_by = $creator, \
             is_shared  = true, \
             parent_id  = $parent_id",
        )
        .bind(("title",     format!("Re: {note_id}")))
        .bind(("body",      payload.body))
        .bind(("trees",     payload.trees))
        .bind(("creator",   payload.created_by))
        .bind(("parent_id", nid))
        .await?
        .take(0)?;

    replies
        .into_iter()
        .next()
        .map(|r| (StatusCode::CREATED, Json(r)))
        .ok_or_else(|| AppError::BadRequest("Failed to create reply".to_string()))
}
