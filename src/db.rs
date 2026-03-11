use surrealdb::{
    engine::any::{self, Any},
    opt::auth::Root,
    Surreal,
};

use crate::config::Config;

pub type Db = Surreal<Any>;

pub async fn connect(config: &Config) -> anyhow::Result<Db> {
    let db = any::connect(&config.db_url).await?;

    db.signin(Root {
        username: config.db_username.clone(),
        password: config.db_password.clone(),
    })
    .await?;

    db.use_ns(&config.db_namespace)
        .use_db(&config.db_database)
        .await?;

    let schema = include_str!("../migrations/schema.surql");
    db.query(schema).await?;

    // Backfill any records created before the `verified` field was introduced.
    db.query("UPDATE person SET verified = false WHERE verified = NONE").await?;

    tracing::info!("Connected to SurrealDB at {}", config.db_url);

    Ok(db)
}
