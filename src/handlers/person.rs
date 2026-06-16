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
    models::{family_tree::FamilyTree, person::{CreatePerson, Person, TreeMembershipRequest, UpdatePerson}},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PersonFilter {
    /// When present, only return/allow access to persons with this `created_by` value.
    pub created_by: Option<String>,
    /// When present, only return persons belonging to this family tree.
    pub tree: Option<String>,
}

/// Returns true if `username` matches the configured admin username
/// (`ADMIN_USERNAME` env var, defaulting to `"theboss"`).
fn is_admin(username: &str) -> bool {
    let admin = std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "theboss".to_string());
    username == admin
}

/// Returns `AppError::NotFound` if `created_by` is set, is not the admin
/// username, and does not match the person's `created_by` field.
pub fn check_ownership(person: &Person, created_by: &Option<String>) -> Result<(), AppError> {
    if let Some(ref creator) = created_by {
        if !is_admin(creator) && person.created_by.as_deref() != Some(creator.as_str()) {
            return Err(AppError::NotFound);
        }
    }
    Ok(())
}

/// Returns `true` if `user_id` (JWT sub UUID) has an editor grant for any of
/// the given trees.
pub async fn is_tree_editor(db: &DbConn, user_id: &str, trees: &[String]) -> Result<bool, AppError> {
    if trees.is_empty() || user_id.is_empty() {
        return Ok(false);
    }
    let grant: Option<serde_json::Value> = db
        .query(
            "SELECT id FROM tree_editor \
             WHERE user_id = $uid AND tree_name IN $trees LIMIT 1",
        )
        .bind(("uid", user_id.to_string()))
        .bind(("trees", trees.to_vec()))
        .await?
        .take(0)?;
    Ok(grant.is_some())
}

/// Confirms the authenticated caller may write to `person`.
///
/// Passes when:
/// - dev mode (no JWT_SECRET, `auth.user_id` is empty), OR
/// - the caller is the admin user, OR
/// - the person was created by the caller (`created_by == auth.username`), OR
/// - the caller is the team owner of any tree the person belongs to, OR
/// - the caller has an explicit editor grant for any of those trees.
///
/// Returns `AppError::NotFound` on denial (same surface as read auth, so
/// unauthenticated callers cannot distinguish missing from forbidden).
pub async fn can_write_to_person(
    person: &Person,
    auth: &ClannAuth,
    db: &DbConn,
) -> Result<(), AppError> {
    // Dev mode: no JWT enforcement.
    if auth.user_id.is_empty() {
        return Ok(());
    }

    // Admin bypass.
    if !auth.username.is_empty() && is_admin(&auth.username) {
        return Ok(());
    }

    // Creator can always write their own records.
    if !auth.username.is_empty()
        && person.created_by.as_deref() == Some(auth.username.as_str())
    {
        return Ok(());
    }

    // Check team-level and tree-editor access in one pass.
    if !person.trees.is_empty() {
        // Fetch team_id for each tree the person belongs to.
        let trees_info: Vec<serde_json::Value> = db
            .query("SELECT team_id FROM family_tree WHERE name IN $trees")
            .bind(("trees", person.trees.clone()))
            .await?
            .take(0)?;

        for t in &trees_info {
            if let Some(tid) = t.get("team_id").and_then(|v| v.as_str()) {
                // Team owner has full write access to all persons in their team trees.
                if auth.teams.get(tid).map(|r| r == "owner").unwrap_or(false) {
                    return Ok(());
                }
            }
        }

        // Per-tree editor grant.
        if is_tree_editor(db, &auth.user_id, &person.trees).await? {
            return Ok(());
        }
    }

    Err(AppError::NotFound)
}

#[utoipa::path(
    post,
    path = "/api/persons",
    request_body = CreatePerson,
    responses(
        (status = 201, description = "Person created", body = Person),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 422, description = "Invalid request body", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn create_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreatePerson>,
) -> Result<(StatusCode, Json<Person>), AppError> {
    let db = db.lock().await;

    if payload.trees.is_empty() {
        return Err(AppError::BadRequest("At least one tree must be specified".to_string()));
    }

    // Enforce subscription member limit (total members created by this user)
    if let (Some(limit), Some(ref creator)) = (auth.member_limit(), &payload.created_by) {
        let existing: Vec<Person> = db
            .query("SELECT * FROM person WHERE created_by = $creator")
            .bind(("creator", creator.clone()))
            .await?
            .take(0)?;
        if existing.len() >= limit {
            return Err(AppError::Forbidden(format!(
                "Member limit of {limit} reached for your plan. Upgrade to add more people."
            )));
        }
    }

    // Validate all specified trees exist
    for tree_name in &payload.trees {
        let tree_exists: Option<FamilyTree> = db
            .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
            .bind(("name", tree_name.clone()))
            .await?
            .take(0)?;
        if tree_exists.is_none() {
            return Err(AppError::BadRequest(format!(
                "Family tree '{}' not found",
                tree_name
            )));
        }
    }

    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let person: Option<Person> = db.create("person").content(body).await?;
    let person = person.ok_or(AppError::BadRequest("Failed to create person".to_string()))?;
    Ok((StatusCode::CREATED, Json(person)))
}

#[utoipa::path(
    get,
    path = "/api/persons",
    params(PersonFilter),
    responses(
        (status = 200, description = "List of persons, optionally filtered", body = Vec<Person>),
    ),
    tag = "persons"
)]
pub async fn list_persons(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<Vec<Person>>, AppError> {
    let db = db.lock().await;
    let creator_filter = filter.created_by.as_deref().filter(|c| !is_admin(c));

    // When a specific tree is requested, check whether this is a team member
    // accessing a tree they don't own. In JWT mode the team UUID is confirmed
    // against auth.teams; in dev mode (auth.user_id empty, teams always empty)
    // we fall back to the tree-owner field as the authority.
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
                let is_team_linked = t.team_id.is_some();
                let is_not_owner = t.owner != creator;
                if !is_team_linked || !is_not_owner {
                    false
                } else if auth.user_id.is_empty() {
                    // Dev mode: no JWT validation — trust tree-owner check
                    true
                } else {
                    // JWT mode: require the team to appear in the caller's claims
                    t.team_id.as_ref().map_or(false, |tid| auth.teams.contains_key(tid))
                }
            }
        };

        if bypass { None } else { Some(creator) }
    } else {
        creator_filter
    };

    let persons: Vec<Person> = match (effective_creator, filter.tree.as_deref()) {
        (Some(creator), Some(tree)) => db
            .query("SELECT * FROM person WHERE created_by = $creator AND $tree IN trees")
            .bind(("creator", creator.to_string()))
            .bind(("tree", tree.to_string()))
            .await?
            .take(0)?,
        (Some(creator), None) => db
            .query("SELECT * FROM person WHERE created_by = $creator")
            .bind(("creator", creator.to_string()))
            .await?
            .take(0)?,
        (None, Some(tree)) => db
            .query("SELECT * FROM person WHERE $tree IN trees")
            .bind(("tree", tree.to_string()))
            .await?
            .take(0)?,
        (None, None) => db.select("person").await?,
    };
    Ok(Json(persons))
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Person found", body = Person),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn get_person(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<Person>, AppError> {
    let db = db.lock().await;
    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;
    Ok(Json(person))
}

#[utoipa::path(
    put,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = UpdatePerson,
    responses(
        (status = 200, description = "Person updated", body = Person),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn update_person(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<UpdatePerson>,
) -> Result<Json<Person>, AppError> {
    let db = db.lock().await;
    let check: Option<Person> = db.select(("person", id.as_str())).await?;
    can_write_to_person(&check.ok_or(AppError::NotFound)?, &auth, &db).await?;
    let body = serde_json::to_value(&payload)
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let person: Option<Person> = db.update(("person", id.as_str())).merge(body).await?;
    let person = person.ok_or(AppError::NotFound)?;
    Ok(Json(person))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
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
    Query(filter): Query<PersonFilter>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    let check: Option<Person> = db.select(("person", id.as_str())).await?;
    let Some(person) = check else {
        return Ok(StatusCode::NO_CONTENT);
    };
    can_write_to_person(&person, &auth, &db).await?;
    let _: Option<Person> = db.delete(("person", id.as_str())).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/trees",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = TreeMembershipRequest,
    responses(
        (status = 204, description = "Person added to tree"),
        (status = 400, description = "Tree not found", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn add_person_to_tree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<TreeMembershipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    can_write_to_person(&person, &auth, &db).await?;

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

    db.query("UPDATE $person SET trees = array::union(trees, [$tree])")
        .bind(("person", RecordId::new("person", id.as_str())))
        .bind(("tree", payload.tree))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/trees/{tree_name}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("tree_name" = String, Path, description = "Tree name (slug) to remove"),
    ),
    responses(
        (status = 204, description = "Person removed from tree"),
        (status = 400, description = "Cannot remove from last tree", body = ErrorResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "persons"
)]
pub async fn remove_person_from_tree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, tree_name)): Path<(String, String)>,
    Query(filter): Query<PersonFilter>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let person: Option<Person> = db.select(("person", id.as_str())).await?;
    let person = person.ok_or(AppError::NotFound)?;
    can_write_to_person(&person, &auth, &db).await?;

    if person.trees.len() <= 1 {
        return Err(AppError::BadRequest(
            "Cannot remove person from their last tree".to_string(),
        ));
    }

    db.query("UPDATE $person SET trees -= $tree")
        .bind(("person", RecordId::new("person", id.as_str())))
        .bind(("tree", tree_name))
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
