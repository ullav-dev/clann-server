pub struct Config {
    pub db_url: String,
    pub db_namespace: String,
    pub db_database: String,
    pub db_username: String,
    pub db_password: String,
    pub server_port: u16,
    pub upload_dir: String,
    /// Path to the SurrealDB data file (used when starting SurrealDB with file-backed storage).
    pub db_path: String,
    pub enable_docs: bool,
    /// JWKS URI on UUM — used by the RS256 token validator.
    pub oauth2_jwks_url: String,
    /// OAuth2 issuer URL (UUM) — validated in RS256 tokens.
    pub oauth2_issuer: String,
    /// RFC 9728 canonical URI for this MCP resource server — used as the OAuth2 audience.
    pub mcp_canonical_uri: String,
    /// Base URL of tack-server's own API -- the Phase 3 notes/folders
    /// handlers proxy through here via `tack_client.rs` instead of
    /// SurrealDB directly (see that module's own doc comment).
    pub tack_api_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            db_url: std::env::var("DB_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string()),
            db_namespace: std::env::var("DB_NAMESPACE").unwrap_or_else(|_| "clann".to_string()),
            db_database: std::env::var("DB_DATABASE").unwrap_or_else(|_| "ancestry".to_string()),
            db_username: env_or_file("DB_USERNAME", "root"),
            db_password: env_or_file("DB_PASSWORD", "secret"),
            server_port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "./uploads".to_string()),
            db_path: std::env::var("DB_PATH")
                .unwrap_or_else(|_| "/opt/ullav/clann/data.db".to_string()),
            enable_docs: std::env::var("ENABLE_DOCS")
                .unwrap_or_else(|_| "true".into())
                .parse()
                .unwrap_or(true),
            oauth2_jwks_url: std::env::var("OAUTH2_JWKS_URL")
                .unwrap_or_else(|_| "http://localhost:8081/oauth2/jwks".into()),
            oauth2_issuer: std::env::var("OAUTH2_ISSUER")
                .unwrap_or_else(|_| "http://localhost:8081".into()),
            mcp_canonical_uri: std::env::var("CLANN_MCP_CANONICAL_URI")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            tack_api_url: std::env::var("TACK_API_URL")
                .unwrap_or_else(|_| "http://localhost:8087".into()),
        }
    }
}

/// Reads a config value from `KEY_FILE` (Docker secrets pattern) if set,
/// otherwise falls back to `KEY`, then to `default`.
fn env_or_file(key: &str, default: &str) -> String {
    let file_key = format!("{}_FILE", key);
    if let Ok(path) = std::env::var(&file_key) {
        return std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read secret file {path}: {e}"))
            .trim()
            .to_string();
    }
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
