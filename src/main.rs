use clann_server::{config, db, routes};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use ullav_mcp_auth::TokenValidator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "clann_server=debug,tower_http=debug".into()
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::from_env();
    let db = db::connect(&config).await?;
    tokio::fs::create_dir_all(&config.upload_dir).await?;

    let validator = TokenValidator::new(
        config.oauth2_jwks_url.clone(),
        config.oauth2_issuer.clone(),
        String::new(), // no audience check for general API tokens
    );

    let mcp_config = routes::McpConfig {
        canonical_uri: config.mcp_canonical_uri.clone(),
        authorization_server: config.oauth2_issuer.clone(),
        jwks_url: config.oauth2_jwks_url.clone(),
    };

    let router = routes::build_router(
        db,
        config.upload_dir.clone(),
        config.enable_docs,
        Some(validator),
        Some(mcp_config),
    );

    let addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", addr);

    axum::serve(listener, router).await?;

    Ok(())
}
