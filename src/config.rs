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
