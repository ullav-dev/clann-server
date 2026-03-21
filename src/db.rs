use std::sync::Arc;

use tokio::sync::Mutex;
use surrealdb::{
    engine::any::{self, Any},
    opt::auth::Root,
    Surreal,
};

use crate::config::Config;

/// The inner SurrealDB connection type.
pub type DbConn = Surreal<Any>;

/// The shared database state: a mutex-wrapped connection.
///
/// `Surreal<Any>` over WebSocket maintains server-side session state
/// (namespace / database).  Concurrent queries on the same connection can
/// interleave and corrupt that state, producing "Connection uninitialised" or
/// "Specify a namespace to use" errors.  Wrapping in a `Mutex` serialises all
/// database access, which is perfectly adequate for this application's load.
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

    // Backfill any records created before the `verified` field was introduced.
    db.query("UPDATE person SET verified = false WHERE verified = NONE").await?;

    tracing::info!("Connected to SurrealDB at {}", config.db_url);

    Ok(Arc::new(Mutex::new(db)))
}
