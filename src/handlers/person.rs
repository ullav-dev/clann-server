use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use surrealdb::types::RecordId;

use crate::{
    auth::ClannAuth,
    db::{Db, DbConn},
    error::{AppError, ErrorResponse},
    models::{
        family_tree::FamilyTree,
        person::{
            build_proxy_response, CreatePerson, Person, PersonProxy, PersonResponse,
            PersonProxyResponse, PersonProxyStub, TreeMembershipRequest, UpdateCanonicalPerson,
            UpdatePersonProxy,
        },
    },
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PersonFilter {
    pub created_by: Option<String>,
    pub tree: Option<String>,
}

fn is_admin(username: &str) -> bool {
    let admin = std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "theboss".to_string());
    username == admin
}

/// Returns `true` if `user_id` has an editor grant for the given tree.
pub async fn is_tree_editor(db: &DbConn, user_id: &str, trees: &[String]) -> Result<bool, AppError> {
    if trees.is_empty() || user_id.is_empty() {
        return Ok(false);
    }
    let grant: Option<serde_json::Value> = db
        .query("SELECT id FROM tree_editor WHERE user_id = $uid AND tree_name IN $trees LIMIT 1")
        .bind(("uid", user_id.to_string()))
        .bind(("trees", trees.to_vec()))
        .await?
        .take(0)?;
    Ok(grant.is_some())
}

/// Returns true if the caller is a member of `tree` (owner, team member, or editor).
/// Always true in dev mode (empty user_id).
pub async fn is_tree_member(tree: &str, auth: &ClannAuth, db: &DbConn) -> Result<bool, AppError> {
    if auth.user_id.is_empty() {
        return Ok(true);
    }

    let tree_rec: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", tree.to_string()))
        .await?
        .take(0)?;

    let Some(t) = tree_rec else { return Ok(false) };

    if t.owner == auth.username {
        return Ok(true);
    }

    if let Some(ref tid) = t.team_id {
        if auth.teams.contains_key(tid.as_str()) {
            return Ok(true);
        }
    }

    if is_tree_editor(db, &auth.user_id, &[tree.to_string()]).await? {
        return Ok(true);
    }

    Ok(false)
}

/// Returns `Ok(())` when the caller may write to the given proxy.
/// Same surface as read auth (NotFound) to prevent proxy enumeration.
pub async fn can_write_to_proxy(
    proxy: &PersonProxy,
    auth: &ClannAuth,
    db: &DbConn,
) -> Result<(), AppError> {
    if auth.user_id.is_empty() {
        return Ok(());
    }

    if !auth.username.is_empty() && is_admin(&auth.username) {
        return Ok(());
    }

    if !auth.username.is_empty()
        && proxy.created_by.as_deref() == Some(auth.username.as_str())
    {
        return Ok(());
    }

    let tree_rec: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", proxy.tree.clone()))
        .await?
        .take(0)?;

    if let Some(t) = tree_rec {
        if let Some(ref tid) = t.team_id {
            if auth.teams.get(tid.as_str()).map(|r| r == "owner").unwrap_or(false) {
                return Ok(());
            }
        }
    }

    if is_tree_editor(db, &auth.user_id, &[proxy.tree.clone()]).await? {
        return Ok(());
    }

    Err(AppError::NotFound)
}

/// Auth check for canonical-person-scoped write operations.
/// Passes for admins, for the person's creator, and (like can_write_to_proxy) when auth is absent.
pub async fn can_write_to_person(
    person: &Person,
    auth: &ClannAuth,
    _db: &DbConn,
) -> Result<(), AppError> {
    if auth.user_id.is_empty() {
        return Ok(());
    }
    if !auth.username.is_empty() && is_admin(&auth.username) {
        return Ok(());
    }
    if !auth.username.is_empty()
        && person.created_by.as_deref() == Some(auth.username.as_str())
    {
        return Ok(());
    }
    Err(AppError::Forbidden("You do not have write access to this person".to_string()))
}

/// Validate that a filter's `created_by` matches the person's creator.
/// Returns `Ok(())` if `created_by` is `None` (no filter) or matches.
pub fn check_ownership(person: &Person, created_by: &Option<String>) -> Result<(), AppError> {
    if let Some(ref filter_user) = created_by {
        if person.created_by.as_deref() != Some(filter_user.as_str()) {
            return Err(AppError::NotFound);
        }
    }
    Ok(())
}

/// Decide whether to return a full response or a privacy stub for a cross-tree proxy.
/// Tree members always get the full response. Cross-tree callers get a stub when `is_private = true`.
/// Non-members with no shared canonical get NotFound.
pub async fn redact_if_private(
    proxy: PersonProxy,
    canonical: Person,
    auth: &ClannAuth,
    db: &DbConn,
) -> Result<PersonResponse, AppError> {
    let member = is_tree_member(&proxy.tree, auth, db).await?;

    if member {
        return Ok(PersonResponse::Full(build_proxy_response(proxy, canonical)));
    }

    if !proxy.is_private {
        return Ok(PersonResponse::Full(build_proxy_response(proxy, canonical)));
    }

    // Private proxy — check for a shared canonical (caller has proxy in another tree for same person).
    let shared_canonical: Option<serde_json::Value> = db
        .query("SELECT count() AS n FROM person_proxy WHERE person_id = $pid GROUP ALL")
        .bind(("pid", proxy.person_id.clone()))
        .await?
        .take(0)?;
    let count = shared_canonical
        .and_then(|v| v.get("n").and_then(|x| x.as_u64()))
        .unwrap_or(0);

    if count <= 1 {
        // No shared canonical — caller has no business knowing this proxy exists.
        return Err(AppError::NotFound);
    }

    Ok(PersonResponse::Stub(PersonProxyStub {
        id: proxy.id,
        person_id: proxy.person_id,
        tree: proxy.tree,
        sex: canonical.sex,
        is_private: true,
    }))
}

/// Fetch a proxy record + its canonical, or return NotFound.
pub async fn fetch_proxy_and_canonical(
    db: &DbConn,
    proxy_ulid: &str,
) -> Result<(PersonProxy, Person), AppError> {
    let proxy: Option<PersonProxy> = db.select(("person_proxy", proxy_ulid)).await?;
    let proxy = proxy.ok_or(AppError::NotFound)?;
    let canonical: Option<Person> = db.select(proxy.person_id.clone()).await?;
    let canonical = canonical.ok_or(AppError::NotFound)?;
    Ok((proxy, canonical))
}

#[utoipa::path(
    post,
    path = "/api/persons",
    request_body = CreatePerson,
    responses(
        (status = 201, description = "Person created", body = PersonProxyResponse),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 403, description = "Member limit reached", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn create_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreatePerson>,
) -> Result<(StatusCode, Json<PersonProxyResponse>), AppError> {
    let db = db.lock().await;

    // Validate tree exists.
    let tree_exists: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", payload.tree.clone()))
        .await?
        .take(0)?;
    if tree_exists.is_none() {
        return Err(AppError::BadRequest(format!(
            "Family tree '{}' not found",
            payload.tree
        )));
    }

    // Enforce subscription member limit (counted on canonical persons, not proxies).
    if let (Some(limit), Some(ref creator)) = (auth.member_limit(), &payload.created_by) {
        let count_res: Option<serde_json::Value> = db
            .query("SELECT count() AS n FROM person WHERE created_by = $creator GROUP ALL")
            .bind(("creator", creator.clone()))
            .await?
            .take(0)?;
        let count = count_res
            .and_then(|v| v.get("n").and_then(|x| x.as_u64()))
            .unwrap_or(0) as usize;
        if count >= limit {
            return Err(AppError::Forbidden(format!(
                "Member limit of {limit} reached for your plan. Upgrade to add more people."
            )));
        }
    }

    // Step 1: Create canonical person record.
    // Use query bindings rather than .content(json!) to avoid JSON null vs SurrealDB NONE mismatch.
    let sex_str = match payload.sex {
        crate::models::person::Sex::Male => "Male",
        crate::models::person::Sex::Female => "Female",
    };
    let canonicals: Vec<Person> = db
        .query(
            "CREATE person SET \
             family_name         = $fn, \
             first_name          = $fi, \
             middle_name         = $mn, \
             sex                 = $sex, \
             date_of_birth       = $dob, \
             place_of_birth      = $pob, \
             date_of_death       = $dod, \
             place_of_death      = $pod, \
             birth_cert_authority = $bca, \
             birth_cert_number   = $bcn, \
             created_by          = $creator",
        )
        .bind(("fn",      payload.family_name.clone()))
        .bind(("fi",      payload.first_name.clone()))
        .bind(("mn",      payload.middle_name.clone()))
        .bind(("sex",     sex_str))
        .bind(("dob",     payload.date_of_birth.clone()))
        .bind(("pob",     payload.place_of_birth.clone()))
        .bind(("dod",     payload.date_of_death.clone()))
        .bind(("pod",     payload.place_of_death.clone()))
        .bind(("bca",     payload.birth_cert_authority.clone()))
        .bind(("bcn",     payload.birth_cert_number.clone()))
        .bind(("creator", payload.created_by.clone()))
        .await?
        .take(0)?;
    let canonical = canonicals.into_iter().next()
        .ok_or_else(|| AppError::BadRequest("Failed to create person".to_string()))?;

    // Step 2: Create proxy record linked to the canonical.
    let proxy_res: Result<Vec<PersonProxy>, _> = db
        .query(
            "CREATE person_proxy SET \
             person_id              = $pid, \
             tree                   = $tree, \
             preferred_family_name  = $pfn, \
             preferred_first_name   = $pFirst, \
             preferred_middle_name  = $pmn, \
             nickname               = $nick, \
             username               = $uname, \
             email                  = $email, \
             biography              = $bio, \
             is_private             = $private, \
             created_by             = $creator",
        )
        .bind(("pid",     canonical.id.clone()))
        .bind(("tree",    payload.tree))
        .bind(("pfn",     payload.preferred_family_name))
        .bind(("pFirst",  payload.preferred_first_name))
        .bind(("pmn",     payload.preferred_middle_name))
        .bind(("nick",    payload.nickname))
        .bind(("uname",   payload.username))
        .bind(("email",   payload.email))
        .bind(("bio",     payload.biography))
        .bind(("private", payload.is_private.unwrap_or(false)))
        .bind(("creator", payload.created_by))
        .await?
        .take(0);

    match proxy_res {
        Ok(mut proxies) if !proxies.is_empty() => {
            let proxy = proxies.remove(0);
            Ok((StatusCode::CREATED, Json(build_proxy_response(proxy, canonical))))
        }
        _ => {
            // Rollback: delete the orphaned canonical.
            let _: Option<Person> = db.delete(canonical.id).await.ok().flatten();
            Err(AppError::BadRequest("Failed to create person proxy".to_string()))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/persons",
    params(PersonFilter),
    responses(
        (status = 200, description = "List of persons (full or stub based on privacy)", body = Vec<PersonProxyResponse>),
    ),
    tag = "persons"
)]
pub async fn list_persons(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<Vec<PersonResponse>>, AppError> {
    let db = db.lock().await;

    // Resolve effective creator filter (team members can bypass the creator scope).
    let creator_filter = filter.created_by.as_deref().filter(|c| !is_admin(c));
    let effective_creator: Option<&str> = if let (Some(tree_name), Some(creator)) =
        (filter.tree.as_deref(), creator_filter)
    {
        let tree: Option<FamilyTree> = db
            .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
            .bind(("name", tree_name.to_string()))
            .await?
            .take(0)?;
        let bypass = match &tree {
            None => false,
            Some(t) => {
                let is_not_owner = t.owner != creator;
                if !is_not_owner { false }
                else if auth.user_id.is_empty() { true }
                else { t.team_id.as_ref().map_or(false, |tid| auth.teams.contains_key(tid)) }
            }
        };
        if bypass { None } else { Some(creator) }
    } else {
        creator_filter
    };

    // Fetch proxies.
    let proxies: Vec<PersonProxy> = match (effective_creator, filter.tree.as_deref()) {
        (Some(creator), Some(tree)) => db
            .query("SELECT * FROM person_proxy WHERE tree = $tree AND created_by = $creator")
            .bind(("tree", tree.to_string()))
            .bind(("creator", creator.to_string()))
            .await?
            .take(0)?,
        (Some(creator), None) => db
            .query("SELECT * FROM person_proxy WHERE created_by = $creator")
            .bind(("creator", creator.to_string()))
            .await?
            .take(0)?,
        (None, Some(tree)) => db
            .query("SELECT * FROM person_proxy WHERE tree = $tree")
            .bind(("tree", tree.to_string()))
            .await?
            .take(0)?,
        (None, None) => db.select("person_proxy").await?,
    };

    if proxies.is_empty() {
        return Ok(Json(vec![]));
    }

    // Batch-fetch all canonical persons to avoid N+1.
    let person_ids: Vec<RecordId> = proxies.iter().map(|p| p.person_id.clone()).collect();
    let canonicals: Vec<Person> = db
        .query("SELECT * FROM person WHERE id IN $ids")
        .bind(("ids", person_ids))
        .await?
        .take(0)?;

    // Build a lookup map: person ULID → Person.
    use surrealdb::types::RecordIdKey;
    let canonical_map: std::collections::HashMap<String, Person> = canonicals
        .into_iter()
        .map(|c| {
            let key = match &c.id.key {
                RecordIdKey::String(k) => k.clone(),
                other => format!("{other:?}"),
            };
            (key, c)
        })
        .collect();

    let is_member_of_tree = match filter.tree.as_deref() {
        Some(t) => {
            // Cache a single membership check for the requested tree.
            if auth.user_id.is_empty() {
                true
            } else {
                let tree: Option<FamilyTree> = db
                    .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
                    .bind(("name", t.to_string()))
                    .await?
                    .take(0)?;
                match tree {
                    None => false,
                    Some(ref tr) => {
                        tr.owner == auth.username
                            || tr.team_id.as_ref().map_or(false, |tid| auth.teams.contains_key(tid.as_str()))
                            || is_tree_editor(&*db, &auth.user_id, &[t.to_string()]).await?
                    }
                }
            }
        }
        None => auth.user_id.is_empty(),
    };

    let mut results = Vec::with_capacity(proxies.len());
    for proxy in proxies {
        let proxy_person_key = match &proxy.person_id.key {
            RecordIdKey::String(k) => k.clone(),
            other => format!("{other:?}"),
        };
        let Some(canonical) = canonical_map.get(&proxy_person_key) else { continue };

        if proxy.is_private && !is_member_of_tree {
            results.push(PersonResponse::Stub(PersonProxyStub {
                id: proxy.id,
                person_id: proxy.person_id,
                tree: proxy.tree,
                sex: canonical.sex.clone(),
                is_private: true,
            }));
        } else {
            results.push(PersonResponse::Full(build_proxy_response(proxy, canonical.clone())));
        }
    }

    Ok(Json(results))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    responses(
        (status = 200, description = "Person found (full or stub)", body = PersonProxyResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn get_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<PersonResponse>, AppError> {
    let db = db.lock().await;
    let (proxy, canonical) = fetch_proxy_and_canonical(&db, &id).await?;
    let response = redact_if_private(proxy, canonical, &auth, &db).await?;
    Ok(Json(response))
}

#[utoipa::path(
    put,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    request_body = UpdatePersonProxy,
    responses(
        (status = 200, description = "Proxy updated", body = PersonProxyResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn update_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<UpdatePersonProxy>,
) -> Result<Json<PersonProxyResponse>, AppError> {
    let db = db.lock().await;
    let (proxy, canonical) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let updated: Option<PersonProxy> = db
        .update(("person_proxy", id.as_str()))
        .merge(body)
        .await?;
    let updated = updated.ok_or(AppError::NotFound)?;
    Ok(Json(build_proxy_response(updated, canonical)))
}

#[utoipa::path(
    patch,
    path = "/api/persons/{id}/canonical",
    params(
        ("id" = String, Path, description = "Proxy ULID — resolves to the canonical person")
    ),
    request_body = UpdateCanonicalPerson,
    responses(
        (status = 200, description = "Canonical updated", body = PersonProxyResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn update_canonical(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateCanonicalPerson>,
) -> Result<Json<PersonProxyResponse>, AppError> {
    let db = db.lock().await;
    let (proxy, _) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    // Build partial JSON — only include fields that were set.
    let mut update = serde_json::Map::new();
    if let Some(v) = payload.family_name { update.insert("family_name".into(), v.into()); }
    if let Some(v) = payload.first_name { update.insert("first_name".into(), v.into()); }
    if let Some(v) = payload.middle_name { update.insert("middle_name".into(), v.into()); }
    if let Some(v) = payload.sex {
        let s = match v { crate::models::person::Sex::Male => "Male", crate::models::person::Sex::Female => "Female" };
        update.insert("sex".into(), s.into());
    }
    if let Some(v) = payload.date_of_birth { update.insert("date_of_birth".into(), v.into()); }
    if let Some(v) = payload.place_of_birth { update.insert("place_of_birth".into(), v.into()); }
    if let Some(v) = payload.date_of_death { update.insert("date_of_death".into(), v.into()); }
    if let Some(v) = payload.place_of_death { update.insert("place_of_death".into(), v.into()); }
    if let Some(v) = payload.birth_cert_authority { update.insert("birth_cert_authority".into(), v.into()); }
    if let Some(v) = payload.birth_cert_number { update.insert("birth_cert_number".into(), v.into()); }

    let canonical: Option<Person> = db
        .update(proxy.person_id.clone())
        .merge(serde_json::Value::Object(update))
        .await?;
    let canonical = canonical.ok_or(AppError::NotFound)?;
    // Re-fetch proxy (unchanged) to build the response.
    let proxy_fresh: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    Ok(Json(build_proxy_response(proxy_fresh.ok_or(AppError::NotFound)?, canonical)))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    responses(
        (status = 204, description = "Person deleted"),
    ),
    tag = "persons"
)]
pub async fn delete_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    let proxy: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    let Some(proxy) = proxy else {
        return Ok(StatusCode::NO_CONTENT);
    };
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let proxy_rid = RecordId::new("person_proxy", id.as_str());
    let canonical_id = proxy.person_id.clone();

    db.query(
        "DELETE has_father  WHERE in = $pid OR out = $pid; \
         DELETE has_mother  WHERE in = $pid OR out = $pid; \
         DELETE has_sibling WHERE in = $pid OR out = $pid; \
         DELETE has_spouse  WHERE in = $pid OR out = $pid; \
         DELETE life_event  WHERE person_proxy_id = $pid; \
         DELETE person_proxy WHERE id = $pid;",
    )
    .bind(("pid", proxy_rid))
    .await?;

    // Auto-delete canonical if no proxies remain.
    let remaining: Option<serde_json::Value> = db
        .query("SELECT count() AS n FROM person_proxy WHERE person_id = $cid GROUP ALL")
        .bind(("cid", canonical_id.clone()))
        .await?
        .take(0)?;
    let n = remaining.and_then(|v| v.get("n").and_then(|x| x.as_u64())).unwrap_or(0);
    if n == 0 {
        let _: Option<Person> = db.delete(canonical_id).await.ok().flatten();
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/trees",
    params(
        ("id" = String, Path, description = "Proxy ULID of an existing proxy for this person")
    ),
    request_body = TreeMembershipRequest,
    responses(
        (status = 201, description = "Person added to tree — returns the new proxy", body = PersonProxyResponse),
        (status = 400, description = "Tree not found or already a member", body = ErrorResponse),
        (status = 404, description = "Proxy not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn add_person_to_tree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<TreeMembershipRequest>,
) -> Result<(StatusCode, Json<PersonProxyResponse>), AppError> {
    let db = db.lock().await;

    let (proxy, canonical) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let tree_exists: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", payload.tree.clone()))
        .await?
        .take(0)?;
    if tree_exists.is_none() {
        return Err(AppError::BadRequest(format!("Family tree '{}' not found", payload.tree)));
    }

    // unique_proxy_canonical_tree index will reject a duplicate.
    let new_proxies: Vec<PersonProxy> = db
        .query(
            "CREATE person_proxy SET \
             person_id  = $pid, \
             tree       = $tree, \
             created_by = $creator",
        )
        .bind(("pid",     canonical.id.clone()))
        .bind(("tree",    payload.tree))
        .bind(("creator", proxy.created_by))
        .await?
        .take(0)?;

    let new_proxy = new_proxies
        .into_iter()
        .next()
        .ok_or_else(|| AppError::BadRequest("Failed to create proxy for tree".to_string()))?;

    Ok((StatusCode::CREATED, Json(build_proxy_response(new_proxy, canonical))))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/trees/{tree_name}",
    params(
        ("id" = String, Path, description = "Proxy ULID"),
        ("tree_name" = String, Path, description = "Tree name slug to remove the proxy from"),
    ),
    responses(
        (status = 204, description = "Person removed from tree"),
        (status = 400, description = "Cannot remove from last proxy", body = ErrorResponse),
        (status = 404, description = "Proxy not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn remove_person_from_tree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, _tree_name)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let proxy: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    let proxy = proxy.ok_or(AppError::NotFound)?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    // Require at least one remaining proxy after deletion.
    let remaining: Option<serde_json::Value> = db
        .query("SELECT count() AS n FROM person_proxy WHERE person_id = $cid GROUP ALL")
        .bind(("cid", proxy.person_id.clone()))
        .await?
        .take(0)?;
    let n = remaining.and_then(|v| v.get("n").and_then(|x| x.as_u64())).unwrap_or(0);
    if n <= 1 {
        return Err(AppError::BadRequest(
            "Cannot remove person from their last tree — use DELETE /api/persons/{id} instead".to_string(),
        ));
    }

    let proxy_rid = RecordId::new("person_proxy", id.as_str());
    db.query(
        "DELETE has_father  WHERE in = $pid OR out = $pid; \
         DELETE has_mother  WHERE in = $pid OR out = $pid; \
         DELETE has_sibling WHERE in = $pid OR out = $pid; \
         DELETE has_spouse  WHERE in = $pid OR out = $pid; \
         DELETE life_event  WHERE person_proxy_id = $pid; \
         DELETE person_proxy WHERE id = $pid;",
    )
    .bind(("pid", proxy_rid))
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// List all proxy records (across trees) that share the same canonical person as this proxy.
#[utoipa::path(
    get,
    path = "/api/persons/{id}/linked-proxies",
    params(
        ("id" = String, Path, description = "Proxy ULID")
    ),
    responses(
        (status = 200, description = "Other proxies for the same canonical person", body = Vec<PersonProxyResponse>),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn list_proxy_links(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<Vec<PersonResponse>>, AppError> {
    let db = db.lock().await;
    let proxy: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    let proxy = proxy.ok_or(AppError::NotFound)?;

    let proxies: Vec<PersonProxy> = db
        .query("SELECT * FROM person_proxy WHERE person_id = $cid AND id != $self")
        .bind(("cid", proxy.person_id.clone()))
        .bind(("self", proxy.id.clone()))
        .await?
        .take(0)?;

    let canonical: Option<Person> = db.select(proxy.person_id).await?;
    let canonical = canonical.ok_or(AppError::NotFound)?;

    let mut results = Vec::with_capacity(proxies.len());
    for p in proxies {
        let tree = p.tree.clone();
        let is_priv = p.is_private;
        let resp = if is_priv {
            let member = is_tree_member(&tree, &auth, &db).await?;
            if member {
                PersonResponse::Full(build_proxy_response(p, canonical.clone()))
            } else {
                PersonResponse::Stub(PersonProxyStub {
                    id: p.id,
                    person_id: p.person_id,
                    tree: p.tree,
                    sex: canonical.sex.clone(),
                    is_private: true,
                })
            }
        } else {
            PersonResponse::Full(build_proxy_response(p, canonical.clone()))
        };
        results.push(resp);
    }

    Ok(Json(results))
}

/// Collapse two proxies in the same tree into one survivor, re-pointing their edges and life events.
#[utoipa::path(
    post,
    path = "/api/persons/{id}/collapse-into/{survivor_id}",
    params(
        ("id" = String, Path, description = "Proxy ULID to collapse (loser)"),
        ("survivor_id" = String, Path, description = "Proxy ULID to keep (survivor)"),
    ),
    responses(
        (status = 204, description = "Collapsed"),
        (status = 400, description = "Proxies are not in the same tree", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn collapse_same_tree_duplicate(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, survivor_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let loser: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    let loser = loser.ok_or(AppError::NotFound)?;
    let survivor: Option<PersonProxy> = db.select(("person_proxy", survivor_id.as_str())).await?;
    let survivor = survivor.ok_or(AppError::NotFound)?;

    if loser.tree != survivor.tree {
        return Err(AppError::BadRequest(
            "Both proxies must be in the same tree to collapse".to_string(),
        ));
    }

    can_write_to_proxy(&loser, &auth, &db).await?;

    let loser_rid   = RecordId::new("person_proxy", id.as_str());
    let survivor_rid = RecordId::new("person_proxy", survivor_id.as_str());
    let loser_canonical = loser.person_id.clone();

    // Redirect edges that point to/from the loser proxy to the survivor, delete duplicates.
    db.query(
        "UPDATE has_father  SET in  = $s WHERE in  = $l AND NOT (SELECT id FROM has_father  WHERE in  = $s AND out = out LIMIT 1); \
         DELETE has_father  WHERE in  = $l; \
         UPDATE has_father  SET out = $s WHERE out = $l AND NOT (SELECT id FROM has_father  WHERE out = $s AND in  = in  LIMIT 1); \
         DELETE has_father  WHERE out = $l; \
         UPDATE has_mother  SET in  = $s WHERE in  = $l AND NOT (SELECT id FROM has_mother  WHERE in  = $s AND out = out LIMIT 1); \
         DELETE has_mother  WHERE in  = $l; \
         UPDATE has_mother  SET out = $s WHERE out = $l AND NOT (SELECT id FROM has_mother  WHERE out = $s AND in  = in  LIMIT 1); \
         DELETE has_mother  WHERE out = $l; \
         UPDATE has_sibling SET in  = $s WHERE in  = $l AND NOT (SELECT id FROM has_sibling WHERE in  = $s AND out = out LIMIT 1); \
         DELETE has_sibling WHERE in  = $l; \
         UPDATE has_sibling SET out = $s WHERE out = $l AND NOT (SELECT id FROM has_sibling WHERE out = $s AND in  = in  LIMIT 1); \
         DELETE has_sibling WHERE out = $l; \
         UPDATE has_spouse  SET in  = $s WHERE in  = $l AND NOT (SELECT id FROM has_spouse  WHERE in  = $s AND out = out LIMIT 1); \
         DELETE has_spouse  WHERE in  = $l; \
         UPDATE has_spouse  SET out = $s WHERE out = $l AND NOT (SELECT id FROM has_spouse  WHERE out = $s AND in  = in  LIMIT 1); \
         DELETE has_spouse  WHERE out = $l; \
         UPDATE life_event SET person_proxy_id = $s WHERE person_proxy_id = $l; \
         DELETE person_proxy WHERE id = $l;",
    )
    .bind(("l", loser_rid))
    .bind(("s", survivor_rid))
    .await?;

    // Auto-delete the loser's canonical if it has no remaining proxies (not survivor's canonical).
    if loser.person_id != survivor.person_id {
        let rem: Option<serde_json::Value> = db
            .query("SELECT count() AS n FROM person_proxy WHERE person_id = $cid GROUP ALL")
            .bind(("cid", loser_canonical.clone()))
            .await?
            .take(0)?;
        let n = rem.and_then(|v| v.get("n").and_then(|x| x.as_u64())).unwrap_or(0);
        if n == 0 {
            let _: Option<Person> = db.delete(loser_canonical).await.ok().flatten();
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/find-duplicates",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    responses(
        (status = 200, description = "Duplicate search result", body = crate::models::contact_request::DuplicateSearchResult),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn find_duplicates(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<crate::models::contact_request::DuplicateSearchResult>, AppError> {
    let db = db.lock().await;
    let (proxy, canonical) = fetch_proxy_and_canonical(&db, &id).await?;

    let requester = if auth.username.is_empty() {
        proxy.created_by.clone().unwrap_or_default()
    } else {
        auth.username.clone()
    };

    let mut res = db
        .query(
            "SELECT count() AS count, array::group(created_by) AS owners \
             FROM person_proxy \
             WHERE string::lowercase(person_id.family_name) = string::lowercase($family_name) \
               AND string::lowercase(person_id.first_name) = string::lowercase($first_name) \
               AND person_id != $canonical_id \
               AND created_by != $requester \
             GROUP ALL",
        )
        .bind(("family_name", canonical.family_name.clone()))
        .bind(("first_name", canonical.first_name.clone()))
        .bind(("canonical_id", canonical.id.clone()))
        .bind(("requester", requester))
        .await?;

    let row: Option<serde_json::Value> = res.take(0)?;
    let count = row.as_ref().and_then(|v| v.get("count")).and_then(|v| v.as_u64()).unwrap_or(0);
    let owners: Vec<String> = row
        .as_ref()
        .and_then(|v| v.get("owners"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    Ok(Json(crate::models::contact_request::DuplicateSearchResult { count, owners }))
}
