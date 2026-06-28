use std::sync::Arc;

use tokio::sync::Mutex;
use surrealdb::{
    engine::any::{self, Any},
    opt::auth::Root,
    types::{RecordId, RecordIdKey},
    Surreal,
};

use crate::config::Config;
use crate::models::life_event::LifeEvent;
use crate::models::person::{Person, PersonProxy};

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

    seed_life_events(&db).await?;

    tracing::info!("Connected to SurrealDB at {}", config.db_url);
    Ok(Arc::new(Mutex::new(db)))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn record_key(id: &RecordId) -> String {
    match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    }
}

/// Returns true if a life_event with the given proxy ID and event_type already exists.
async fn life_event_exists(db: &DbConn, proxy_rid: RecordId, event_type: &str) -> anyhow::Result<bool> {
    let result: Option<serde_json::Value> = db
        .query("SELECT count() AS n FROM life_event WHERE person_proxy_id = $pid AND event_type = $et GROUP ALL")
        .bind(("pid", proxy_rid))
        .bind(("et", event_type.to_string()))
        .await?
        .take(0)?;
    Ok(result
        .and_then(|v| v.get("n").and_then(|x| x.as_u64()))
        .unwrap_or(0) > 0)
}

// ── seed ──────────────────────────────────────────────────────────────────────

async fn seed_life_events(db: &DbConn) -> anyhow::Result<()> {
    // Fetch proxies for canonical persons that have date/place data worth seeding.
    let proxies: Vec<PersonProxy> = db
        .query(
            "SELECT * FROM person_proxy \
             WHERE person_id.date_of_birth != NONE OR person_id.place_of_birth != NONE \
                OR person_id.date_of_death  != NONE OR person_id.place_of_death != NONE",
        )
        .await?
        .take(0)?;

    let mut seeded = 0usize;

    for proxy in &proxies {
        let canonical: Option<Person> = db.select(proxy.person_id.clone()).await?;
        let Some(p) = canonical else { continue };
        let proxy_rid = RecordId::new("person_proxy", record_key(&proxy.id).as_str());

        if p.date_of_birth.is_some() || p.place_of_birth.is_some() {
            if !life_event_exists(db, proxy_rid.clone(), "Birth").await? {
                let _: Vec<LifeEvent> = db
                    .query(
                        "CREATE life_event SET \
                         person_proxy_id = $pid, name = 'Birth', date = $date, \
                         event_type = 'Birth', description = $desc, \
                         is_canonical = true, contributed_by_tree = $tree, \
                         verified = false, created_by = $creator",
                    )
                    .bind(("pid",     proxy_rid.clone()))
                    .bind(("date",    p.date_of_birth.clone()))
                    .bind(("desc",    p.place_of_birth.clone()))
                    .bind(("tree",    proxy.tree.clone()))
                    .bind(("creator", proxy.created_by.clone()))
                    .await?
                    .take(0)?;
                seeded += 1;
            }
        }

        if p.date_of_death.is_some() || p.place_of_death.is_some() {
            if !life_event_exists(db, proxy_rid.clone(), "Death").await? {
                let _: Vec<LifeEvent> = db
                    .query(
                        "CREATE life_event SET \
                         person_proxy_id = $pid, name = 'Death', date = $date, \
                         event_type = 'Death', description = $desc, \
                         is_canonical = true, contributed_by_tree = $tree, \
                         verified = false, created_by = $creator",
                    )
                    .bind(("pid",     proxy_rid.clone()))
                    .bind(("date",    p.date_of_death.clone()))
                    .bind(("desc",    p.place_of_death.clone()))
                    .bind(("tree",    proxy.tree.clone()))
                    .bind(("creator", proxy.created_by.clone()))
                    .await?
                    .take(0)?;
                seeded += 1;
            }
        }
    }

    // Marriage events: spouse edges are now between proxies.
    // Traverse out.person_id.* for the partner's canonical name.
    let spouse_pairs: Vec<serde_json::Value> = db
        .query(
            "SELECT \
               <string>in  AS proxy_a, \
               out.person_id.first_name  AS partner_first, \
               out.person_id.family_name AS partner_family, \
               spouse_from, \
               in.created_by AS created_by, \
               in.tree AS tree \
             FROM has_spouse WHERE in < out",
        )
        .await?
        .take(0)?;

    for row in &spouse_pairs {
        let proxy_a_str = row.get("proxy_a").and_then(|v| v.as_str()).unwrap_or("");
        let partner_first  = row.get("partner_first").and_then(|v| v.as_str()).unwrap_or("");
        let partner_family = row.get("partner_family").and_then(|v| v.as_str()).unwrap_or("");
        let spouse_from    = row.get("spouse_from").and_then(|v| v.as_str()).map(String::from);
        let created_by     = row.get("created_by").and_then(|v| v.as_str()).map(String::from);
        let tree           = row.get("tree").and_then(|v| v.as_str()).map(String::from);

        if proxy_a_str.is_empty() { continue; }
        let key_a = proxy_a_str.strip_prefix("person_proxy:").unwrap_or(proxy_a_str);
        let pid_a = RecordId::new("person_proxy", key_a);

        if !life_event_exists(db, pid_a.clone(), "Marriage").await? {
            let partner_name = format!("{} {}", partner_first, partner_family).trim().to_string();
            let event_name = if partner_name.is_empty() {
                "Marriage".to_string()
            } else {
                format!("Marriage to {}", partner_name)
            };

            let _: Vec<LifeEvent> = db
                .query(
                    "CREATE life_event SET \
                     person_proxy_id = $pid, name = $name, date = $date, \
                     event_type = 'Marriage', is_canonical = true, \
                     contributed_by_tree = $tree, \
                     verified = false, created_by = $creator",
                )
                .bind(("pid",     pid_a))
                .bind(("name",    event_name))
                .bind(("date",    spouse_from))
                .bind(("tree",    tree))
                .bind(("creator", created_by))
                .await?
                .take(0)?;
            seeded += 1;
        }
    }

    if seeded > 0 {
        tracing::info!("Seeded {} life event(s) from existing person/relationship data", seeded);
    }

    Ok(())
}
