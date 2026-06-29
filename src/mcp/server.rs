/// Clann MCP server — read-only genealogy tools over Streamable HTTP.
///
/// Auth is validated by the Axum MCP middleware before requests reach this
/// service, so tools trust the caller is authenticated and operate without
/// re-validating the token.

use std::sync::Arc;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{ServerCapabilities, ServerInfo},
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

// ── Parameter types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTreesParams {
    /// Username of the tree owner (e.g. the currently authenticated user's username).
    pub username: String,
}

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
}

// ── Server ────────────────────────────────────────────────────────────────────

pub struct ClannServer {
    db: Db,
}

impl ClannServer {
    fn new(db: Db) -> Self {
        Self { db }
    }
}

#[tool_router]
impl ClannServer {
    /// List family trees owned by a given user.
    #[tool(description = "List family trees owned by a given user")]
    async fn list_trees(
        &self,
        Parameters(p): Parameters<ListTreesParams>,
    ) -> Result<String, rmcp::ErrorData> {
        let db = self.db.lock().await;
        let rows: Vec<serde_json::Value> = db
            .query("SELECT <string>id AS id, name, display_name, owner, is_primary, team_id FROM family_tree WHERE owner = $username")
            .bind(("username", p.username))
            .await
            .map_err(db_err)?
            .take(0)
            .map_err(db_err)?;

        Ok(serde_json::to_string_pretty(&rows).unwrap())
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
    ) -> Result<String, rmcp::ErrorData> {
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

        Ok(serde_json::to_string_pretty(&matches).unwrap())
    }

    /// Get full details of a person by their proxy ID (includes canonical birth/death data).
    #[tool(description = "Get full details of a person by their proxy record ID")]
    async fn get_person(
        &self,
        Parameters(p): Parameters<GetPersonParams>,
    ) -> Result<String, rmcp::ErrorData> {
        let proxy_rid = parse_proxy_id(&p.person_proxy_id)?;
        let db = self.db.lock().await;

        let proxy: Option<serde_json::Value> = db
            .query(
                "SELECT \
                    <string>id AS person_proxy_id, \
                    <string>person_id AS canonical_person_id, \
                    tree, \
                    preferred_first_name, preferred_family_name, preferred_middle_name, \
                    nickname, biography, username, email, is_private, verified, \
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
            Some(v) => Ok(serde_json::to_string_pretty(&v).unwrap()),
        }
    }

    /// Get the immediate family of a person: parents, siblings, spouse(s), and children.
    ///
    /// Returns separate lists for fathers, mothers, spouses, and children. Each entry
    /// includes `person_proxy_id`, `first_name`, and `family_name`.
    #[tool(
        description = "Get immediate family (parents, siblings, spouses, children) of a person"
    )]
    async fn get_family(
        &self,
        Parameters(p): Parameters<GetFamilyParams>,
    ) -> Result<String, rmcp::ErrorData> {
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

        let result = serde_json::json!({
            "fathers": fathers,
            "mothers": mothers,
            "spouses": spouses,
            "children": children,
        });

        Ok(serde_json::to_string_pretty(&result).unwrap())
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
            "Clann MCP server — read-only genealogy tools. \
             Use these tools to explore family trees, search for persons, \
             and navigate family relationships.",
        )
    }
}

// ── Service factory ───────────────────────────────────────────────────────────

pub fn make_mcp_service(db: Db, canonical_host: String) -> StreamableHttpService<ClannServer, LocalSessionManager> {
    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["localhost", "127.0.0.1", "::1", canonical_host.as_str()]);
    StreamableHttpService::new(
        move || Ok(ClannServer::new(db.clone())),
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
