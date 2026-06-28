/// Phase 2 migration: create person_proxy records from existing person.trees[] data,
/// rewrite relationship edges from person→person to person_proxy→person_proxy,
/// and migrate life events from person_id to person_proxy_id.
///
/// Run BEFORE starting the new server binary. Idempotent — safe to re-run after partial failure.
/// Rollback: DELETE FROM person_proxy; DELETE FROM has_* WHERE meta::tb(in)="person_proxy";
///           UPDATE life_event SET person_proxy_id = NONE WHERE person_proxy_id != NONE;

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use surrealdb::{engine::any, opt::auth::Root};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn str_field<'a>(row: &'a Value, field: &str) -> Option<&'a str> {
    row.get(field)?.as_str()
}

fn str_arr(row: &Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn bool_field(row: &Value, field: &str) -> bool {
    row.get(field).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn int_field(row: &Value, field: &str) -> Option<i64> {
    row.get(field)?.as_i64()
}

async fn count_q(db: &surrealdb::Surreal<surrealdb::engine::any::Any>, q: &str) -> Result<i64> {
    let rows: Vec<Value> = db.query(q).await?.take(0)?;
    Ok(rows.first().and_then(|r| r["count"].as_i64()).unwrap_or(0))
}

// ── Main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let db_url  = std::env::var("DB_URL")       .unwrap_or_else(|_| "ws://localhost:8000".to_string());
    let db_ns   = std::env::var("DB_NAMESPACE") .unwrap_or_else(|_| "clann".to_string());
    let db_db   = std::env::var("DB_DATABASE")  .unwrap_or_else(|_| "ancestry".to_string());
    let db_user = std::env::var("DB_USERNAME")  .unwrap_or_else(|_| "root".to_string());
    let db_pass = std::env::var("DB_PASSWORD")  .unwrap_or_else(|_| "secret".to_string());

    println!("=== clann-server v26.3 person_proxy migration ===");
    println!("Connecting to {} …", db_url);
    let db = any::connect(&db_url).await?;
    db.signin(Root { username: db_user, password: db_pass }).await?;
    db.use_ns(&db_ns).use_db(&db_db).await?;
    println!("Connected ({db_ns}/{db_db}).\n");

    // ── Pre-flight counts ─────────────────────────────────────────────────────
    println!("--- Pre-flight counts ---");
    let n_persons  = count_q(&db, "SELECT count() AS count FROM person GROUP ALL").await?;
    let n_proxies  = count_q(&db, "SELECT count() AS count FROM person_proxy GROUP ALL").await?;
    let n_events   = count_q(&db, "SELECT count() AS count FROM life_event GROUP ALL").await?;
    let n_fathers  = count_q(&db, "SELECT count() AS count FROM has_father  GROUP ALL").await?;
    let n_mothers  = count_q(&db, "SELECT count() AS count FROM has_mother  GROUP ALL").await?;
    let n_siblings = count_q(&db, "SELECT count() AS count FROM has_sibling GROUP ALL").await?;
    let n_spouses  = count_q(&db, "SELECT count() AS count FROM has_spouse  GROUP ALL").await?;
    println!("  persons:  {n_persons}");
    println!("  proxies:  {n_proxies}");
    println!("  events:   {n_events}");
    println!("  fathers:  {n_fathers}");
    println!("  mothers:  {n_mothers}");
    println!("  siblings: {n_siblings}");
    println!("  spouses:  {n_spouses}\n");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let preflight_file = format!("migration_preflight_{ts}.json");
    std::fs::write(&preflight_file, serde_json::to_string_pretty(&serde_json::json!({
        "timestamp": ts,
        "persons": n_persons, "proxies_before": n_proxies, "life_events": n_events,
        "has_father": n_fathers, "has_mother": n_mothers,
        "has_sibling": n_siblings, "has_spouse": n_spouses,
    }))?)?;
    println!("Pre-flight written to {preflight_file}\n");

    // ── Step A: Create person_proxy records ───────────────────────────────────
    println!("--- Step A: Creating person_proxy records ---");
    // Use meta::id() to get the ULID key as a plain string.
    let persons: Vec<Value> = db
        .query("SELECT meta::id(id) AS id_key, trees, nickname, username, email, \
                verified, biography, image_path, life_image_path, image_bytes, \
                life_image_bytes, created_by FROM person")
        .await?.take(0)?;

    let mut created_count = 0usize;
    let mut skipped_count = 0usize;

    for row in &persons {
        let person_key = match str_field(row, "id_key") { Some(k) => k.to_string(), None => continue };
        let trees = str_arr(row, "trees");

        if trees.is_empty() {
            eprintln!("  WARNING: person:{person_key} has no trees — skipping");
            continue;
        }

        for tree_name in &trees {
            let existing: Vec<Value> = db
                .query(format!(
                    "SELECT meta::id(id) AS id_key FROM person_proxy \
                     WHERE person_id = person:{person_key} AND tree = $tree LIMIT 1"
                ))
                .bind(("tree", tree_name.clone()))
                .await?.take(0)?;

            if !existing.is_empty() {
                skipped_count += 1;
                continue;
            }


            // Embed person_key directly (ULID is alphanumeric — safe).
            // Use RETURN NONE to avoid deserializing the record-typed response field,
            // which would fail with "Expected any, got record" when using serde_json::Value.
            let result = db
                .query(format!(
                    "CREATE person_proxy SET \
                     person_id             = person:{person_key}, \
                     tree                  = $tree, \
                     preferred_family_name = NONE, \
                     preferred_first_name  = NONE, \
                     preferred_middle_name = NONE, \
                     nickname              = $nickname, \
                     username              = $username, \
                     email                 = $email, \
                     verified              = $verified, \
                     biography             = $biography, \
                     image_path            = $image_path, \
                     life_image_path       = $life_image_path, \
                     image_bytes           = $image_bytes, \
                     life_image_bytes      = $life_image_bytes, \
                     is_private            = false, \
                     created_by            = $created_by \
                     RETURN NONE"
                ))
                .bind(("tree",            tree_name.clone()))
                .bind(("nickname",        str_field(row, "nickname").map(|s| s.to_string())))
                .bind(("username",        str_field(row, "username").map(|s| s.to_string())))
                .bind(("email",           str_field(row, "email").map(|s| s.to_string())))
                .bind(("verified",        bool_field(row, "verified")))
                .bind(("biography",       str_field(row, "biography").map(|s| s.to_string())))
                .bind(("image_path",      str_field(row, "image_path").map(|s| s.to_string())))
                .bind(("life_image_path", str_field(row, "life_image_path").map(|s| s.to_string())))
                .bind(("image_bytes",     int_field(row, "image_bytes")))
                .bind(("life_image_bytes",int_field(row, "life_image_bytes")))
                .bind(("created_by",      str_field(row, "created_by").map(|s| s.to_string())))
                .await;

            match result {
                Err(e) => {
                    eprintln!("  ERROR creating proxy for person:{person_key} in '{tree_name}': {e}");
                }
                Ok(_) => {
                    println!("  Created person_proxy for person:{person_key} in tree '{tree_name}'");
                    created_count += 1;
                }
            }
        }
    }
    println!("  Done. created={created_count} skipped_existing={skipped_count}\n");

    // ── Step B: Build lookup map ──────────────────────────────────────────────
    println!("--- Step B: Building lookup map ---");
    let all_proxies: Vec<Value> = db
        .query("SELECT meta::id(id) AS proxy_key, meta::id(person_id) AS person_key, tree FROM person_proxy")
        .await?.take(0)?;

    // (person_ulid, tree_name) → proxy_ulid
    let mut lookup: HashMap<(String, String), String> = HashMap::new();
    for row in &all_proxies {
        if let (Some(proxy_key), Some(person_key), Some(tree)) = (
            str_field(row, "proxy_key"),
            str_field(row, "person_key"),
            str_field(row, "tree"),
        ) {
            lookup.insert((person_key.to_string(), tree.to_string()), proxy_key.to_string());
        }
    }
    println!("  Lookup map: {} entries\n", lookup.len());

    // ── Step C: Migrate relationship edges ────────────────────────────────────
    println!("--- Step C: Migrating relationship edges ---");
    let rel_tables = ["has_father", "has_mother", "has_sibling", "has_spouse"];

    for tbl in &rel_tables {
        let edges: Vec<Value> = db
            .query(format!(
                "SELECT meta::id(in) AS in_key, meta::id(out) AS out_key, \
                 pedigree, sibling_type, via_parent_id, spouse_from, spouse_to \
                 FROM {tbl} WHERE meta::tb(in) = 'person'"
            ))
            .await?.take(0)?;

        println!("  {tbl}: {} old person→person edges to migrate", edges.len());
        let mut edge_created = 0usize;
        let mut edge_skipped = 0usize;

        for edge in &edges {
            let in_key  = match str_field(edge, "in_key")  { Some(k) => k.to_string(), None => continue };
            let out_key = match str_field(edge, "out_key") { Some(k) => k.to_string(), None => continue };

            let in_trees:  Vec<String> = lookup.keys().filter(|(pk, _)| pk == &in_key) .map(|(_, t)| t.clone()).collect();
            let out_trees: Vec<String> = lookup.keys().filter(|(pk, _)| pk == &out_key).map(|(_, t)| t.clone()).collect();
            let shared: Vec<String>    = in_trees.iter().filter(|t| out_trees.contains(t)).cloned().collect();

            if shared.is_empty() {
                eprintln!("  WARNING: {tbl} edge (person:{in_key} → person:{out_key}) has no shared tree — skipping");
                continue;
            }

            for tree_name in &shared {
                let proxy_in_key  = match lookup.get(&(in_key.clone(),  tree_name.clone())) { Some(k) => k.clone(), None => continue };
                let proxy_out_key = match lookup.get(&(out_key.clone(), tree_name.clone())) { Some(k) => k.clone(), None => continue };

                // Use embedded record IDs (ULIDs are safe to embed directly).
                let existing: Vec<Value> = db
                    .query(format!(
                        "SELECT meta::id(id) AS id_key FROM {tbl} \
                         WHERE in = person_proxy:{proxy_in_key} AND out = person_proxy:{proxy_out_key} LIMIT 1"
                    ))
                    .await?.take(0)?;

                if !existing.is_empty() {
                    edge_skipped += 1;
                    continue;
                }

                let pedigree     = str_field(edge, "pedigree").map(|s| s.to_string());
                let sibling_type = str_field(edge, "sibling_type").map(|s| s.to_string());
                let spouse_from  = str_field(edge, "spouse_from").map(|s| s.to_string());
                let spouse_to    = str_field(edge, "spouse_to").map(|s| s.to_string());

                let relate_result = match *tbl {
                    "has_sibling" => {
                        let via = str_field(edge, "via_parent_id").and_then(|v| {
                            let vpid_key = if v.starts_with("person:") { &v[7..] } else { v };
                            lookup.get(&(vpid_key.to_string(), tree_name.clone()))
                                  .map(|k| format!("person_proxy:{k}"))
                        });
                        db.query(format!(
                            "RELATE person_proxy:{proxy_in_key} -> {tbl} -> person_proxy:{proxy_out_key} \
                             CONTENT {{ pedigree: $pedigree, sibling_type: $st, via_parent_id: $via }} \
                             RETURN NONE"
                        ))
                        .bind(("pedigree", pedigree))
                        .bind(("st",       sibling_type.unwrap_or_else(|| "Brother".to_string())))
                        .bind(("via",      via))
                        .await
                    }
                    "has_spouse" => {
                        // has_spouse has no pedigree field — only spouse_from and spouse_to.
                        db.query(format!(
                            "RELATE person_proxy:{proxy_in_key} -> {tbl} -> person_proxy:{proxy_out_key} \
                             CONTENT {{ spouse_from: $sf, spouse_to: $st }} \
                             RETURN NONE"
                        ))
                        .bind(("sf", spouse_from))
                        .bind(("st", spouse_to))
                        .await
                    }
                    _ => {
                        db.query(format!(
                            "RELATE person_proxy:{proxy_in_key} -> {tbl} -> person_proxy:{proxy_out_key} \
                             CONTENT {{ pedigree: $pedigree }} RETURN NONE"
                        ))
                        .bind(("pedigree", pedigree))
                        .await
                    }
                };
                match relate_result {
                    Err(e) => {
                        eprintln!("  ERROR RELATE {tbl} {proxy_in_key}→{proxy_out_key} (transport): {e}");
                        continue;
                    }
                    Ok(mut res) => {
                        // take() as Vec<()> to surface any SurrealDB-level errors.
                        // RETURN NONE produces an empty array [], so Vec<()> will be empty on success.
                        if let Err(e) = res.take::<Vec<()>>(0) {
                            eprintln!("  ERROR RELATE {tbl} {proxy_in_key}→{proxy_out_key}: {e}");
                            continue;
                        }
                    }
                }

                println!("  Created {tbl} edge person_proxy:{proxy_in_key} → person_proxy:{proxy_out_key} in tree '{tree_name}'");
                edge_created += 1;
            }
        }
        println!("  {tbl}: created={edge_created} skipped_existing={edge_skipped}");
    }
    println!();

    // ── Step D: Migrate life events ───────────────────────────────────────────
    println!("--- Step D: Migrating life events ---");
    let unmigrated: Vec<Value> = db
        .query("SELECT meta::id(id) AS id_key, meta::id(person_id) AS person_key \
                FROM life_event WHERE person_proxy_id = NONE")
        .await?.take(0)?;

    println!("  {} unmigrated life events", unmigrated.len());
    let mut event_migrated = 0usize;
    let mut event_skipped  = 0usize;

    for row in &unmigrated {
        let event_key  = match str_field(row, "id_key")     { Some(k) => k.to_string(), None => continue };
        let person_key = match str_field(row, "person_key") {
            Some(k) => k.to_string(),
            None => {
                eprintln!("  WARNING: life_event:{event_key} has no person_id — skipping");
                event_skipped += 1;
                continue;
            }
        };

        let mut proxy_entries: Vec<(String, String)> = lookup.keys()
            .filter(|(pk, _)| pk == &person_key)
            .map(|(_, t)| (t.clone(), lookup[&(person_key.clone(), t.clone())].clone()))
            .collect();
        proxy_entries.sort_by_key(|(t, _)| t.clone());

        if proxy_entries.is_empty() {
            eprintln!("  WARNING: life_event:{event_key} person:{person_key} has no proxy — skipping");
            event_skipped += 1;
            continue;
        }

        let (first_tree, first_proxy_key) = &proxy_entries[0];
        db.query(format!(
            "UPDATE life_event:{event_key} SET \
             person_proxy_id = person_proxy:{first_proxy_key}, \
             contributed_by_tree = $tree, is_canonical = false"
        ))
        .bind(("tree", first_tree.clone()))
        .await?;

        println!("  Migrated life_event:{event_key} → person_proxy:{first_proxy_key} in tree '{first_tree}'");
        event_migrated += 1;

        if proxy_entries.len() > 1 {
            println!("    (person had {} trees; assigned to first. Others: {})",
                proxy_entries.len(),
                proxy_entries[1..].iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(", "));
        }
    }
    println!("  Done. migrated={event_migrated} skipped={event_skipped}\n");

    // ── Step E: Verification ──────────────────────────────────────────────────
    println!("--- Step E: Verification ---");
    let n_proxies_after  = count_q(&db, "SELECT count() AS count FROM person_proxy GROUP ALL").await?;
    // Count proxy→proxy edges (new) and person→person edges (old) separately.
    let n_pp_fathers   = count_q(&db, "SELECT count() AS count FROM has_father  WHERE meta::tb(in) = 'person_proxy' GROUP ALL").await?;
    let n_pp_mothers   = count_q(&db, "SELECT count() AS count FROM has_mother  WHERE meta::tb(in) = 'person_proxy' GROUP ALL").await?;
    let n_pp_siblings  = count_q(&db, "SELECT count() AS count FROM has_sibling WHERE meta::tb(in) = 'person_proxy' GROUP ALL").await?;
    let n_pp_spouses   = count_q(&db, "SELECT count() AS count FROM has_spouse  WHERE meta::tb(in) = 'person_proxy' GROUP ALL").await?;
    let n_old_fathers  = count_q(&db, "SELECT count() AS count FROM has_father  WHERE meta::tb(in) = 'person' GROUP ALL").await?;
    let n_old_mothers  = count_q(&db, "SELECT count() AS count FROM has_mother  WHERE meta::tb(in) = 'person' GROUP ALL").await?;
    let n_old_siblings = count_q(&db, "SELECT count() AS count FROM has_sibling WHERE meta::tb(in) = 'person' GROUP ALL").await?;
    let n_old_spouses  = count_q(&db, "SELECT count() AS count FROM has_spouse  WHERE meta::tb(in) = 'person' GROUP ALL").await?;
    let n_unmig_events = count_q(&db, "SELECT count() AS count FROM life_event WHERE person_proxy_id = NONE GROUP ALL").await?;

    println!("  person_proxy records: {n_proxies_after}");
    println!("  has_father  proxy→proxy: {n_pp_fathers}  (old person→person: {n_old_fathers})");
    println!("  has_mother  proxy→proxy: {n_pp_mothers}  (old person→person: {n_old_mothers})");
    println!("  has_sibling proxy→proxy: {n_pp_siblings}  (old person→person: {n_old_siblings})");
    println!("  has_spouse  proxy→proxy: {n_pp_spouses}  (old person→person: {n_old_spouses})");
    println!("  unmigrated life_events: {n_unmig_events} (these are orphaned — persons with no trees)");

    // proxy counts must be >= original person counts (may be higher for multi-tree persons).
    let edge_ok = n_pp_fathers  >= n_old_fathers
               && n_pp_mothers  >= n_old_mothers
               && n_pp_siblings >= n_old_siblings
               && n_pp_spouses  >= n_old_spouses;
    let ok = edge_ok; // orphaned life events are expected and do not fail the migration

    if !edge_ok {
        if n_pp_fathers  < n_old_fathers  { eprintln!("  WARNING: fewer proxy father edges than original"); }
        if n_pp_mothers  < n_old_mothers  { eprintln!("  WARNING: fewer proxy mother edges than original"); }
        if n_pp_siblings < n_old_siblings { eprintln!("  WARNING: fewer proxy sibling edges than original"); }
        if n_pp_spouses  < n_old_spouses  { eprintln!("  WARNING: fewer proxy spouse edges than original"); }
    }

    let result_file = format!("migration_result_{ts}.json");
    std::fs::write(&result_file, serde_json::to_string_pretty(&serde_json::json!({
        "timestamp": ts,
        "person_proxy_records": n_proxies_after,
        "proxy_fathers": n_pp_fathers, "proxy_mothers": n_pp_mothers,
        "proxy_siblings": n_pp_siblings, "proxy_spouses": n_pp_spouses,
        "unmigrated_life_events": n_unmig_events,
        "ok": ok,
    }))?)?;
    println!("Result written to {result_file}");

    if ok {
        println!("\n✓ Migration completed successfully.");
    } else {
        println!("\n✗ Migration completed with warnings — review output before starting new server.");
    }

    Ok(())
}
