//! Phase 1 backfill: clann-server's `research_note`/`research_folder`
//! (SurrealDB) -> tack-server's Notes API
//! (/Users/colin/.claude/plans/linked-roaming-rabbit.md).
//!
//! ADDITIVE ONLY. Never mutates `research_note`/`research_folder` -- the
//! SurrealDB source stays a fully intact rollback target throughout (see
//! the plan's "Rollback strategy"). Idempotency lives entirely in
//! `tack_migration_state` (a live SurrealDB table, checked before every
//! write), not a separate state file -- simpler than the plan's original
//! "state file + durable column" design now that folder targets are
//! derived before any write happens (see Step A0 below), which removed
//! the need for a repoint-after-the-fact phase entirely.
//!
//! Structure, in order:
//!   A0 (derive)  -- resolve every input, decide every note's visibility
//!                    policy outcome, derive the exact (folder, team) set
//!                    that Step A needs to create. Zero writes. This alone
//!                    IS `--dry-run`'s entire output.
//!   A  (folders) -- create derived (legacy_folder_id, team_id) pairs.
//!   B  (notes)   -- create notes + replies + tree attachments + the
//!                    tack_note_meta sidecar, using A0's decisions and A's
//!                    folder map directly (no separate repoint step --
//!                    the target folder is already known before the note
//!                    is created).
//!
//! `--dry-run`: runs A0, prints/writes the full report, performs zero
//! writes to tack-server, clann-server's own SurrealDB, or
//! `tack_migration_state`.
//!
//! Run BEFORE any user-facing cutover. Safe to re-run at any time --
//! already-migrated folders/notes are skipped via `tack_migration_state`.

use anyhow::Result;
use clann_server::backfill::{resolve_note_trees, resolve_team_organizations, resolve_trees, resolve_usernames, TreeInfo};
use clann_server::backfill_visibility::{decide, AmbiguityReason, Decision, SkipReason as VisibilitySkip, Visibility as DecidedVisibility};
use clann_server::tack_client::{TackClient, Visibility as TackVisibility};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use surrealdb::{engine::any, opt::auth::Root};
use uuid::Uuid;

// ── Small JSON row helpers (same idiom as migrate_proxy.rs) ─────────────────

fn str_field<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    row.get(field)?.as_str()
}
fn str_arr(row: &Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}
fn bool_field(row: &Value, field: &str) -> bool {
    row.get(field).and_then(|v| v.as_bool()).unwrap_or(false)
}

// ── Row shapes read from SurrealDB (plain strings via meta::id(), no RecordId parsing) ──

#[derive(Debug, Clone)]
struct FolderRow {
    id: String,
    name: String,
    created_by: String,
}

#[derive(Debug, Clone)]
struct NoteRow {
    id: String,
    title: String,
    description: Option<String>,
    body: String,
    trees: Vec<String>,
    folder_id: Option<String>,
    created_by: Option<String>,
    created_at: Option<String>,
    is_shared: bool,
    parent_id: Option<String>,
}

fn dam_url_count(body: &str) -> usize {
    // A rough, deliberately simple parity check -- see this file's own
    // verification section doc comment. Not meant to be a precise markdown
    // parser, just a stable count to diff pre/post migration.
    body.matches("](http").count()
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ── Report shapes ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct SkipRow {
    note_id: String,
    reason: String,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
struct AmbiguousRow {
    note_id: String,
    chosen_team_id: String,
    all_candidate_teams: Vec<String>,
}

#[derive(Debug, Serialize, Default)]
struct Report {
    dry_run: bool,
    total_folders: usize,
    total_top_level_notes: usize,
    total_replies: usize,
    resolved_usernames: usize,
    unresolved_usernames: Vec<String>,
    resolved_teams_with_org: usize,
    teams_missing_org: Vec<String>,
    derived_folders_needed: usize,
    notes_to_migrate: usize,
    notes_ambiguous: Vec<AmbiguousRow>,
    notes_skipped: Vec<SkipRow>,
    folders_created: usize,
    folders_already_migrated: usize,
    notes_created: usize,
    notes_already_migrated: usize,
    replies_created: usize,
    replies_skipped_unresolvable_author: usize,
    body_sha256_mismatches: Vec<String>,
    dam_url_count_mismatches: Vec<String>,
}

fn visibility_skip_str(r: VisibilitySkip) -> &'static str {
    match r {
        VisibilitySkip::NoTrees => "no_trees",
        VisibilitySkip::AllTreeSlugsUnresolvable => "all_tree_slugs_unresolvable",
        VisibilitySkip::NoTeamOnAnyResolvedTree => "no_team_on_any_resolved_tree",
    }
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

    println!("=== Clann -> tack-server notes backfill {} ===", if dry_run { "(DRY RUN)" } else { "(LIVE)" });
    if !dry_run && (uum_token.is_empty() || tack_token.is_empty()) {
        anyhow::bail!("UUM_ADMIN_TOKEN and TACK_BACKFILL_TOKEN are required for a live run (omit --dry-run to require them, or set both)");
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

    // ── Fetch source rows ──────────────────────────────────────────────────
    println!("--- Fetching source rows ---");
    let tree_rows: Vec<Value> = db
        .query("SELECT name, meta::id(id) AS tree_id, team_id FROM family_tree")
        .await?
        .take(0)?;
    let folder_rows_raw: Vec<Value> = db
        .query("SELECT meta::id(id) AS id, name, created_by FROM research_folder")
        .await?
        .take(0)?;
    // Split top-level/replies rather than a single `meta::id(parent_id)`
    // projection -- `meta::id()` requires a `record` argument and errors
    // outright on `NONE` (verified directly against a real local dev
    // instance: top-level notes have no `parent_id` at all, not a record).
    let top_level_rows_raw: Vec<Value> = db
        .query(
            "SELECT meta::id(id) AS id, title, description, body, trees, folder_id, \
             created_by, created_at, is_shared \
             FROM research_note WHERE parent_id = NONE ORDER BY created_at ASC",
        )
        .await?
        .take(0)?;
    let reply_rows_raw: Vec<Value> = db
        .query(
            "SELECT meta::id(id) AS id, title, description, body, trees, folder_id, \
             created_by, created_at, is_shared, meta::id(parent_id) AS parent_id \
             FROM research_note WHERE parent_id != NONE ORDER BY created_at ASC",
        )
        .await?
        .take(0)?;

    let folders: Vec<FolderRow> = folder_rows_raw
        .iter()
        .filter_map(|r| {
            Some(FolderRow {
                id: str_field(r, "id")?.to_string(),
                name: str_field(r, "name")?.to_string(),
                created_by: str_field(r, "created_by").unwrap_or("").to_string(),
            })
        })
        .collect();

    fn parse_note(r: &Value) -> Option<NoteRow> {
        Some(NoteRow {
            id: str_field(r, "id")?.to_string(),
            title: str_field(r, "title").unwrap_or("").to_string(),
            description: str_field(r, "description").map(str::to_string),
            body: str_field(r, "body").unwrap_or("").to_string(),
            trees: str_arr(r, "trees"),
            folder_id: str_field(r, "folder_id").map(str::to_string),
            created_by: str_field(r, "created_by").map(str::to_string),
            created_at: str_field(r, "created_at").map(str::to_string),
            is_shared: bool_field(r, "is_shared"),
            parent_id: str_field(r, "parent_id").map(str::to_string),
        })
    }

    let top_level_owned: Vec<NoteRow> = top_level_rows_raw.iter().filter_map(parse_note).collect();
    let reply_rows: Vec<NoteRow> = reply_rows_raw.iter().filter_map(parse_note).collect();

    let top_level: Vec<&NoteRow> = top_level_owned.iter().collect();
    let replies_by_parent: HashMap<String, Vec<&NoteRow>> = {
        let mut m: HashMap<String, Vec<&NoteRow>> = HashMap::new();
        for n in &reply_rows {
            if let Some(parent) = &n.parent_id {
                m.entry(parent.clone()).or_default().push(n);
            }
        }
        m
    };

    report.total_folders = folders.len();
    report.total_top_level_notes = top_level.len();
    report.total_replies = reply_rows.len();
    println!(
        "  folders={} top_level_notes={} replies={}\n",
        report.total_folders, report.total_top_level_notes, report.total_replies
    );

    // ── A0: resolve everything, decide everything, derive folder set ───────
    println!("--- Step A0: resolving inputs ---");

    let trees: HashMap<String, TreeInfo> = resolve_trees(tree_rows.iter().filter_map(|r| {
        Some((str_field(r, "name")?, str_field(r, "tree_id")?, str_field(r, "team_id")))
    }));
    println!("  trees resolved from family_tree: {}", trees.len());

    let mut usernames: HashSet<String> = HashSet::new();
    for f in &folders {
        if !f.created_by.is_empty() {
            usernames.insert(f.created_by.clone());
        }
    }
    for n in top_level_owned.iter().chain(reply_rows.iter()) {
        if let Some(u) = &n.created_by {
            usernames.insert(u.clone());
        }
    }
    let username_report = resolve_usernames(&http, &uum_base, &uum_token, &usernames).await;
    report.resolved_usernames = username_report.resolved.len();
    report.unresolved_usernames = username_report.skipped.iter().map(|s| s.key.clone()).collect();
    println!(
        "  usernames resolved={} unresolved={}",
        username_report.resolved.len(),
        username_report.skipped.len()
    );

    // Distinct team ids referenced by any resolved tree.
    let candidate_team_ids: HashSet<String> = trees.values().filter_map(|t| t.team_id.clone()).collect();
    let team_org_report = resolve_team_organizations(&http, &uum_base, &uum_token, &candidate_team_ids).await;
    report.resolved_teams_with_org = team_org_report.resolved.len();
    report.teams_missing_org = team_org_report.skipped.iter().map(|s| s.key.clone()).collect();
    println!(
        "  teams with organization={} without={}\n",
        team_org_report.resolved.len(),
        team_org_report.skipped.len()
    );

    // Per-note decision.
    struct NoteDecision<'a> {
        note: &'a NoteRow,
        decision: Decision,
    }
    let mut decisions: Vec<NoteDecision> = Vec::new();
    for note in &top_level {
        let (resolved_trees, _unresolvable) = resolve_note_trees(&note.trees, &trees);
        let had_any_slugs = !note.trees.is_empty();
        let had_any_resolved_trees = !resolved_trees.is_empty();
        // Only a team that BOTH belongs to a resolved tree AND has a
        // resolved organization is usable -- a team missing an org would
        // make tack_client::create_note 400 at write time regardless of
        // what decide() picks, so it's excluded from the candidate set
        // here rather than left for Step B to discover the hard way.
        let resolved_team_ids: BTreeSet<String> = resolved_trees
            .iter()
            .filter_map(|t| t.team_id.clone())
            .filter(|team_id| team_org_report.resolved.contains_key(team_id))
            .collect();
        let d = decide(note.is_shared, &resolved_team_ids, had_any_slugs, had_any_resolved_trees);
        decisions.push(NoteDecision { note, decision: d });
    }

    for nd in &decisions {
        match &nd.decision {
            Decision::Skip(reason) => {
                report.notes_skipped.push(SkipRow {
                    note_id: nd.note.id.clone(),
                    reason: visibility_skip_str(*reason).to_string(),
                    detail: None,
                });
            }
            Decision::MigrateAmbiguous(target, AmbiguityReason::MultiTeam) => {
                let (resolved_trees, _) = resolve_note_trees(&nd.note.trees, &trees);
                let all_candidates: Vec<String> = resolved_trees
                    .iter()
                    .filter_map(|t| t.team_id.clone())
                    .filter(|t| team_org_report.resolved.contains_key(t))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                report.notes_ambiguous.push(AmbiguousRow {
                    note_id: nd.note.id.clone(),
                    chosen_team_id: target.team_id.clone(),
                    all_candidate_teams: all_candidates,
                });
            }
            Decision::Migrate(_) => {}
        }
    }
    report.notes_to_migrate =
        decisions.iter().filter(|nd| !matches!(nd.decision, Decision::Skip(_))).count();

    // Derived (legacy_folder_id, team_id) pairs Step A needs to create --
    // see this file's own top doc comment on why this replaces a separate
    // repoint phase entirely.
    let mut needed_folders: BTreeMap<(String, String), ()> = BTreeMap::new();
    for nd in &decisions {
        let target = match &nd.decision {
            Decision::Migrate(t) | Decision::MigrateAmbiguous(t, _) => t,
            Decision::Skip(_) => continue,
        };
        if let Some(folder_id) = &nd.note.folder_id {
            needed_folders.insert((folder_id.clone(), target.team_id.clone()), ());
        }
    }
    report.derived_folders_needed = needed_folders.len();

    println!("--- A0 summary ---");
    println!("  notes to migrate: {}", report.notes_to_migrate);
    println!("  notes ambiguous (multi-team, forced private): {}", report.notes_ambiguous.len());
    println!("  notes skipped: {}", report.notes_skipped.len());
    println!("  folders needed (legacy_folder, team) pairs: {}\n", report.derived_folders_needed);

    if dry_run {
        write_report(&report)?;
        println!("Dry run complete -- zero writes performed. See report file above for the full skip/ambiguity list.");
        return Ok(());
    }

    // ── Step A: create derived folders ──────────────────────────────────────
    println!("--- Step A: creating folders ---");
    let folder_by_id: HashMap<&str, &FolderRow> = folders.iter().map(|f| (f.id.as_str(), f)).collect();
    // (legacy_folder_id, team_id) -> tack folder uuid
    let mut folder_map: HashMap<(String, String), Uuid> = HashMap::new();

    for (legacy_folder_id, team_id) in needed_folders.keys() {
        let name = folder_by_id.get(legacy_folder_id.as_str()).map(|f| f.name.as_str()).unwrap_or("Unfiled");
        let state_key = format!("research_folder:{legacy_folder_id}|{team_id}");

        let existing: Vec<Value> = db
            .query("SELECT tack_id FROM tack_migration_state WHERE surreal_id = $sid AND kind = 'folder' LIMIT 1")
            .bind(("sid", state_key.clone()))
            .await?
            .take(0)?;

        if let Some(row) = existing.first() {
            if let Some(tack_id) = str_field(row, "tack_id") {
                if let Ok(uuid) = Uuid::parse_str(tack_id) {
                    folder_map.insert((legacy_folder_id.clone(), team_id.clone()), uuid);
                    report.folders_already_migrated += 1;
                    continue;
                }
            }
        }

        let team_uuid = match Uuid::parse_str(team_id) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let created = tack.create_note_folder(&tack_auth, team_uuid, name).await?;
        db.query("CREATE tack_migration_state SET surreal_id = $sid, kind = 'folder', tack_id = $tid")
            .bind(("sid", state_key))
            .bind(("tid", created.id.to_string()))
            .await?;
        folder_map.insert((legacy_folder_id.clone(), team_id.clone()), created.id);
        report.folders_created += 1;
        println!("  created folder '{name}' for team {team_id} -> {}", created.id);
    }
    println!(
        "  folders: created={} already_migrated={}\n",
        report.folders_created, report.folders_already_migrated
    );

    // ── Step B: create notes + replies + attachments + sidecar ─────────────
    println!("--- Step B: creating notes ---");
    for nd in &decisions {
        let target = match &nd.decision {
            Decision::Migrate(t) | Decision::MigrateAmbiguous(t, _) => t.clone(),
            Decision::Skip(_) => continue,
        };
        let note = nd.note;

        let existing: Vec<Value> = db
            .query("SELECT tack_id FROM tack_migration_state WHERE surreal_id = $sid AND kind = 'note' LIMIT 1")
            .bind(("sid", note.id.clone()))
            .await?
            .take(0)?;
        if let Some(row) = existing.first() {
            if str_field(row, "tack_id").is_some() {
                report.notes_already_migrated += 1;
                continue;
            }
        }

        let author_username = match &note.created_by {
            Some(u) => u,
            None => {
                report.notes_skipped.push(SkipRow {
                    note_id: note.id.clone(),
                    reason: "no_created_by".into(),
                    detail: None,
                });
                continue;
            }
        };
        let author_uuid = match username_report.resolved.get(author_username) {
            Some(u) => *u,
            None => {
                report.notes_skipped.push(SkipRow {
                    note_id: note.id.clone(),
                    reason: visibility_skip_str_username_unresolved(),
                    detail: Some(author_username.clone()),
                });
                continue;
            }
        };
        let team_uuid = match Uuid::parse_str(&target.team_id) {
            Ok(u) => u,
            Err(_) => continue,
        };
        let visibility = match target.visibility {
            DecidedVisibility::Team => TackVisibility::Team,
            DecidedVisibility::Private => TackVisibility::Private,
        };
        let folder_uuid = note
            .folder_id
            .as_ref()
            .and_then(|fid| folder_map.get(&(fid.clone(), target.team_id.clone())))
            .copied();
        let created_at = note.created_at.as_deref().and_then(|s| s.parse().ok());

        let body_before_hash = sha256_hex(&note.body);
        let dam_before = dam_url_count(&note.body);

        let created = tack
            .create_note(
                &tack_auth,
                team_uuid,
                visibility,
                &note.title,
                &note.body,
                folder_uuid,
                Some(author_uuid),
                created_at,
            )
            .await?;

        let body_after_hash = sha256_hex(&created.body_markdown);
        if body_after_hash != body_before_hash {
            report.body_sha256_mismatches.push(note.id.clone());
        }
        let dam_after = dam_url_count(&created.body_markdown);
        if dam_after != dam_before {
            report.dam_url_count_mismatches.push(note.id.clone());
        }

        db.query("CREATE tack_migration_state SET surreal_id = $sid, kind = 'note', tack_id = $tid")
            .bind(("sid", note.id.clone()))
            .bind(("tid", created.id.to_string()))
            .await?;

        if note.description.is_some() {
            db.query("CREATE tack_note_meta SET tack_note_id = $tid, description = $desc")
                .bind(("tid", created.id.to_string()))
                .bind(("desc", note.description.clone()))
                .await?;
        }

        let (resolved_trees, _) = resolve_note_trees(&note.trees, &trees);
        for tree_info in resolved_trees {
            let _ = tack.attach(&tack_auth, created.id, "clann", "tree", &tree_info.tree_id).await;
        }

        report.notes_created += 1;

        // Replies -- ordering already ASC from the source query.
        if let Some(replies) = replies_by_parent.get(&note.id) {
            for reply in replies {
                let reply_author = match &reply.created_by {
                    Some(u) => username_report.resolved.get(u).copied(),
                    None => None,
                };
                let reply_author = match reply_author {
                    Some(u) => u,
                    None => {
                        report.replies_skipped_unresolvable_author += 1;
                        continue;
                    }
                };
                let reply_created_at = reply.created_at.as_deref().and_then(|s| s.parse().ok());
                match tack
                    .create_reply(&tack_auth, created.id, &reply.body, Some(reply_author), reply_created_at)
                    .await
                {
                    Ok(_) => report.replies_created += 1,
                    Err(e) => eprintln!("  ERROR creating reply for note {}: {e}", note.id),
                }
            }
        }
    }
    println!(
        "  notes: created={} already_migrated={} replies_created={} replies_skipped_unresolvable_author={}\n",
        report.notes_created, report.notes_already_migrated, report.replies_created, report.replies_skipped_unresolvable_author
    );

    write_report(&report)?;
    if report.body_sha256_mismatches.is_empty() && report.dam_url_count_mismatches.is_empty() {
        println!("\n✓ Backfill completed. No body-hash or DAM-URL-count mismatches.");
    } else {
        println!(
            "\n✗ Backfill completed WITH DISCREPANCIES -- {} body mismatches, {} DAM-URL-count mismatches. Review the report before treating this run as clean.",
            report.body_sha256_mismatches.len(),
            report.dam_url_count_mismatches.len()
        );
    }
    Ok(())
}

fn visibility_skip_str_username_unresolved() -> String {
    "unresolvable_author_username".to_string()
}

fn write_report(report: &Report) -> Result<()> {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let kind = if report.dry_run { "dryrun" } else { "backfill" };
    let file = format!("tack_{kind}_report_{ts}.json");
    std::fs::write(&file, serde_json::to_string_pretty(report)?)?;
    println!("Report written to {file}");
    Ok(())
}
