//! Wire types for `/api/notes/*` -- backed by tack-server via `tack_client.rs`
//! (Phase 3 of the migration plan, `/Users/colin/.claude/plans/linked-
//! roaming-rabbit.md`), not SurrealDB `research_note` rows directly anymore.
//!
//! Design B (settled with the advisor before writing this): return
//! tack-shaped JSON, not the old `ResearchNote` SurrealDB record shape.
//! clann-webapp's own adapter is being rewritten in this same change, so
//! preserving the old wire shape would only have bought three unnecessary
//! reverse-resolution layers (UUID->username, tree UUID->slug, tack folder
//! UUID->legacy folder id) for a consumer that's also changing.
//!
//! `trees` is dropped from every response here (present on the old
//! `ResearchNote`) -- the Clann UI always lists a tree's own notes already
//! scoped to that one tree (see `handlers::research_note::list_research_
//! notes`), so a note's own tree membership is redundant on every row; the
//! new `TackNoteThread`-based detail UI doesn't render it either. Still
//! *written* at creation time (as a `content_attachments` row on the
//! tack-server note) so `GET /notes/by-entity` can find it -- just not
//! echoed back.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::tack_client::{TackNote, Visibility};

/// A note as tack-server itself models it, plus `description` (no column in
/// tack's own `Note` schema -- carried in clann-server's own `tack_note_meta`
/// sidecar, see that table's own doc comment in `migrations/schema.surql`)
/// and `folder_id` (the *Clann* folder id -- `research_folder`'s own
/// registry id, NOT tack's internal per-team folder UUID; see
/// `handlers::research_note`'s own module doc comment for why these are
/// different ids and how one resolves to the other).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ClannNote {
    pub id: Uuid,
    pub team_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub folder_id: Option<String>,
    pub visibility: Visibility,
    pub title: String,
    pub body_markdown: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    /// RFC3339 -- this crate's own utoipa setup has no chrono feature
    /// enabled (see every other model in this codebase: timestamps are
    /// `String`, never `chrono::DateTime` directly), unlike tack-server's
    /// own `Note` this is converted from.
    pub created_at: String,
    pub updated_at: String,
    pub reply_count: i64,
}

impl ClannNote {
    /// Merges a raw tack `Note` with its sidecar row (`None` for a note with
    /// no description and no folder assignment -- most replies, and any
    /// top-level note nobody's filed or described yet).
    pub fn from_tack(note: TackNote, description: Option<String>, folder_id: Option<String>) -> Self {
        Self {
            id: note.id,
            team_id: note.team_id,
            parent_id: note.parent_id,
            folder_id,
            visibility: note.visibility,
            title: note.title,
            body_markdown: note.body_markdown,
            description,
            created_by: note.created_by,
            created_at: note.created_at.to_rfc3339(),
            updated_at: note.updated_at.to_rfc3339(),
            reply_count: note.reply_count,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateResearchNote {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub body: String,
    /// The tree this note belongs to (by name -- the same slug the rest of
    /// this API already keys trees on). Exactly one, not a list: the UI has
    /// never offered multi-tree notes or post-creation tree reassignment
    /// (confirmed directly against `ResearchPage.tsx` before this rewrite),
    /// so this only ever needs to express what's actually possible to
    /// create, not the old model's theoretical `Vec<String>`.
    pub tree: String,
    /// A *Clann* folder id (`research_folder`'s own registry), not a tack
    /// folder UUID. Only valid together with a team-linked tree -- see
    /// `handlers::research_note::create_research_note`.
    #[serde(default)]
    pub folder_id: Option<String>,
    pub visibility: Visibility,
    // Deliberately NO `created_by` field, unlike the old `CreateResearchNote`
    // -- that field let a caller attribute a note to *any* username with
    // zero server-side validation (the old handler trusted it outright).
    // Authorship is now always the authenticated caller
    // (`ClannAuth::user_id`), same as every other create endpoint in this
    // codebase. A real hardening bundled into this rewire, not an oversight.
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateResearchNote {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    // No `trees` field -- see this module's own doc comment; the UI never
    // offered this and tack has no move-note-between-organizations
    // operation to support it if it did.
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetNoteFolderPayload {
    /// A Clann folder id, or `null` to unfile. Rejected outright for a
    /// personal (team-less) note -- tack has no folder concept for those.
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateNoteReply {
    pub body: String,
    // No `created_by`/`trees` -- same hardening as CreateResearchNote's own
    // doc comment; a reply's trees were always inherited from its parent
    // and unused for anything (never read back), so dropped entirely.
}
