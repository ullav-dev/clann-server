//! Resolution layer for the Clann -> tack-server notes migration's backfill
//! (/Users/colin/.claude/plans/linked-roaming-rabbit.md, Phase 1). Every
//! function here is bulk-loaded once, before the per-note backfill loop
//! (`src/bin/tack_backfill.rs`) runs -- not called once per note -- so a
//! source with N notes referencing M distinct usernames/trees/teams costs
//! O(M) network calls here, not O(N).
//!
//! Each resolver returns a `ResolutionReport<T>`: what resolved cleanly,
//! and what didn't, tagged with a `SkipReason` the per-note loop can attach
//! to its own skip list (grouped by reason code, per the plan's
//! verification section -- "the skip list is a go/no-go gate itself").
//! Nothing here guesses; an ambiguous or missing input is always a skip,
//! never a best-effort fallback -- see the plan's "never resolve ambiguity
//! toward wider exposure" rule, which this module exists to make possible
//! to enforce upstream in the caller.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "reason", content = "detail")]
pub enum SkipReason {
    /// No user in ullav-user-management has this username (deleted/renamed
    /// account -- `research_note.created_by`/`research_folder.created_by`
    /// hold a point-in-time username, not a stable UUID; see
    /// tack-notes-adapter.ts's own IDENTITY comment for the same fact
    /// verified independently on the frontend side).
    UnresolvableUsername,
    /// `GET /admin/users?search=` returned more than one exact
    /// case-insensitive username match. Shouldn't happen -- `username` is
    /// UNIQUE in ullav-user-management's schema (verified directly,
    /// `001_initial.sql`) -- but a resolver that can't happen to raise a
    /// skip instead of picking one silently is the load-bearing kind.
    AmbiguousUsername,
    /// `research_note.trees[]`/`research_folder`'s note references a tree
    /// name with no matching `family_tree.name` row (renamed or deleted
    /// tree since the note was written).
    UnresolvableTreeSlug,
    /// The tree's own `family_tree.team_id` is set, but
    /// `GET /admin/teams/{id}` returned 404 -- the team itself no longer
    /// exists in ullav-user-management.
    TeamNotFound,
    /// The team exists but has no `organization_id` assigned yet --
    /// tack-server's own `resolve_team_organization[_live]` would 400 on
    /// this at note-creation time; caught here instead, in bulk, before the
    /// per-note loop ever starts.
    TeamNoOrganization,
    /// Transport/auth failure talking to ullav-user-management itself --
    /// distinct from "this specific user/team doesn't exist", since a
    /// batch of these usually means the admin token or base URL is wrong,
    /// not that the data is bad.
    LookupFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkipEntry {
    pub key: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ResolutionReport<T> {
    pub resolved: HashMap<String, T>,
    pub skipped: Vec<SkipEntry>,
}

impl<T> ResolutionReport<T> {
    fn skip(&mut self, key: impl Into<String>, reason: SkipReason) {
        self.skipped.push(SkipEntry { key: key.into(), reason });
    }
}

// ── Username -> UUM UUID ─────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct AdminUserSearchRow {
    id: Uuid,
    username: String,
}

#[derive(Debug, serde::Deserialize)]
struct AdminUsersPage {
    users: Vec<AdminUserSearchRow>,
}

/// Resolves every distinct username to its ullav-user-management UUID via
/// `GET /admin/users?search=<username>`. `search` is a substring match
/// server-side (`LOWER(username) LIKE '%...%'`, verified directly against
/// `db::list_users_paginated`), so every result page is filtered down to an
/// exact case-insensitive username match here -- `search` alone is not
/// enough on its own to be a resolution.
pub async fn resolve_usernames(
    http: &reqwest::Client,
    uum_base: &str,
    uum_token: &str,
    usernames: &HashSet<String>,
) -> ResolutionReport<Uuid> {
    let mut report = ResolutionReport::default();

    for username in usernames {
        let resp = http
            .get(format!("{uum_base}/admin/users"))
            .bearer_auth(uum_token)
            .query(&[("search", username.as_str()), ("page_size", "50")])
            .send()
            .await;

        let page: AdminUsersPage = match resp {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(p) => p,
                Err(_) => {
                    report.skip(username, SkipReason::LookupFailed);
                    continue;
                }
            },
            _ => {
                report.skip(username, SkipReason::LookupFailed);
                continue;
            }
        };

        let matches: Vec<&AdminUserSearchRow> = page
            .users
            .iter()
            .filter(|u| u.username.eq_ignore_ascii_case(username))
            .collect();

        match matches.as_slice() {
            [one] => {
                report.resolved.insert(username.clone(), one.id);
            }
            [] => report.skip(username, SkipReason::UnresolvableUsername),
            _ => report.skip(username, SkipReason::AmbiguousUsername),
        }
    }

    report
}

#[derive(Debug, serde::Deserialize)]
struct AdminUserById {
    username: String,
}

/// The reverse of `resolve_usernames` -- UUM UUID -> username, via
/// `GET /admin/users/{id}` (a single exact lookup, no substring-match
/// ambiguity to filter, unlike the forward direction). Used by the
/// reverse-drain tool (`src/bin/tack_reverse_drain.rs`) to convert a tack
/// note's UUID-typed `created_by` back into the username
/// `research_note.created_by` expects.
pub async fn resolve_usernames_for_uuids(
    http: &reqwest::Client,
    uum_base: &str,
    uum_token: &str,
    user_ids: &HashSet<Uuid>,
) -> ResolutionReport<String> {
    let mut report = ResolutionReport::default();

    for user_id in user_ids {
        let key = user_id.to_string();
        let resp = http.get(format!("{uum_base}/admin/users/{user_id}")).bearer_auth(uum_token).send().await;

        match resp {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => {
                report.skip(key, SkipReason::UnresolvableUsername);
            }
            Ok(r) if r.status().is_success() => match r.json::<AdminUserById>().await {
                Ok(u) => {
                    report.resolved.insert(key, u.username);
                }
                Err(_) => report.skip(key, SkipReason::LookupFailed),
            },
            _ => report.skip(key, SkipReason::LookupFailed),
        }
    }

    report
}

// ── Team -> organization UUID ────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct AdminTeamLookup {
    organization_id: Option<Uuid>,
}

/// Resolves every distinct team UUID's `organization_id` via
/// `GET /admin/teams/{id}` -- the same endpoint tack-server's own
/// `resolve_team_organization_live` calls at request time, so a team this
/// resolves as `TeamNoOrganization` is exactly a team that would 400 out of
/// `tack_client::create_note`/`create_note_folder` if it slipped through
/// unfiltered. Resolving it here, once per distinct team before the
/// per-note loop, turns that into one clean skip-list entry per team
/// instead of one per note referencing it.
pub async fn resolve_team_organizations(
    http: &reqwest::Client,
    uum_base: &str,
    uum_token: &str,
    team_ids: &HashSet<String>,
) -> ResolutionReport<Uuid> {
    let mut report = ResolutionReport::default();

    for team_id in team_ids {
        let resp = http
            .get(format!("{uum_base}/admin/teams/{team_id}"))
            .bearer_auth(uum_token)
            .send()
            .await;

        match resp {
            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => {
                report.skip(team_id, SkipReason::TeamNotFound);
            }
            Ok(r) if r.status().is_success() => match r.json::<AdminTeamLookup>().await {
                Ok(AdminTeamLookup { organization_id: Some(org_id) }) => {
                    report.resolved.insert(team_id.clone(), org_id);
                }
                Ok(AdminTeamLookup { organization_id: None }) => {
                    report.skip(team_id, SkipReason::TeamNoOrganization);
                }
                Err(_) => report.skip(team_id, SkipReason::LookupFailed),
            },
            _ => report.skip(team_id, SkipReason::LookupFailed),
        }
    }

    report
}

// ── Tree name slug -> family_tree id + team_id ──────────────────────────────

#[derive(Debug, Clone)]
pub struct TreeInfo {
    /// `family_tree` record's ULID (the `meta::id(id)` form, no
    /// `family_tree:` prefix) -- the plan's DECIDED canonical
    /// `content_attachments.entity_id` (see tack-notes-adapter.ts's own
    /// comment on this same decision from the frontend side). Not yet a
    /// UUID -- clann-server's own record ids are ULIDs, not UUIDs; tack's
    /// `content_attachments.entity_id` is a free-form string column, so
    /// this is fine as-is, no cross-system UUID needed.
    pub tree_id: String,
    pub team_id: Option<String>,
}

/// Pure, no I/O -- takes already-fetched `family_tree` rows (e.g. from the
/// Phase 0 census script's own dump, or a fresh `SELECT` in the backfill
/// binary) rather than owning a DB connection itself, so it's trivially
/// unit-testable without a live SurrealDB. `name` is `family_tree`'s own
/// uniquely-indexed slug column (`idx_tree_name UNIQUE`, verified directly
/// against `migrations/schema.surql`) -- the same field
/// `research_note.trees[]` stores.
pub fn resolve_trees<'a, I>(trees: I) -> HashMap<String, TreeInfo>
where
    I: IntoIterator<Item = (&'a str, &'a str, Option<&'a str>)>,
    // (name, tree_id, team_id)
{
    trees
        .into_iter()
        .map(|(name, tree_id, team_id)| {
            (name.to_string(), TreeInfo { tree_id: tree_id.to_string(), team_id: team_id.map(str::to_string) })
        })
        .collect()
}

/// Looks up every tree slug a note references, distinguishing "resolves
/// cleanly" from "at least one slug doesn't exist" -- callers combine this
/// with the visibility policy (`src/backfill_visibility.rs`, added
/// alongside the per-note loop) rather than this function making a
/// visibility call itself; resolution and policy are deliberately kept as
/// separate, independently swappable pieces (see the plan's Phase 1 build
/// order).
pub fn resolve_note_trees<'a>(
    tree_slugs: &'a [String],
    trees: &'a HashMap<String, TreeInfo>,
) -> (Vec<&'a TreeInfo>, Vec<&'a str>) {
    let mut resolved = Vec::new();
    let mut unresolvable = Vec::new();
    for slug in tree_slugs {
        match trees.get(slug) {
            Some(info) => resolved.push(info),
            None => unresolvable.push(slug.as_str()),
        }
    }
    (resolved, unresolvable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn resolve_usernames_exact_match_only() {
        let server = MockServer::start().await;
        // "sam" search returns two candidates; only "sam" itself is an exact match.
        Mock::given(method("GET"))
            .and(path("/admin/users"))
            .and(query_param("search", "sam"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "users": [
                    {"id": "11111111-1111-1111-1111-111111111111", "username": "sam"},
                    {"id": "22222222-2222-2222-2222-222222222222", "username": "samantha"}
                ],
                "total": 2
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/admin/users"))
            .and(query_param("search", "ghost"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "users": [], "total": 0 })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let usernames: HashSet<String> = ["sam".to_string(), "ghost".to_string()].into_iter().collect();
        let report = resolve_usernames(&http, &server.uri(), "tok", &usernames).await;

        assert_eq!(
            report.resolved.get("sam").unwrap().to_string(),
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].key, "ghost");
        assert_eq!(report.skipped[0].reason, SkipReason::UnresolvableUsername);
    }

    #[tokio::test]
    async fn resolve_usernames_ambiguous_is_a_skip_not_a_guess() {
        let server = MockServer::start().await;
        // Pathological case: search substring-matches two rows that BOTH
        // happen to equal the query exactly under case-folding (shouldn't
        // occur given the UNIQUE constraint, but the resolver must not
        // silently pick one if it somehow did).
        Mock::given(method("GET"))
            .and(path("/admin/users"))
            .and(query_param("search", "dup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "users": [
                    {"id": "11111111-1111-1111-1111-111111111111", "username": "dup"},
                    {"id": "22222222-2222-2222-2222-222222222222", "username": "DUP"}
                ],
                "total": 2
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let usernames: HashSet<String> = ["dup".to_string()].into_iter().collect();
        let report = resolve_usernames(&http, &server.uri(), "tok", &usernames).await;

        assert!(report.resolved.is_empty());
        assert_eq!(report.skipped[0].reason, SkipReason::AmbiguousUsername);
    }

    #[tokio::test]
    async fn resolve_usernames_for_uuids_round_trips_and_flags_deleted() {
        let server = MockServer::start().await;
        let known = "11111111-1111-1111-1111-111111111111";
        let deleted = "22222222-2222-2222-2222-222222222222";

        Mock::given(method("GET"))
            .and(path(format!("/admin/users/{known}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "username": "colin" })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/admin/users/{deleted}")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let ids: HashSet<Uuid> = [known, deleted].into_iter().map(|s| Uuid::parse_str(s).unwrap()).collect();
        let report = resolve_usernames_for_uuids(&http, &server.uri(), "tok", &ids).await;

        assert_eq!(report.resolved.get(known).unwrap(), "colin");
        assert_eq!(report.skipped.iter().find(|s| s.key == deleted).unwrap().reason, SkipReason::UnresolvableUsername);
    }

    #[tokio::test]
    async fn resolve_team_organizations_distinguishes_missing_from_unassigned() {
        let server = MockServer::start().await;
        let team_with_org = "11111111-1111-1111-1111-111111111111";
        let team_no_org = "22222222-2222-2222-2222-222222222222";
        let team_missing = "33333333-3333-3333-3333-333333333333";

        Mock::given(method("GET"))
            .and(path(format!("/admin/teams/{team_with_org}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "organization_id": "44444444-4444-4444-4444-444444444444"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/admin/teams/{team_no_org}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "organization_id": null })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/admin/teams/{team_missing}")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let team_ids: HashSet<String> =
            [team_with_org, team_no_org, team_missing].into_iter().map(str::to_string).collect();
        let report = resolve_team_organizations(&http, &server.uri(), "tok", &team_ids).await;

        assert_eq!(
            report.resolved.get(team_with_org).unwrap().to_string(),
            "44444444-4444-4444-4444-444444444444"
        );
        let reason_for = |id: &str| report.skipped.iter().find(|s| s.key == id).unwrap().reason.clone();
        assert_eq!(reason_for(team_no_org), SkipReason::TeamNoOrganization);
        assert_eq!(reason_for(team_missing), SkipReason::TeamNotFound);
    }

    #[test]
    fn resolve_trees_and_note_trees_split_resolvable_from_not() {
        let rows = vec![
            ("smith-family", "01ARZ3NDEKTSV4RRFFQ69G5FAV", Some("team-uuid-1")),
            ("orphan-tree", "01ARZ3NDEKTSV4RRFFQ69G5FAW", None),
        ];
        let trees = resolve_trees(rows);

        let slugs = vec!["smith-family".to_string(), "orphan-tree".to_string(), "ghost-tree".to_string()];
        let (resolved, unresolvable) = resolve_note_trees(&slugs, &trees);

        assert_eq!(resolved.len(), 2);
        assert_eq!(unresolvable, vec!["ghost-tree"]);
        assert_eq!(trees["smith-family"].team_id.as_deref(), Some("team-uuid-1"));
        assert_eq!(trees["orphan-tree"].team_id, None);
    }
}
