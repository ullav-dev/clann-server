pub struct Config {
    pub db_url: String,
    pub db_namespace: String,
    pub db_database: String,
    pub db_username: String,
    pub db_password: String,
    pub server_port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            db_url: std::env::var("DB_URL").unwrap_or_else(|_| "ws://localhost:8000".to_string()),
            db_namespace: std::env::var("DB_NAMESPACE").unwrap_or_else(|_| "clann".to_string()),
            db_database: std::env::var("DB_DATABASE").unwrap_or_else(|_| "ancestry".to_string()),
            db_username: std::env::var("DB_USERNAME").unwrap_or_else(|_| "root".to_string()),
            db_password: std::env::var("DB_PASSWORD").unwrap_or_else(|_| "secret".to_string()),
            server_port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(3000),
        }
    }
}
