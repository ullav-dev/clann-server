/// Clann MCP server — genealogy tools over Streamable HTTP.
///
/// Auth is validated by the Axum MCP middleware before requests reach this
/// service. The validated username from the JWT token is injected via
/// `AUTHENTICATED_USERNAME` task-local storage so that tools never accept
/// a username as a parameter — the caller's identity is always bound to
/// the token, not to a string typed into chat.
///
/// Privacy rule: tools never expose another user's username or identifying
/// information. Only genealogical facts and opaque proxy IDs cross the
/// boundary.

use std::sync::Arc;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpService, StreamableHttpServerConfig,
    session::local::LocalSessionManager,
};
use schemars::JsonSchema;
use serde::Deserialize;
use surrealdb::types::RecordId;

use crate::db::Db;

// Task-local storage for the username extracted from the validated JWT.
// Set by the MCP auth middleware; read by the ClannServer factory.
tokio::task_local! {
    pub(crate) static AUTHENTICATED_USERNAME: String;
}

// ── Parameter types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTreesParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchPersonsParams {
    /// Slug name of the family tree to search (e.g. `"smith-family"`).
    pub tree_name: String,
    /// Search string matched against first and family names (case-insensitive).
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPersonParams {
    /// Person proxy record ID — either the full `person_proxy:<id>` form or just the `<id>` part.
    pub person_proxy_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetFamilyParams {
    /// Person proxy record ID — either the full `person_proxy:<id>` form or just the `<id>` part.
    pub person_proxy_id: String,
    /// If true, also return this person's siblings (others sharing at least one parent).
    /// Defaults to false.
    #[serde(default)]
    pub include_siblings: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindDuplicatesParams {
    /// Person proxy record ID of the person to search duplicates for.
    pub person_proxy_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListContactRequestsParams {
    /// Optional role filter: `"sent"`, `"received"`, or omit for all.
    pub role: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateContactRequestParams {
    /// Proxy ID of the person in your own tree (e.g. `person_proxy:<ulid>`).
    pub from_proxy_id: String,
    /// Proxy IDs of the matched persons in other trees to contact (from `find_duplicates`).
    pub target_proxy_ids: Vec<String>,
    /// Opening message to send. Claude will suggest one, but it can be edited before sending.
    pub message: String,
}

// ── Server ────────────────────────────────────────────────────────────────────

pub struct ClannServer {
    db: Db,
    /// Username extracted from the validated JWT — never supplied by the caller.
    username: String,
}

impl ClannServer {
    fn new(db: Db, username: String) -> Self {
        Self { db, username }
    }
}

#[tool_router]
impl ClannServer {
    /// List family trees owned by a given user.
    #[tool(description = "List family trees owned by the authenticated user")]
    async fn list_trees(
        &self,
        Parameters(_p): Parameters<ListTreesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let db = self.db.lock().await;
        let rows: Vec<serde_json::Value> = db
            .query("SELECT <string>id AS id, name, display_name, is_primary, team_id FROM family_tree WHERE owner = $username")
            .bind(("username", self.username.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        Ok(to_safe_json(serde_json::to_value(&rows).unwrap()))
    }

    /// Search for persons by name within a family tree.
    ///
    /// Returns up to 20 matches. The `person_proxy_id` in each result can be
    /// passed to `get_person` or `get_family` for further detail.
    #[tool(
        description = "Search for persons by name within a family tree. Returns up to 20 matches."
    )]
    async fn search_persons(
        &self,
        Parameters(p): Parameters<SearchPersonsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let db = self.db.lock().await;
        // Fetch all proxies in the tree then filter in Rust; SurrealDB embedded
        // engine does not support string::lowercase() with the ?? operator in the
        // same expression, so we keep the query simple.
        let rows: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>id AS person_proxy_id, \
                    preferred_first_name ?? person_id.first_name AS first_name, \
                    preferred_family_name ?? person_id.family_name AS family_name, \
                    person_id.date_of_birth AS date_of_birth, \
                    person_id.date_of_death AS date_of_death, \
                    is_private \
                 FROM person_proxy \
                 WHERE tree = $tree \
                 LIMIT 500",
            )
            .bind(("tree", p.tree_name))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let q = p.query.to_lowercase();
        let matches: Vec<&serde_json::Value> = rows
            .iter()
            .filter(|r| {
                let first = r.get("first_name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                let family = r.get("family_name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
                first.contains(&q) || family.contains(&q) || format!("{first} {family}").contains(&q)
            })
            .take(20)
            .collect();

        Ok(to_safe_json(serde_json::to_value(&matches).unwrap()))
    }

    /// Get full details of a person by their proxy ID (includes canonical birth/death data).
    #[tool(description = "Get full details of a person by their proxy record ID")]
    async fn get_person(
        &self,
        Parameters(p): Parameters<GetPersonParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let proxy_rid = parse_proxy_id(&p.person_proxy_id)?;
        let db = self.db.lock().await;

        let proxy: Option<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>id AS person_proxy_id, \
                    <string>person_id AS canonical_person_id, \
                    tree, \
                    preferred_first_name, preferred_family_name, preferred_middle_name, \
                    nickname, biography, is_private, verified, \
                    person_id.first_name AS canonical_first_name, \
                    person_id.family_name AS canonical_family_name, \
                    person_id.middle_name AS canonical_middle_name, \
                    person_id.sex AS sex, \
                    person_id.date_of_birth AS date_of_birth, \
                    person_id.place_of_birth AS place_of_birth, \
                    person_id.date_of_death AS date_of_death, \
                    person_id.place_of_death AS place_of_death \
                 FROM person_proxy \
                 WHERE id = $id \
                 LIMIT 1",
            )
            .bind(("id", proxy_rid))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        match proxy {
            None => Err(rmcp::ErrorData::invalid_params(
                format!("person_proxy '{}' not found", p.person_proxy_id),
                None,
            )),
            Some(v) => Ok(to_safe_json(v)),
        }
    }

    /// Get the immediate family of a person: parents, spouse(s), and children, plus
    /// siblings when `include_siblings` is true.
    ///
    /// Returns separate lists for fathers, mothers, spouses, and children. Each entry
    /// includes `person_proxy_id`, `first_name`, and `family_name`. When
    /// `include_siblings` is set, a `siblings` list is also included (persons who
    /// share at least one parent with this person).
    #[tool(
        description = "Get immediate family (parents, spouses, children) of a person. \
                       Set include_siblings to true to also return siblings (persons \
                       sharing at least one parent)."
    )]
    async fn get_family(
        &self,
        Parameters(p): Parameters<GetFamilyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let proxy_rid = parse_proxy_id(&p.person_proxy_id)?;
        let db = self.db.lock().await;

        let fathers: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>out AS person_proxy_id, \
                    out.preferred_first_name ?? out.person_id.first_name AS first_name, \
                    out.preferred_family_name ?? out.person_id.family_name AS family_name, \
                    pedigree \
                 FROM has_father WHERE in = $id",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let mothers: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>out AS person_proxy_id, \
                    out.preferred_first_name ?? out.person_id.first_name AS first_name, \
                    out.preferred_family_name ?? out.person_id.family_name AS family_name, \
                    pedigree \
                 FROM has_mother WHERE in = $id",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let spouses: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>out AS person_proxy_id, \
                    out.preferred_first_name ?? out.person_id.first_name AS first_name, \
                    out.preferred_family_name ?? out.person_id.family_name AS family_name, \
                    spouse_from, spouse_to \
                 FROM has_spouse WHERE in = $id",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        // Children are persons for which this proxy is the has_father or has_mother out end.
        let children_via_father: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>in AS person_proxy_id, \
                    in.preferred_first_name ?? in.person_id.first_name AS first_name, \
                    in.preferred_family_name ?? in.person_id.family_name AS family_name \
                 FROM has_father WHERE out = $id",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let children_via_mother: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>in AS person_proxy_id, \
                    in.preferred_first_name ?? in.person_id.first_name AS first_name, \
                    in.preferred_family_name ?? in.person_id.family_name AS family_name \
                 FROM has_mother WHERE out = $id",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        // Merge children from both edges, dedup by proxy ID.
        let mut children: Vec<serde_json::Value> = children_via_father;
        let seen: std::collections::HashSet<String> = children
            .iter()
            .filter_map(|c| c.get("person_proxy_id").and_then(|v| v.as_str()).map(str::to_owned))
            .collect();
        for c in children_via_mother {
            let pid = c.get("person_proxy_id").and_then(|v| v.as_str()).unwrap_or("");
            if !seen.contains(pid) {
                children.push(c);
            }
        }

        let mut result = serde_json::json!({
            "fathers": fathers,
            "mothers": mothers,
            "spouses": spouses,
            "children": children,
        });

        if p.include_siblings {
            let mut siblings: Vec<serde_json::Value> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

            for parent in fathers.iter().chain(mothers.iter()) {
                let Some(parent_id) = parent.get("person_proxy_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let parent_rid = parse_proxy_id(parent_id)?;
                let is_father = fathers.iter().any(|f| f.get("person_proxy_id").and_then(|v| v.as_str()) == Some(parent_id));
                let edge = if is_father { "has_father" } else { "has_mother" };

                let rows: Vec<serde_json::Value> = db
                    .query(format!(
                        "SELECT \
                            <string>in AS person_proxy_id, \
                            in.preferred_first_name ?? in.person_id.first_name AS first_name, \
                            in.preferred_family_name ?? in.person_id.family_name AS family_name \
                         FROM {edge} WHERE out = $parent_id AND in != $id"
                    ))
                    .bind(("parent_id", parent_rid))
                    .bind(("id", proxy_rid.clone()))
                    .await
                    .map_err(db_err)?
                    .take(0)
                    .map_err(db_err)?;

                for row in rows {
                    let pid = row.get("person_proxy_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    if !pid.is_empty() && seen.insert(pid) {
                        siblings.push(row);
                    }
                }
            }

            result["siblings"] = serde_json::Value::Array(siblings);
        }

        Ok(to_safe_json(serde_json::to_value(&result).unwrap()))
    }

    /// Find potential duplicate persons across all family trees.
    ///
    /// Matches by exact name, then scores by sex, birth year, and place of birth.
    /// Owner identity is never included — use `create_contact_request` with the
    /// returned `proxy_id` values to reach out to the owner of a match.
    ///
    /// `is_own: true` means the match is in one of your own trees — no contact
    /// request is needed; you can merge directly in the app.
    #[tool(
        description = "Find potential duplicate persons for a given person proxy. \
                       Returns scored candidates. Owner identity is never revealed — \
                       use the proxy_id with create_contact_request to initiate contact."
    )]
    async fn find_duplicates(
        &self,
        Parameters(p): Parameters<FindDuplicatesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let proxy_rid = parse_proxy_id(&p.person_proxy_id)?;
        let db = self.db.lock().await;

        // Fetch the source proxy and its canonical person.
        let proxy: Option<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>person_id AS canonical_id, \
                    created_by, \
                    person_id.first_name AS first_name, \
                    person_id.family_name AS family_name, \
                    person_id.sex AS sex, \
                    person_id.date_of_birth AS dob, \
                    person_id.place_of_birth AS pob \
                 FROM person_proxy WHERE id = $id LIMIT 1",
            )
            .bind(("id", proxy_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let proxy = proxy.ok_or_else(|| {
            rmcp::ErrorData::invalid_params(
                format!("person_proxy '{}' not found", p.person_proxy_id),
                None,
            )
        })?;

        let canonical_id = proxy.get("canonical_id").and_then(|v| v.as_str()).unwrap_or("");
        let first_name = proxy.get("first_name").and_then(|v| v.as_str()).unwrap_or("");
        let family_name = proxy.get("family_name").and_then(|v| v.as_str()).unwrap_or("");
        let my_sex = proxy.get("sex").and_then(|v| v.as_str()).unwrap_or("");
        let my_dob = proxy.get("dob").and_then(|v| v.as_str());
        let my_pob = proxy.get("pob").and_then(|v| v.as_str());

        if first_name.is_empty() || family_name.is_empty() {
            return Ok(to_safe_json(serde_json::json!({ "count": 0, "matches": [] })));
        }

        // Find all proxies with the same name pointing to a different canonical person.
        let rows: Vec<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>id AS proxy_id, \
                    created_by, \
                    person_id.sex AS sex, \
                    person_id.date_of_birth AS dob, \
                    person_id.place_of_birth AS pob \
                 FROM person_proxy \
                 WHERE string::lowercase(person_id.family_name) = string::lowercase($family_name) \
                   AND string::lowercase(person_id.first_name) = string::lowercase($first_name) \
                   AND <string>person_id != $canonical_id \
                   AND id != $self_id",
            )
            .bind(("family_name", family_name))
            .bind(("first_name", first_name))
            .bind(("canonical_id", canonical_id))
            .bind(("self_id", proxy_rid))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let my_dob_year = my_dob.and_then(mcp_extract_year);
        let my_pob_lc = my_pob.map(|s| s.to_lowercase());

        let mut matches: Vec<serde_json::Value> = rows
            .into_iter()
            .filter_map(|row| {
                let proxy_id = row.get("proxy_id").and_then(|v| v.as_str())?.to_string();
                let owner = row.get("created_by").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let cand_sex = row.get("sex").and_then(|v| v.as_str()).map(str::to_string);
                let cand_dob = row.get("dob").and_then(|v| v.as_str()).map(str::to_string);
                let cand_pob = row.get("pob").and_then(|v| v.as_str()).map(str::to_string);

                // Sex mismatch is a hard disqualifier.
                let mut score: i32 = 0;
                if let Some(cs) = cand_sex.as_deref() {
                    if !my_sex.is_empty() {
                        if cs == my_sex {
                            score += 3;
                        } else {
                            return None;
                        }
                    }
                }

                // Birth year: fuzzy match.
                if let (Some(my_y), Some(cand_str)) = (my_dob_year, cand_dob.as_deref()) {
                    if let Some(cy) = mcp_extract_year(cand_str) {
                        if my_y == cy {
                            score += 2;
                        } else if my_y.abs_diff(cy) == 1 {
                            score += 1;
                        }
                    }
                }

                // Place of birth: substring or shared-word containment.
                if let (Some(ref my_p), Some(cand_str)) = (&my_pob_lc, cand_pob.as_deref()) {
                    let cp = cand_str.to_lowercase();
                    if *my_p == cp {
                        score += 3;
                    } else if my_p.contains(&cp) || cp.contains(my_p.as_str()) {
                        score += 2;
                    } else if mcp_shares_significant_word(my_p, &cp) {
                        score += 1;
                    }
                }

                let score = score as u32;
                let confidence = match score {
                    s if s >= 4 => "strong",
                    s if s >= 2 => "likely",
                    _ => "possible",
                };

                let is_own = owner == self.username;

                // Owner is used for is_own only — never returned to the caller.
                Some(serde_json::json!({
                    "proxy_id": proxy_id,
                    "family_name": family_name,
                    "first_name": first_name,
                    "sex": cand_sex,
                    "date_of_birth": cand_dob,
                    "place_of_birth": cand_pob,
                    "score": score,
                    "confidence": confidence,
                    "is_own": is_own,
                }))
            })
            .collect();

        matches.sort_by(|a, b| {
            b.get("score").and_then(|v| v.as_u64()).unwrap_or(0)
                .cmp(&a.get("score").and_then(|v| v.as_u64()).unwrap_or(0))
        });

        let result = serde_json::json!({
            "count": matches.len(),
            "matches": matches,
        });

        Ok(to_safe_json(serde_json::to_value(&result).unwrap()))
    }

    /// List your contact requests (sent and/or received).
    ///
    /// The other party's identity is masked while a request is `pending` or `ignored`.
    /// Once a request is `accepted`, both parties can see each other's identity.
    #[tool(
        description = "List your contact requests with other tree owners. \
                       Identity of the other party is hidden until the request is accepted."
    )]
    async fn list_contact_requests(
        &self,
        Parameters(p): Parameters<ListContactRequestsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let db = self.db.lock().await;

        let mut query = match p.role.as_deref() {
            Some("sent") => {
                db.query(
                    "SELECT <string>id AS id, from_user, status, \
                            initial_message, created_at, updated_at \
                     FROM merge_contact_request \
                     WHERE from_user = $user \
                     ORDER BY created_at DESC",
                )
                .bind(("user", self.username.clone()))
                .await
                .map_err(db_err)?
            }
            Some("received") => {
                db.query(
                    "SELECT <string>id AS id, to_user, status, \
                            initial_message, created_at, updated_at \
                     FROM merge_contact_request \
                     WHERE to_user = $user \
                     ORDER BY created_at DESC",
                )
                .bind(("user", self.username.clone()))
                .await
                .map_err(db_err)?
            }
            _ => {
                db.query(
                    "SELECT <string>id AS id, from_user, to_user, status, \
                            initial_message, created_at, updated_at \
                     FROM merge_contact_request \
                     WHERE from_user = $user OR to_user = $user \
                     ORDER BY created_at DESC",
                )
                .bind(("user", self.username.clone()))
                .await
                .map_err(db_err)?
            }
        };

        let rows: Vec<serde_json::Value> = query.take(0).map_err(db_err)?;

        // Replace from_user/to_user with a direction indicator — usernames are never returned.
        let filtered: Vec<serde_json::Value> = rows
            .into_iter()
            .map(|row| {
                let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
                let from = row.get("from_user").and_then(|v| v.as_str()).unwrap_or("");
                let direction = if from == self.username { "sent" } else { "received" };

                serde_json::json!({
                    "id": row.get("id"),
                    "direction": direction,
                    "status": status,
                    "initial_message": row.get("initial_message"),
                    "created_at": row.get("created_at"),
                    "updated_at": row.get("updated_at"),
                })
            })
            .collect();

        Ok(to_safe_json(serde_json::to_value(&filtered).unwrap()))
    }

    /// Send a contact request to the owner(s) of matched persons in other trees.
    ///
    /// Pass the `proxy_id` values from `find_duplicates` as `target_proxy_ids`.
    /// The server looks up the owner of each proxy internally — their username is
    /// never exposed to this tool. Claude will suggest a message based on the
    /// genealogical context; the user should confirm or edit it before calling this tool.
    ///
    /// Skips: self-contact, already-pending requests for the same proxy + recipient.
    /// Only pass proxies where `is_own` is false.
    #[tool(
        description = "Send a contact request to the owner(s) of matched persons from find_duplicates. \
                       Pass target_proxy_ids from find_duplicates results. \
                       ALWAYS show the user the message and get confirmation before calling this tool."
    )]
    async fn create_contact_request(
        &self,
        Parameters(p): Parameters<CreateContactRequestParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let from_rid = parse_proxy_id(&p.from_proxy_id)?;
        let db = self.db.lock().await;

        // Look up the caller's proxy to get their username.
        let from_proxy: Option<serde_json::Value> = db
            .query(
                "SELECT created_by FROM person_proxy WHERE id = $id LIMIT 1",
            )
            .bind(("id", from_rid.clone()))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        let from_proxy = from_proxy.ok_or_else(|| {
            rmcp::ErrorData::invalid_params(
                format!("from_proxy_id '{}' not found", p.from_proxy_id),
                None,
            )
        })?;
        let caller = from_proxy
            .get("created_by")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut sent = 0u32;
        let mut skipped = 0u32;

        for target_proxy_id_str in &p.target_proxy_ids {
            let target_rid = parse_proxy_id(target_proxy_id_str)?;

            // Look up the owner of the target proxy — never returned to the caller.
            let target_proxy: Option<serde_json::Value> = db
                .query(
                    "SELECT created_by FROM person_proxy WHERE id = $id LIMIT 1",
                )
                .bind(("id", target_rid.clone()))
                .await
                .map_err(db_err)?
                .take(0)
                .map_err(db_err)?;

            let target_owner = match target_proxy
                .as_ref()
                .and_then(|v| v.get("created_by"))
                .and_then(|v| v.as_str())
            {
                Some(o) => o.to_string(),
                None => {
                    skipped += 1;
                    continue;
                }
            };

            // Skip self-contact.
            if target_owner == caller {
                skipped += 1;
                continue;
            }

            // Skip if there is already a pending request for this proxy + recipient.
            let existing: Option<serde_json::Value> = db
                .query(
                    "SELECT id FROM merge_contact_request \
                     WHERE status = 'pending' \
                       AND from_proxy_id = $proxy \
                       AND to_user = $to \
                     LIMIT 1",
                )
                .bind(("proxy", from_rid.clone()))
                .bind(("to", target_owner.clone()))
                .await
                .map_err(db_err)?
                .take(0)
                .map_err(db_err)?;

            if existing.is_some() {
                skipped += 1;
                continue;
            }

            db.query(
                "CREATE merge_contact_request SET \
                 from_proxy_id = $proxy, \
                 from_user = $from, \
                 to_user = $to, \
                 initial_message = $msg, \
                 status = 'pending', \
                 messages = [], \
                 created_at = time::now(), \
                 updated_at = time::now()",
            )
            .bind(("proxy", from_rid.clone()))
            .bind(("from", caller.clone()))
            .bind(("to", target_owner))
            .bind(("msg", p.message.clone()))
            .await
            .map_err(db_err)?;

            sent += 1;
        }

        let result = serde_json::json!({
            "sent": sent,
            "skipped": skipped,
            "message": p.message,
        });

        Ok(to_safe_json(serde_json::to_value(&result).unwrap()))
    }
}

#[tool_handler]
impl rmcp::ServerHandler for ClannServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions(
            "Clann genealogy MCP server. \
             Use these tools to explore family trees, find persons, navigate \
             relationships, discover potential duplicates, and send contact \
             requests to other tree owners. \
             Privacy rule: never reveal another user's identity before a contact \
             request is accepted. Always show the user proposed messages and get \
             explicit confirmation before calling create_contact_request.",
        )
    }
}

// ── Service factory ───────────────────────────────────────────────────────────

pub fn make_mcp_service(db: Db, canonical_host: String) -> StreamableHttpService<ClannServer, LocalSessionManager> {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["localhost", "127.0.0.1", "::1", canonical_host.as_str()]);
    StreamableHttpService::new(
        move || {
            let username = AUTHENTICATED_USERNAME
                .try_with(|u| u.clone())
                .unwrap_or_default();
            Ok(ClannServer::new(db.clone(), username))
        },
        session_manager,
        config,
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a person proxy ID that may be given as `person_proxy:<id>` or just `<id>`.
fn parse_proxy_id(s: &str) -> Result<RecordId, rmcp::ErrorData> {
    let key = s.strip_prefix("person_proxy:").unwrap_or(s);
    if key.is_empty() {
        return Err(rmcp::ErrorData::invalid_params("person_proxy_id must not be empty", None));
    }
    Ok(RecordId::new("person_proxy", key))
}

fn db_err(e: impl std::fmt::Display) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(format!("database error: {e}"), None)
}

/// Fields that must never appear in any MCP response.
///
/// Applied as a recursive scrub to every tool output as a defence-in-depth
/// safety net. The SELECT queries already omit these fields, but this catches
/// any future query additions or accidental inclusions.
const SENSITIVE_FIELDS: &[&str] = &[
    "username", "email", "owner", "from_user", "to_user", "created_by",
];

/// Recursively remove all sensitive fields from a JSON value.
pub(crate) fn scrub_pii(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for key in SENSITIVE_FIELDS {
                map.remove(*key);
            }
            for v in map.values_mut() {
                scrub_pii(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                scrub_pii(v);
            }
        }
        _ => {}
    }
}

/// Serialise a JSON value to a pretty string after scrubbing PII.
/// Serializes a scrubbed (PII-safe) value to a tool result carrying both
/// pretty-printed text (for chat/LLM callers, unchanged from before) and the
/// same *scrubbed* value as `structured_content` — critically, scrubbing
/// happens once here and both representations are built from the result, so
/// structured_content can never leak PII the text output redacts.
fn to_safe_json(mut value: serde_json::Value) -> CallToolResult {
    scrub_pii(&mut value);
    let text = serde_json::to_string_pretty(&value).unwrap();
    super::text_result(text, value)
}

/// Extract a 4-digit year from a free-form date string.
fn mcp_extract_year(s: &str) -> Option<u32> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
            if let Ok(y) = s[i..i + 4].parse::<u32>() {
                if (1700..=2099).contains(&y) {
                    return Some(y);
                }
            }
        }
    }
    None
}

/// True when two lowercased place strings share at least one word longer than 3 chars.
fn mcp_shares_significant_word(a: &str, b: &str) -> bool {
    let words_b: std::collections::HashSet<&str> = b
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.len() > 3)
        .collect();
    a.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.len() > 3)
        .any(|w| words_b.contains(w))
}
