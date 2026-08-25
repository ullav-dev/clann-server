//! Thin client for tack-server's Notes API — the target of the Clann notes
//! migration (see the plan at `/Users/colin/.claude/plans/linked-roaming-
//! rabbit.md`). Not wired into any live handler yet (Phase 0: this module
//! exists and can be exercised by the backfill tooling and integration
//! tests, but `research_note.rs`'s handlers still read/write SurrealDB
//! directly until the Phase 3 frontend cutover).
//!
//! Auth is forwarded, not re-minted: every call takes the caller's own raw
//! `Authorization` header value and passes it straight through, exactly like
//! `ullav-collection-server`'s own `tack_client.rs` (the direct template for
//! this file) and lagan-server's `forwarded_auth_header` pattern before it.
//! This only works because clann-server already authenticates inbound
//! requests against the same UUM JWKS tack-server validates against — there
//! is no separate service credential for ordinary user-facing calls.
//!
//! The one exception is the offline backfill process, which uses a real
//! admin bearer token (`TACK_BACKFILL_TOKEN`) to override `created_by`/
//! `created_at` and preserve original authorship — see the migration plan's
//! "Backfill design" section. That admin token is never used by any
//! request handler in this file's callers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    Team,
    Organization,
}

impl Visibility {
    /// Clann's legacy `is_shared` boolean collapses to `Team` when set —
    /// same mapping Cartlann used for its own legacy-shape translation.
    /// `Organization` is never produced by this mapping; Clann has no
    /// equivalent tier (see the migration plan's data model mapping).
    pub fn from_is_shared(is_shared: bool) -> Self {
        if is_shared { Visibility::Team } else { Visibility::Private }
    }

    pub fn is_shared(&self) -> bool {
        !matches!(self, Visibility::Private)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TackNote {
    pub id: Uuid,
    pub team_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub visibility: Visibility,
    pub title: String,
    pub body_markdown: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reply_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TackNoteFolder {
    pub id: Uuid,
    pub team_id: Uuid,
    pub name: String,
    pub note_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct TackNoteFoldersPage {
    pub folders: Vec<TackNoteFolder>,
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct TackNotesPage {
    pub notes: Vec<TackNote>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TackNoteAttachment {
    pub id: Uuid,
    pub note_id: Uuid,
    pub owning_service: String,
    pub entity_type: String,
    pub entity_id: String,
}

#[derive(Debug, Serialize)]
struct AttachBody<'a> {
    owning_service: &'a str,
    entity_type: &'a str,
    entity_id: &'a str,
}

#[derive(Clone)]
pub struct TackClient {
    http: reqwest::Client,
    base_url: String,
}

impl TackClient {
    pub fn new(http: reqwest::Client, base_url: String) -> Self {
        Self { http, base_url }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn error_for_status(resp: reqwest::Response) -> AppError {
        let status = resp.status();
        let body = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "tack-server request failed".into());
        AppError::TackUpstream(status.as_u16(), body)
    }

    /// `created_by`/`created_at` overrides are admin-only on tack-server's
    /// own side (used exclusively by the backfill process to preserve
    /// original authorship/timestamps — see this module's own doc comment).
    /// Live handler calls always leave both `None`.
    #[allow(clippy::too_many_arguments)]
    /// `team_id: None` creates a genuinely personal, team-less note --
    /// only valid with `visibility: Private` (tack-server 400s otherwise).
    /// See `handlers::research_note`'s own module doc comment for the
    /// live-caller resolution rule.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_note(
        &self,
        auth: &str,
        team_id: Option<Uuid>,
        visibility: Visibility,
        title: &str,
        body_markdown: &str,
        folder_id: Option<Uuid>,
        created_by: Option<Uuid>,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<TackNote, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            team_id: Option<Uuid>,
            visibility: Visibility,
            title: &'a str,
            body_markdown: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            folder_id: Option<Uuid>,
            #[serde(skip_serializing_if = "Option::is_none")]
            created_by: Option<Uuid>,
            #[serde(skip_serializing_if = "Option::is_none")]
            created_at: Option<DateTime<Utc>>,
        }
        let resp = self
            .http
            .post(self.url("/notes"))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&Body { team_id, visibility, title, body_markdown, folder_id, created_by, created_at })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn get_note(&self, auth: &str, id: Uuid) -> Result<Option<TackNote>, AppError> {
        let resp = self
            .http
            .get(self.url(&format!("/notes/{id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        Ok(Some(resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))?))
    }

    pub async fn update_note(
        &self,
        auth: &str,
        id: Uuid,
        title: Option<&str>,
        body_markdown: Option<&str>,
        visibility: Option<Visibility>,
        // Tri-state, mirroring tack-server's own `UpdateNoteRequest.folder_id`:
        // `None` (outer) leaves the folder unchanged (field omitted from the
        // JSON body entirely); `Some(None)` unfiles the note (`"folder_id":
        // null`); `Some(Some(id))` files/moves it into that folder.
        folder_id: Option<Option<Uuid>>,
    ) -> Result<TackNote, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            title: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            body_markdown: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            visibility: Option<Visibility>,
            #[serde(skip_serializing_if = "Option::is_none")]
            folder_id: Option<Option<Uuid>>,
        }
        let resp = self
            .http
            .patch(self.url(&format!("/notes/{id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&Body { title, body_markdown, visibility, folder_id })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn delete_note(&self, auth: &str, id: Uuid) -> Result<(), AppError> {
        let resp = self
            .http
            .delete(self.url(&format!("/notes/{id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        Ok(())
    }

    pub async fn list_replies(&self, auth: &str, parent_id: Uuid) -> Result<Vec<TackNote>, AppError> {
        let resp = self
            .http
            .get(self.url(&format!("/notes/{parent_id}/replies")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    /// `created_by`/`created_at` overrides, same admin-only/backfill-only
    /// caveat as `create_note`.
    pub async fn create_reply(
        &self,
        auth: &str,
        parent_id: Uuid,
        body_markdown: &str,
        created_by: Option<Uuid>,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<TackNote, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            body_markdown: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            created_by: Option<Uuid>,
            #[serde(skip_serializing_if = "Option::is_none")]
            created_at: Option<DateTime<Utc>>,
        }
        let resp = self
            .http
            .post(self.url(&format!("/notes/{parent_id}/replies")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&Body { body_markdown, created_by, created_at })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    /// Links a note to a Clann entity (a tree, per the migration plan) via
    /// tack's generic `content_attachments` table — `owning_service="clann"`,
    /// `entity_type="tree"`, `entity_id=<family_tree id, resolved from the
    /// note's own tree-name-slug at write time, never the raw slug itself>`.
    pub async fn attach(
        &self,
        auth: &str,
        note_id: Uuid,
        owning_service: &str,
        entity_type: &str,
        entity_id: &str,
    ) -> Result<TackNoteAttachment, AppError> {
        let resp = self
            .http
            .post(self.url(&format!("/notes/{note_id}/attachments")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&AttachBody { owning_service, entity_type, entity_id })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn list_attachments(&self, auth: &str, note_id: Uuid) -> Result<Vec<TackNoteAttachment>, AppError> {
        let resp = self
            .http
            .get(self.url(&format!("/notes/{note_id}/attachments")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn detach(&self, auth: &str, note_id: Uuid, attachment_id: Uuid) -> Result<(), AppError> {
        let resp = self
            .http
            .delete(self.url(&format!("/notes/{note_id}/attachments/{attachment_id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        Ok(())
    }

    pub async fn list_team_notes(&self, auth: &str, team_id: Uuid) -> Result<TackNotesPage, AppError> {
        let resp = self
            .http
            .get(self.url("/notes"))
            .query(&[("team_id", team_id.to_string()), ("limit", "100".into())])
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    /// Top-level notes attached to an external entity, oldest-first — the
    /// live-handler equivalent of `GET /notes/by-entity`, used here to list
    /// a Clann tree's own notes uniformly whether the tree is team-linked
    /// or personal (team-less). Deliberately preferred over `list_team_
    /// notes` for this: a team's own notes aren't necessarily scoped to one
    /// tree (nothing stops two trees sharing a team_id), and a personal
    /// tree has no team to scope by at all, whereas every note this method
    /// returns is filtered by tack-server's own `can_view` and by the exact
    /// tree entity, matching what the Clann UI actually needs to show. Not
    /// paginated -- see tack-server's own doc comment on `GET /notes/
    /// by-entity` (entity-attached note counts are small in practice);
    /// accepted at Clann's current real scale, same as every other
    /// consumer of this endpoint.
    pub async fn list_notes_by_entity(&self, auth: &str, entity_id: &str) -> Result<Vec<TackNote>, AppError> {
        let resp = self
            .http
            .get(self.url("/notes/by-entity"))
            .query(&[("owning_service", "clann"), ("entity_type", "tree"), ("entity_id", entity_id)])
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn list_note_folders(&self, auth: &str, team_id: Uuid) -> Result<TackNoteFoldersPage, AppError> {
        let resp = self
            .http
            .get(self.url("/note-folders"))
            .query(&[("team_id", team_id.to_string()), ("limit", "100".into())])
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    /// NO `created_by`/`created_at` override, unlike `create_note`/
    /// `create_reply` -- verified directly against tack-server's own
    /// `CreateNoteFolderRequest` (`models/note.rs`), which carries only
    /// `team_id`/`name`/`attach`. A folder migrated by the backfill is
    /// therefore always attributed to the backfill's own admin caller and
    /// stamped with the run's real timestamp, not `research_folder`'s
    /// original `created_by`/`created_at` -- an earlier draft of this
    /// method sent both anyway, which tack-server's non-`deny_unknown_
    /// fields` Deserialize would have silently swallowed rather than
    /// erroring on, exactly the kind of invisible-authorship-loss bug the
    /// migration plan's own verification section exists to catch. Accepted
    /// as a low-stakes gap for now: no `@ullav-dev/tack-notes` component
    /// surfaces a folder's own creator/date anywhere in its UI (only name
    /// and note count) -- see the plan's backfill design section before
    /// treating this as settled if that ever changes.
    pub async fn create_note_folder(&self, auth: &str, team_id: Uuid, name: &str) -> Result<TackNoteFolder, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            team_id: Uuid,
            name: &'a str,
        }
        let resp = self
            .http
            .post(self.url("/note-folders"))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&Body { team_id, name })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    pub async fn rename_note_folder(&self, auth: &str, id: Uuid, name: &str) -> Result<TackNoteFolder, AppError> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
        }
        let resp = self
            .http
            .patch(self.url(&format!("/note-folders/{id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&Body { name })
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        resp.json().await.map_err(|e| AppError::TackUnreachable(e.to_string()))
    }

    /// Deletes a folder — its notes are unfiled, not deleted, same
    /// semantics as every other tack-notes consumer.
    pub async fn delete_note_folder(&self, auth: &str, id: Uuid) -> Result<(), AppError> {
        let resp = self
            .http
            .delete(self.url(&format!("/note-folders/{id}")))
            .header(reqwest::header::AUTHORIZATION, auth)
            .send()
            .await
            .map_err(|e| AppError::TackUnreachable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        Ok(())
    }
}
