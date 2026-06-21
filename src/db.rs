use std::sync::Arc;

use tokio::sync::Mutex;
use surrealdb::{
    engine::any::{self, Any},
    opt::auth::Root,
    Surreal,
};

use crate::config::Config;

pub type DbConn = Surreal<Any>;

/// Mutex-wrapped connection — see db module docs for rationale.
pub type Db = Arc<Mutex<DbConn>>;

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

    // Backfill legacy person records that predate the verified/trees fields.
    db.query("UPDATE person SET verified = false WHERE verified = NONE").await?;
    db.query("UPDATE person SET trees = [tree] WHERE tree != NONE AND (trees = NONE OR trees = [])").await?;

    // Pedigree refactor: remove foster/step parent edges, convert step siblings to half.
    // These are idempotent — safe to run on every startup.
    db.query("DELETE has_father WHERE pedigree = 'foster'").await?;
    db.query("DELETE has_father WHERE pedigree = 'step'").await?;
    db.query("DELETE has_mother WHERE pedigree = 'foster'").await?;
    db.query("DELETE has_mother WHERE pedigree = 'step'").await?;
    db.query("UPDATE has_sibling SET pedigree = 'half' WHERE pedigree = 'step'").await?;
    db.query("DELETE has_sibling WHERE pedigree = 'foster'").await?;

    tracing::info!("Connected to SurrealDB at {}", config.db_url);
    Ok(Arc::new(Mutex::new(db)))
}
