//! `/api/notes/*` -- Phase 3 of the notes migration
//! (`/Users/colin/.claude/plans/linked-roaming-rabbit.md`): backed by
//! tack-server via `tack_client.rs`, not SurrealDB `research_note` rows
//! directly. Mirrors `tack-server/src/bin/backfill-clann-notes.rs`'s own
//! `decide()` logic *deliberately* -- same repos-apart drift risk that
//! function's own doc comment names; if the resolution rule here ever
//! changes, check that file too, and vice versa. The two differ in exactly
//! one way: a live caller's identity/team-membership is enforced by
//! tack-server itself (this module forwards the caller's own JWT, never an
//! admin token -- see `tack_client.rs`'s own doc comment), so there's no
//! bulk team/org resolution to do here the way the offline backfill needs.
//!
//! Every note carries exactly one tree (`CreateResearchNote::tree`, not a
//! list) -- confirmed directly against `ResearchPage.tsx` before this
//! rewrite that the UI never offers multi-tree notes or post-creation tree
//! reassignment. A tree with `family_tree.team_id` set resolves to a normal
//! team-scoped tack note; a team-less (personal) tree resolves to a
//! genuinely personal tack note (`team_id` omitted, `visibility: private`
//! forced) -- tack-server's own `POST /notes` rejects anything else for a
//! team-less note, see that endpoint's own doc comment.
//!
//! **Folders**: `research_folder` stays in SurrealDB as the name registry
//! (id/name/created_by) -- it's genuinely personal-per-user, not
//! team-or-tree-scoped (`created_by`+`name` UNIQUE, no team/tree column at
//! all), so it has no 1:1 tack equivalent (tack's own `note_folders` are
//! always `team_id`-scoped). A Clann folder resolves to a real tack folder
//! only when a *team* note is actually filed into it, on demand
//! (`resolve_or_create_tack_folder`), reusing `tack_migration_state` (kind
//! = "folder") as the durable `clann_folder_id -> tack_folder_id` map --
//! that table already existed for exactly this key shape (originally meant
//! for the backfill's own bookkeeping, which in practice uses its own
//! `--state-file` instead and never wrote here, so this handler is this
//! table's first real writer). A personal note can never be filed --
//! tack's own `POST/PATCH /notes` reject it outright, enforced there, not
//! re-implemented here.
//!
//! **Known, deliberate regression** (flagged in this PR, not discovered
//! later): folders stop applying to personal notes. Checked directly
//! against the last production dry-run: 5 of 6 real migrated notes are
//! personal. The Clann UI's folder sidebar and per-note folder picker
//! become dead controls for almost every note a real user has, until/unless
//! a real per-user (not per-team) filing concept is designed for tack.
//!
//! Reply policy: tack's own `create_reply` only requires `can_view` on the
//! parent (creator or admin, or any team/org member for a shared note) --
//! the old "cannot reply to a private note" rule is deliberately NOT carried
//! forward. Kept as tack's own default rather than reintroduced as bespoke
//! Clann logic: post-cutover almost every note is a private personal note,
//! and the creator being able to add a follow-up reply to their own note is
//! a reasonable feature, not a hole -- reintroducing the old rule would
//! silently make personal notes un-repliable instead.
//!
//! **Checked, not assumed**: `tree_editor` grants (per-tree collaborator
//! access, independent of tack's team-based ACL) are empty in production as
//! of this rewrite (`SELECT tree_name, user_id FROM tree_editor` returned
//! zero rows) -- using tack's built-in permissions only doesn't silently
//! revoke any real grant today. If that ever changes before this migration
//! completes, it needs revisiting.
//!
//! `created_by` is never accepted from the client on create (unlike the old
//! SurrealDB-backed handler, which trusted whatever username string the
//! request body sent, with zero server-side validation) -- always the
//! authenticated caller (`ClannAuth::user_id`), same as every other
//! create endpoint in this codebase. A real hardening bundled into this
//! rewire, not a silent behavior change nobody would notice.

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::collections::HashMap;
use surrealdb::types::SurrealValue;
use uuid::Uuid;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::research_note::{ClannNote, CreateNoteReply, CreateResearchNote, SetNoteFolderPayload, UpdateResearchNote},
    tack_client::{TackClient, Visibility},
};

// ── Tree resolution ──────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct TreeLookup {
    id: String,
    team_id: Option<String>,
}

/// Resolves a tree by its name slug to its own stable id (a raw SurrealDB
/// record key, e.g. `"dw1w409sojpzndrpvinz"` -- NOT reformatted as a UUID;
/// matches the exact convention `backfill-clann-notes.rs` already
/// established for `content_attachments.entity_id`) and its team, if any.
async fn resolve_tree(db: &Db, name: &str) -> Result<TreeLookup, AppError> {
    let db = db.lock().await;
    let rows: Vec<TreeLookup> = db
        .query("SELECT meta::id(id) AS id, team_id FROM family_tree WHERE name = $name")
        .bind(("name", name.to_string()))
        .await?
        .take(0)?;
    rows.into_iter().next().ok_or_else(|| AppError::BadRequest(format!("Unknown tree: {name}")))
}

/// team_id for the tack call, given a resolved tree and the requested
/// visibility -- mirrors `backfill-clann-notes.rs`'s `decide()` for the
/// live, single-tree, non-ambiguous case (see this module's own doc
/// comment on why the two aren't unified into one shared function: no
/// shared crate between clann-server and tack-server's own `src/bin/`).
fn resolve_team_for_tree(tree: &TreeLookup, visibility: Visibility) -> Result<Option<Uuid>, AppError> {
    match &tree.team_id {
        Some(team_id) => Uuid::parse_str(team_id)
            .map(Some)
            .map_err(|_| AppError::Internal(format!("family_tree.team_id is not a valid UUID: {team_id}"))),
        None if visibility == Visibility::Private => Ok(None),
        None => Err(AppError::BadRequest(
            "This tree has no team -- a note on it can only be private (tack has no per-person sharing yet).".into(),
        )),
    }
}

// ── tack_note_meta sidecar (description + Clann folder id) ─────────────────

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct NoteMetaRow {
    tack_note_id: String,
    description: Option<String>,
    legacy_folder_id: Option<String>,
}

/// Batched, not per-note -- the standing "scale-safe by default" rule this
/// codebase already applies everywhere else (see e.g. `NoteTree`'s own
/// pagination discipline on the tack side).
async fn batch_read_note_meta(db: &Db, note_ids: &[Uuid]) -> Result<HashMap<Uuid, NoteMetaRow>, AppError> {
    if note_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let db = db.lock().await;
    let ids: Vec<String> = note_ids.iter().map(|id| id.to_string()).collect();
    let rows: Vec<NoteMetaRow> = db
        .query("SELECT tack_note_id, description, legacy_folder_id FROM tack_note_meta WHERE tack_note_id IN $ids")
        .bind(("ids", ids))
        .await?
        .take(0)?;
    Ok(rows
        .into_iter()
        .filter_map(|r| Uuid::parse_str(&r.tack_note_id).ok().map(|id| (id, r)))
        .collect())
}

async fn read_note_meta(db: &Db, note_id: Uuid) -> Result<Option<NoteMetaRow>, AppError> {
    Ok(batch_read_note_meta(db, &[note_id]).await?.remove(&note_id))
}

/// Creates or updates the sidecar row for one note. `None` for either field
/// means "leave unchanged" if a row already exists, or "not set" on first
/// write -- always writes both fields together (whichever wasn't just
/// changed is re-read from any existing row first), since there's no
/// partial-update tri-state need here the way `UpdateNoteRequest.folder_id`
/// has (this is clann-server's own sidecar, not tack's wire contract).
async fn upsert_note_meta(
    db: &Db,
    note_id: Uuid,
    description: Option<Option<String>>,
    folder_id: Option<Option<String>>,
) -> Result<(), AppError> {
    if description.is_none() && folder_id.is_none() {
        return Ok(());
    }
    let existing = read_note_meta(db, note_id).await?;
    let final_description = description.unwrap_or_else(|| existing.as_ref().and_then(|r| r.description.clone()));
    let final_folder_id = folder_id.unwrap_or_else(|| existing.as_ref().and_then(|r| r.legacy_folder_id.clone()));

    let db = db.lock().await;
    let updated: Vec<serde_json::Value> = db
        .query("UPDATE tack_note_meta SET description = $desc, legacy_folder_id = $fid WHERE tack_note_id = $tid RETURN AFTER")
        .bind(("tid", note_id.to_string()))
        .bind(("desc", final_description.clone()))
        .bind(("fid", final_folder_id.clone()))
        .await?
        .take(0)?;
    if updated.is_empty() {
        db.query("CREATE tack_note_meta SET tack_note_id = $tid, description = $desc, legacy_folder_id = $fid")
            .bind(("tid", note_id.to_string()))
            .bind(("desc", final_description))
            .bind(("fid", final_folder_id))
            .await?;
    }
    Ok(())
}

// ── Folder resolution (Clann folder id -> tack folder UUID, on demand) ─────

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct FolderRow {
    name: String,
}

#[derive(Debug, serde::Deserialize, SurrealValue)]
struct MigrationStateRow {
    tack_id: String,
}

/// Finds or creates the tack folder for `(clann_folder_id, team_id)`,
/// reusing `tack_migration_state` (kind = "folder", `surreal_id` = the
/// Clann folder id) as the durable map -- see this module's own doc comment.
/// If a mapping already exists for a *different* team than requested, this
/// still returns that folder id -- tack-server's own `check_folder_in_team`
/// then rejects the actual file/create call with a clear error, rather than
/// this function guessing which team "should" win. Confirmed this never
/// happens in real data today (no Clann folder's notes span more than one
/// team), so it's a safety net, not an expected path.
async fn resolve_or_create_tack_folder(
    db: &Db,
    tack: &TackClient,
    auth: &str,
    clann_folder_id: &str,
    team_id: Uuid,
) -> Result<Uuid, AppError> {
    {
        let conn = db.lock().await;
        let existing: Vec<MigrationStateRow> = conn
            .query("SELECT tack_id FROM tack_migration_state WHERE surreal_id = $sid AND kind = 'folder'")
            .bind(("sid", clann_folder_id.to_string()))
            .await?
            .take(0)?;
        if let Some(row) = existing.into_iter().next() {
            if let Ok(id) = Uuid::parse_str(&row.tack_id) {
                return Ok(id);
            }
        }
    }

    let name = {
        let conn = db.lock().await;
        let fid = surrealdb::types::RecordId::new("research_folder", clann_folder_id);
        let folder: Option<FolderRow> = conn.select(fid).await?;
        folder.map(|f| f.name).unwrap_or_else(|| "Unfiled".to_string())
    };

    let created = tack.create_note_folder(auth, team_id, &name).await?;

    let conn = db.lock().await;
    conn.query("CREATE tack_migration_state SET surreal_id = $sid, kind = 'folder', tack_id = $tid")
        .bind(("sid", clann_folder_id.to_string()))
        .bind(("tid", created.id.to_string()))
        .await?;
    Ok(created.id)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/notes",
    request_body = CreateResearchNote,
    responses(
        (status = 201, body = ClannNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn create_research_note(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreateResearchNote>,
) -> Result<(StatusCode, Json<ClannNote>), AppError> {
    if payload.body.trim().is_empty() {
        return Err(AppError::BadRequest("body must not be empty".into()));
    }
    let tree = resolve_tree(&db, &payload.tree).await?;
    let team_id = resolve_team_for_tree(&tree, payload.visibility)?;

    let folder_uuid = match (&payload.folder_id, team_id) {
        (Some(_), None) => {
            return Err(AppError::BadRequest("A personal note (no team on this tree) can't be filed into a folder.".into()));
        }
        (Some(clann_folder_id), Some(team_uuid)) => {
            Some(resolve_or_create_tack_folder(&db, &tack, &auth.raw_authorization, clann_folder_id, team_uuid).await?)
        }
        (None, _) => None,
    };

    let note = tack
        .create_note(&auth.raw_authorization, team_id, payload.visibility, &payload.title, &payload.body, folder_uuid, None, None)
        .await?;

    let _ = tack.attach(&auth.raw_authorization, note.id, "clann", "tree", &tree.id).await;

    if payload.description.is_some() || payload.folder_id.is_some() {
        upsert_note_meta(&db, note.id, Some(payload.description.clone()), Some(payload.folder_id.clone())).await?;
    }

    Ok((StatusCode::CREATED, Json(ClannNote::from_tack(note, payload.description, payload.folder_id))))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct NoteFilter {
    pub tree: String,
}

#[utoipa::path(
    get,
    path = "/api/notes",
    params(NoteFilter),
    responses(
        (status = 200, body = Vec<ClannNote>),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn list_research_notes(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Query(filter): Query<NoteFilter>,
) -> Result<Json<Vec<ClannNote>>, AppError> {
    let tree = resolve_tree(&db, &filter.tree).await?;
    let mut notes = tack.list_notes_by_entity(&auth.raw_authorization, &tree.id).await?;
    // GET /notes/by-entity returns oldest-first; the Clann UI has always
    // shown newest-first (the old handler's own `ORDER BY updated_at DESC`)
    // -- restored here rather than silently changing list order underfoot.
    notes.reverse();

    let ids: Vec<Uuid> = notes.iter().map(|n| n.id).collect();
    let meta = batch_read_note_meta(&db, &ids).await?;

    Ok(Json(
        notes
            .into_iter()
            .map(|n| {
                let m = meta.get(&n.id);
                ClannNote::from_tack(n, m.and_then(|m| m.description.clone()), m.and_then(|m| m.legacy_folder_id.clone()))
            })
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "tack note UUID")),
    responses(
        (status = 200, body = ClannNote),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn get_research_note(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
) -> Result<Json<ClannNote>, AppError> {
    let note = tack.get_note(&auth.raw_authorization, note_id).await?.ok_or(AppError::NotFound)?;
    let meta = read_note_meta(&db, note_id).await?;
    Ok(Json(ClannNote::from_tack(note, meta.as_ref().and_then(|m| m.description.clone()), meta.and_then(|m| m.legacy_folder_id))))
}

#[utoipa::path(
    put,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "tack note UUID")),
    request_body = UpdateResearchNote,
    responses(
        (status = 200, body = ClannNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn update_research_note(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
    Json(payload): Json<UpdateResearchNote>,
) -> Result<Json<ClannNote>, AppError> {
    if let Some(ref body) = payload.body {
        if body.trim().is_empty() {
            return Err(AppError::BadRequest("body must not be empty".into()));
        }
    }
    let updated = tack
        .update_note(&auth.raw_authorization, note_id, payload.title.as_deref(), payload.body.as_deref(), payload.visibility, None)
        .await?;

    if payload.description.is_some() {
        upsert_note_meta(&db, note_id, Some(payload.description.clone()), None).await?;
    }
    let meta = read_note_meta(&db, note_id).await?;
    Ok(Json(ClannNote::from_tack(updated, meta.as_ref().and_then(|m| m.description.clone()), meta.and_then(|m| m.legacy_folder_id))))
}

#[utoipa::path(
    delete,
    path = "/api/notes/{note_id}",
    params(("note_id" = String, Path, description = "tack note UUID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn delete_research_note(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    tack.delete_note(&auth.raw_authorization, note_id).await?;
    let conn = db.lock().await;
    let _ = conn.query("DELETE tack_note_meta WHERE tack_note_id = $tid").bind(("tid", note_id.to_string())).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/notes/{note_id}/folder",
    params(("note_id" = String, Path, description = "tack note UUID")),
    request_body = SetNoteFolderPayload,
    responses(
        (status = 200, body = ClannNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn set_note_folder(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
    Json(payload): Json<SetNoteFolderPayload>,
) -> Result<Json<ClannNote>, AppError> {
    let note = tack.get_note(&auth.raw_authorization, note_id).await?.ok_or(AppError::NotFound)?;

    let tack_folder_id = match (&payload.folder_id, note.team_id) {
        (Some(_), None) => {
            return Err(AppError::BadRequest("This note has no team, so it can't be filed into a folder.".into()));
        }
        (Some(clann_folder_id), Some(team_uuid)) => {
            Some(resolve_or_create_tack_folder(&db, &tack, &auth.raw_authorization, clann_folder_id, team_uuid).await?)
        }
        (None, _) => None,
    };

    let updated = tack.update_note(&auth.raw_authorization, note_id, None, None, None, Some(tack_folder_id)).await?;
    upsert_note_meta(&db, note_id, None, Some(payload.folder_id.clone())).await?;
    let meta = read_note_meta(&db, note_id).await?;
    Ok(Json(ClannNote::from_tack(updated, meta.as_ref().and_then(|m| m.description.clone()), meta.and_then(|m| m.legacy_folder_id))))
}

#[utoipa::path(
    get,
    path = "/api/notes/{note_id}/replies",
    params(("note_id" = String, Path, description = "tack note UUID")),
    responses(
        (status = 200, body = Vec<ClannNote>),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn list_note_replies(
    State(db): State<Db>,
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
) -> Result<Json<Vec<ClannNote>>, AppError> {
    let replies = tack.list_replies(&auth.raw_authorization, note_id).await?;
    let ids: Vec<Uuid> = replies.iter().map(|r| r.id).collect();
    let meta = batch_read_note_meta(&db, &ids).await?;
    Ok(Json(
        replies
            .into_iter()
            .map(|r| {
                let m = meta.get(&r.id);
                ClannNote::from_tack(r, m.and_then(|m| m.description.clone()), m.and_then(|m| m.legacy_folder_id.clone()))
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/notes/{note_id}/replies",
    params(("note_id" = String, Path, description = "tack note UUID")),
    request_body = CreateNoteReply,
    responses(
        (status = 201, body = ClannNote),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "research-notes"
)]
pub async fn create_note_reply(
    Extension(tack): Extension<TackClient>,
    Extension(auth): Extension<ClannAuth>,
    Path(note_id): Path<Uuid>,
    Json(payload): Json<CreateNoteReply>,
) -> Result<(StatusCode, Json<ClannNote>), AppError> {
    if payload.body.trim().is_empty() {
        return Err(AppError::BadRequest("body must not be empty".into()));
    }
    let reply = tack.create_reply(&auth.raw_authorization, note_id, &payload.body, None, None).await?;
    Ok((StatusCode::CREATED, Json(ClannNote::from_tack(reply, None, None))))
}
