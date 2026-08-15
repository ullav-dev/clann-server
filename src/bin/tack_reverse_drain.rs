//! Reverse-drain: tack-server -> clann-server's own SurrealDB
//! (/Users/colin/.claude/plans/linked-roaming-rabbit.md, "Rollback
//! strategy"). The other half of `tack_backfill.rs` -- replays whatever
//! happened in tack-server *after* a Phase 3/4 frontend cutover back into
//! `research_note`/`research_folder`, so the old SurrealDB-backed UI can
//! resume from an accurate state if the migration needs to be rolled back.
//!
//! Per the plan, this must be built and **successfully rehearsed in
//! staging before Phase 2 starts** -- rehearsal explicitly includes
//! reverting the deployed code with the sidecar tables already populated
//! and confirming identical pre-migration behavior. This binary is that
//! tool; the rehearsal itself is an operational step, not something this
//! file can do on its own.
//!
//! Scope: `TEAM_IDS` (required, comma-separated) -- the operator's own
//! record of which teams were actually cut over, matching how a real
//! rollback would be invoked (this tool has no way to discover that on its
//! own; `tack_migration_state`/`tack_reverse_drain_state` record *what* was
//! migrated, not *which teams went live*).
//!
//! Reconciliation, per note:
//! - Already in `tack_migration_state` (kind='note') -- originally
//!   migrated forward. Its `research_note` row is UPDATEd (title, body,
//!   is_shared, trees) to reflect any post-cutover edit. `created_by`/
//!   `created_at` are never touched -- tack has no way to edit a note's
//!   author after creation (verified: `UpdateNoteRequest` has no such
//!   field), so the original migrated authorship is already correct and
//!   permanent.
//! - Not in `tack_migration_state` -- created fresh in tack after cutover.
//!   A new `research_note` row is CREATEd, with every field reverse-
//!   mapped, tracked in `tack_reverse_drain_state` for idempotency on
//!   re-run.
//! - In `tack_migration_state` (kind='note') but no longer resolves in
//!   tack (deleted post-cutover) -- flagged in the report as needing
//!   manual review, never auto-deleted. Silently deleting a Clann user's
//!   historical data because a rollback tool guessed a delete was intended
//!   is a worse failure mode than a stale row a human has to clear.
//!
//! `description`: recovered from `tack_note_meta` when a sidecar row
//! exists for that tack note id (true for every note this same
//! clann-server instance originally migrated forward, and for any note
//! created post-cutover through a Phase 3-repointed handler that writes
//! the sidecar symmetrically to `tack_backfill.rs`'s own Step B). Dropped
//! with a logged warning only when no sidecar row exists at all -- the
//! plan's own "does not round-trip cleanly" concern, scoped to exactly the
//! case where it's unavoidable rather than applied blanket.
//!
//! Replies: matched by position, not content -- for a previously-migrated
//! note, the first N tack replies (N = however many `research_note` reply
//! rows already exist for that parent) are assumed to be the original
//! ones; anything past that is new and gets created. This is a known,
//! documented heuristic (robust to new replies appended at the end, not
//! robust to an existing early reply being edited in tack) -- acceptable
//! for a rehearsal-stage tool, not something to treat as exact.
//!
//! `--dry-run`: full reconciliation and reporting, zero writes -- same
//! contract as `tack_backfill.rs`'s own `--dry-run`.

use anyhow::Result;
use clann_server::backfill::{resolve_trees, resolve_usernames_for_uuids, TreeInfo};
use clann_server::tack_client::{TackClient, Visibility as TackVisibility};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use surrealdb::{engine::any, opt::auth::Root};
use uuid::Uuid;

const UNKNOWN_AUTHOR_PLACEHOLDER: &str = "clann-migration-rollback";

fn str_field<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    row.get(field)?.as_str()
}

#[derive(Debug, Serialize, Default)]
struct Report {
    dry_run: bool,
    teams_processed: Vec<String>,
    folders_created: usize,
    notes_created: usize,
    notes_updated: usize,
    replies_created: usize,
    notes_deleted_post_cutover_needs_review: Vec<String>,
    descriptions_not_recoverable: Vec<String>,
    unresolvable_authors: Vec<String>,
    unresolvable_trees: Vec<String>,
}

fn write_report(report: &Report) -> Result<()> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let kind = if report.dry_run { "dryrun" } else { "run" };
    let file = format!("tack_reverse_drain_{kind}_report_{ts}.json");
    std::fs::write(&file, serde_json::to_string_pretty(report)?)?;
    println!("Report written to {file}");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let dry_run = std::env::args().any(|a| a == "--dry-run");

    let db_url = std::env::var("DB_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string());
    let db_ns = std::env::var("DB_NAMESPACE").unwrap_or_else(|_| "clann".to_string());
    let db_db = std::env::var("DB_DATABASE").unwrap_or_else(|_| "ancestry".to_string());
    let db_user = std::env::var("DB_USERNAME").unwrap_or_else(|_| "root".to_string());
    let db_pass = std::env::var("DB_PASSWORD").unwrap_or_else(|_| "secret".to_string());

    let uum_base = std::env::var("UUM_URL").unwrap_or_else(|_| "http://localhost:8081".to_string());
    let uum_token = std::env::var("UUM_ADMIN_TOKEN").unwrap_or_default();
    let tack_base = std::env::var("TACK_URL").unwrap_or_else(|_| "http://localhost:8087".to_string());
    let tack_token = std::env::var("TACK_BACKFILL_TOKEN").unwrap_or_default();
    let team_ids_raw = std::env::var("TEAM_IDS").unwrap_or_default();
    let team_ids: Vec<Uuid> = team_ids_raw.split(',').filter_map(|s| Uuid::parse_str(s.trim()).ok()).collect();

    println!("=== tack-server -> Clann reverse drain {} ===", if dry_run { "(DRY RUN)" } else { "(LIVE)" });
    if team_ids.is_empty() {
        anyhow::bail!("TEAM_IDS is required (comma-separated team UUIDs that were actually cut over)");
    }
    if !dry_run && (uum_token.is_empty() || tack_token.is_empty()) {
        anyhow::bail!("UUM_ADMIN_TOKEN and TACK_BACKFILL_TOKEN are required for a live run");
    }

    println!("Connecting to SurrealDB at {db_url} ...");
    let db = any::connect(&db_url).await?;
    db.signin(Root { username: db_user, password: db_pass }).await?;
    db.use_ns(&db_ns).use_db(&db_db).await?;
    println!("Connected ({db_ns}/{db_db}).\n");

    let http = reqwest::Client::new();
    let tack = TackClient::new(http.clone(), tack_base);
    let tack_auth = format!("Bearer {tack_token}");

    let mut report = Report { dry_run, ..Default::default() };

    // ── Reverse maps ────────────────────────────────────────────────────────
    let tree_rows: Vec<Value> =
        db.query("SELECT name, meta::id(id) AS tree_id, team_id FROM family_tree").await?.take(0)?;
    let trees: HashMap<String, TreeInfo> = resolve_trees(
        tree_rows.iter().filter_map(|r| Some((str_field(r, "name")?, str_field(r, "tree_id")?, str_field(r, "team_id")))),
    );
    let tree_id_to_name: HashMap<&str, &str> = trees.iter().map(|(name, info)| (info.tree_id.as_str(), name.as_str())).collect();

    let forward_note_rows: Vec<Value> =
        db.query("SELECT surreal_id, tack_id FROM tack_migration_state WHERE kind = 'note'").await?.take(0)?;
    // tack note uuid -> surreal research_note id
    let forward_note_map: HashMap<String, String> = forward_note_rows
        .iter()
        .filter_map(|r| Some((str_field(r, "tack_id")?.to_string(), str_field(r, "surreal_id")?.to_string())))
        .collect();

    let forward_folder_rows: Vec<Value> =
        db.query("SELECT surreal_id, tack_id FROM tack_migration_state WHERE kind = 'folder'").await?.take(0)?;
    // tack folder uuid -> legacy research_folder id (strips the "|team" suffix -- see 002's own doc comment)
    let forward_folder_map: HashMap<String, String> = forward_folder_rows
        .iter()
        .filter_map(|r| {
            let surreal_id = str_field(r, "surreal_id")?; // "research_folder:<id>|<team>"
            let tack_id = str_field(r, "tack_id")?.to_string();
            let legacy_id = surreal_id.strip_prefix("research_folder:")?.split('|').next()?.to_string();
            Some((tack_id, legacy_id))
        })
        .collect();

    let prior_drain_rows: Vec<Value> = db.query("SELECT tack_id, kind, surreal_id FROM tack_reverse_drain_state").await?.take(0)?;
    let mut drained_notes: HashMap<String, String> = HashMap::new();
    let mut drained_folders: HashMap<String, String> = HashMap::new();
    for r in &prior_drain_rows {
        let (Some(tack_id), Some(kind), Some(surreal_id)) =
            (str_field(r, "tack_id"), str_field(r, "kind"), str_field(r, "surreal_id"))
        else {
            continue;
        };
        match kind {
            "note" => drained_notes.insert(tack_id.to_string(), surreal_id.to_string()),
            "folder" => drained_folders.insert(tack_id.to_string(), surreal_id.to_string()),
            _ => None,
        };
    }

    // ── Per-team reconciliation ──────────────────────────────────────────────
    for team_id in &team_ids {
        report.teams_processed.push(team_id.to_string());
        println!("--- Team {team_id} ---");

        let folders_page = tack.list_note_folders(&tack_auth, *team_id).await?;
        // combined tack folder id -> surreal research_folder id (original or reverse-drained)
        let mut folder_reverse_map: HashMap<Uuid, String> = HashMap::new();
        for f in &folders_page.folders {
            if let Some(legacy) = forward_folder_map.get(&f.id.to_string()) {
                folder_reverse_map.insert(f.id, legacy.clone());
                continue;
            }
            if let Some(already) = drained_folders.get(&f.id.to_string()) {
                folder_reverse_map.insert(f.id, already.clone());
                continue;
            }
            // Fresh folder, created in tack after cutover.
            if dry_run {
                println!("  [dry-run] would create research_folder for new tack folder '{}' ({})", f.name, f.id);
                continue;
            }
            let created: Vec<Value> = db
                .query("CREATE research_folder SET name = $name, created_by = $creator")
                .bind(("name", f.name.clone()))
                .bind(("creator", UNKNOWN_AUTHOR_PLACEHOLDER.to_string()))
                .await?
                .take(0)?;
            let Some(surreal_id) = created.first().and_then(|r| str_field(r, "id")).map(str::to_string) else { continue };
            db.query("CREATE tack_reverse_drain_state SET tack_id = $tid, kind = 'folder', surreal_id = $sid")
                .bind(("tid", f.id.to_string()))
                .bind(("sid", surreal_id.clone()))
                .await?;
            folder_reverse_map.insert(f.id, surreal_id);
            report.folders_created += 1;
            println!("  created research_folder for new tack folder '{}' ({})", f.name, f.id);
        }

        let notes_page = tack.list_team_notes(&tack_auth, *team_id).await?;

        // Bulk-resolve every distinct author UUID before the per-note loop.
        let author_ids: HashSet<Uuid> = notes_page.notes.iter().map(|n| n.created_by).collect();
        let author_report = resolve_usernames_for_uuids(&http, &uum_base, &uum_token, &author_ids).await;
        for skipped in &author_report.skipped {
            report.unresolvable_authors.push(skipped.key.clone());
        }

        let seen_tack_ids: HashSet<String> = notes_page.notes.iter().map(|n| n.id.to_string()).collect();

        for note in &notes_page.notes {
            let tack_id = note.id.to_string();

            let trees_for_note: Vec<String> = {
                let attachments = tack.list_attachments(&tack_auth, note.id).await.unwrap_or_default();
                attachments
                    .iter()
                    .filter(|a| a.owning_service == "clann" && a.entity_type == "tree")
                    .filter_map(|a| match tree_id_to_name.get(a.entity_id.as_str()) {
                        Some(name) => Some(name.to_string()),
                        None => {
                            report.unresolvable_trees.push(format!("{tack_id}:{}", a.entity_id));
                            None
                        }
                    })
                    .collect()
            };
            let is_shared = note.visibility != TackVisibility::Private;
            let folder_legacy_id = note.folder_id.and_then(|fid| folder_reverse_map.get(&fid).cloned());

            let description: Option<String> = {
                let meta: Vec<Value> = db
                    .query("SELECT description FROM tack_note_meta WHERE tack_note_id = $tid LIMIT 1")
                    .bind(("tid", tack_id.clone()))
                    .await?
                    .take(0)?;
                match meta.first().and_then(|r| r.get("description")) {
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => {
                        report.descriptions_not_recoverable.push(tack_id.clone());
                        None
                    }
                }
            };

            if let Some(surreal_id) = forward_note_map.get(&tack_id) {
                // Originally migrated -- reconcile editable fields only.
                if dry_run {
                    println!("  [dry-run] would reconcile existing note {surreal_id} (tack {tack_id})");
                } else {
                    db.query(
                        "UPDATE type::record('research_note', $id) SET \
                         title = $title, body = $body, trees = $trees, is_shared = $is_shared",
                    )
                    .bind(("id", surreal_id.strip_prefix("research_note:").unwrap_or(surreal_id).to_string()))
                    .bind(("title", note.title.clone()))
                    .bind(("body", note.body_markdown.clone()))
                    .bind(("trees", trees_for_note.clone()))
                    .bind(("is_shared", is_shared))
                    .await?;
                    report.notes_updated += 1;
                }
            } else if let Some(_existing) = drained_notes.get(&tack_id) {
                // Already reverse-drained on a prior run -- idempotent no-op
                // for creation; still worth reconciling content the same
                // way, but kept simple (create-only tracking) for this
                // rehearsal-stage tool.
            } else {
                // Fresh note, created in tack after cutover.
                let author_username = author_report
                    .resolved
                    .get(&note.created_by.to_string())
                    .cloned()
                    .unwrap_or_else(|| UNKNOWN_AUTHOR_PLACEHOLDER.to_string());
                if dry_run {
                    println!("  [dry-run] would create research_note for new tack note '{}' ({})", note.title, tack_id);
                } else {
                    let created: Vec<Value> = db
                        .query(
                            "CREATE research_note SET title = $title, description = $desc, body = $body, \
                             trees = $trees, folder_id = $folder_id, created_by = $creator, is_shared = $shared",
                        )
                        .bind(("title", note.title.clone()))
                        .bind(("desc", description.clone()))
                        .bind(("body", note.body_markdown.clone()))
                        .bind(("trees", trees_for_note.clone()))
                        .bind(("folder_id", folder_legacy_id.clone()))
                        .bind(("creator", author_username))
                        .bind(("shared", is_shared))
                        .await?
                        .take(0)?;
                    let Some(surreal_id) = created.first().and_then(|r| str_field(r, "id")).map(str::to_string) else {
                        continue;
                    };
                    db.query("CREATE tack_reverse_drain_state SET tack_id = $tid, kind = 'note', surreal_id = $sid")
                        .bind(("tid", tack_id.clone()))
                        .bind(("sid", surreal_id))
                        .await?;
                    report.notes_created += 1;
                    println!("  created research_note for new tack note '{}' ({})", note.title, tack_id);
                }
            }

            // Replies -- position-matched, see this file's own doc comment.
            let surreal_parent = forward_note_map.get(&tack_id).cloned().or_else(|| drained_notes.get(&tack_id).cloned());
            if let Some(parent_id) = surreal_parent {
                let existing_replies: Vec<Value> = db
                    .query("SELECT count() AS n FROM research_note WHERE parent_id = type::record('research_note', $pid) GROUP ALL")
                    .bind(("pid", parent_id.strip_prefix("research_note:").unwrap_or(&parent_id).to_string()))
                    .await?
                    .take(0)?;
                let existing_count = existing_replies.first().and_then(|r| r.get("n")).and_then(Value::as_u64).unwrap_or(0) as usize;

                let tack_replies = tack.list_replies(&tack_auth, note.id).await.unwrap_or_default();
                for reply in tack_replies.iter().skip(existing_count) {
                    let reply_author = author_report.resolved.get(&reply.created_by.to_string()).cloned();
                    let reply_author = match reply_author {
                        Some(u) => u,
                        None => {
                            // Not bulk-resolved above (a reply author distinct
                            // from every top-level note's author) -- resolve
                            // individually here rather than skip; replies are
                            // few relative to notes, so this stays cheap.
                            let ids: HashSet<Uuid> = [reply.created_by].into_iter().collect();
                            resolve_usernames_for_uuids(&http, &uum_base, &uum_token, &ids)
                                .await
                                .resolved
                                .remove(&reply.created_by.to_string())
                                .unwrap_or_else(|| UNKNOWN_AUTHOR_PLACEHOLDER.to_string())
                        }
                    };
                    if dry_run {
                        println!("  [dry-run] would create reply for note {tack_id}");
                        continue;
                    }
                    db.query(
                        "CREATE research_note SET title = $title, body = $body, trees = $trees, \
                         created_by = $creator, is_shared = true, parent_id = type::record('research_note', $pid)",
                    )
                    .bind(("title", format!("Re: {tack_id}")))
                    .bind(("body", reply.body_markdown.clone()))
                    .bind(("trees", trees_for_note.clone()))
                    .bind(("creator", reply_author))
                    .bind(("pid", parent_id.strip_prefix("research_note:").unwrap_or(&parent_id).to_string()))
                    .await?;
                    report.replies_created += 1;
                }
            }
        }

        // Deleted-post-cutover detection: every originally-migrated note for
        // this team whose tack id no longer resolves.
        for (tack_id, surreal_id) in &forward_note_map {
            if !seen_tack_ids.contains(tack_id) {
                match tack.get_note(&tack_auth, Uuid::parse_str(tack_id).unwrap_or(Uuid::nil())).await {
                    Ok(None) => report.notes_deleted_post_cutover_needs_review.push(surreal_id.clone()),
                    _ => {} // still exists (maybe just belongs to a different team) or lookup failed transiently
                }
            }
        }
    }

    write_report(&report)?;
    println!(
        "\n{} folders_created={} notes_created={} notes_updated={} replies_created={} needs_review={} descriptions_lost={}",
        if dry_run { "Dry run complete." } else { "Reverse drain complete." },
        report.folders_created,
        report.notes_created,
        report.notes_updated,
        report.replies_created,
        report.notes_deleted_post_cutover_needs_review.len(),
        report.descriptions_not_recoverable.len()
    );
    Ok(())
}
