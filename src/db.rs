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

    // Backfill: migrate single `tree` string to `trees` array for existing records.
    db.query("UPDATE person SET trees = [tree] WHERE tree != NONE AND (trees = NONE OR trees = [])").await?;

    // Seed life events from existing person birth/death/marriage data.
    // Each query only creates an event if one doesn't already exist for that person+type.
    db.query(
        "FOR $p IN (SELECT * FROM person WHERE date_of_birth != NONE OR place_of_birth != NONE) {
            LET $pid_str = <string>$p.id;
            IF (SELECT count() FROM life_event WHERE person_id = $pid_str AND event_type = 'Birth' GROUP ALL)[0].count = 0 {
                CREATE life_event SET
                    person_id   = $pid_str,
                    name        = 'Birth',
                    date        = $p.date_of_birth,
                    event_type  = 'Birth',
                    description = $p.place_of_birth,
                    verified    = false,
                    created_by  = $p.created_by;
            };
        };"
    ).await?;

    db.query(
        "FOR $p IN (SELECT * FROM person WHERE date_of_death != NONE OR place_of_death != NONE) {
            LET $pid_str = <string>$p.id;
            IF (SELECT count() FROM life_event WHERE person_id = $pid_str AND event_type = 'Death' GROUP ALL)[0].count = 0 {
                CREATE life_event SET
                    person_id   = $pid_str,
                    name        = 'Death',
                    date        = $p.date_of_death,
                    event_type  = 'Death',
                    description = $p.place_of_death,
                    verified    = false,
                    created_by  = $p.created_by;
            };
        };"
    ).await?;

    // Seed Marriage events from has_spouse edges (one event per unique pair).
    db.query(
        "FOR $e IN (SELECT *, <string>in AS person_a, out AS person_b FROM has_spouse WHERE in < out) {
            LET $name_b = (SELECT string::concat(first_name, ' ', family_name) AS full FROM person WHERE id = $e.person_b LIMIT 1)[0].full;
            IF (SELECT count() FROM life_event WHERE person_id = $e.person_a AND event_type = 'Marriage' AND name = string::concat('Marriage to ', $name_b ?? '') GROUP ALL)[0].count = 0 {
                CREATE life_event SET
                    person_id   = $e.person_a,
                    name        = string::concat('Marriage to ', $name_b ?? 'unknown'),
                    date        = $e.spouse_from,
                    event_type  = 'Marriage',
                    verified    = false,
                    created_by  = (SELECT created_by FROM person WHERE id = $e.person_b LIMIT 1)[0].created_by;
            };
        };"
    ).await?;

    tracing::info!("Connected to SurrealDB at {}", config.db_url);

    Ok(Arc::new(Mutex::new(db)))
}
