use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::family_tree::{CreateFamilyTree, FamilyTree, SetTreeTeam, UpdateFamilyTree},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TreeFilter {
    /// When present, only return trees with this owner.
    pub owner: Option<String>,
    /// When present, return trees linked to this team UUID (caller must be a member).
    pub team_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/trees",
    request_body = CreateFamilyTree,
    responses(
        (status = 201, description = "Family tree created", body = FamilyTree),
        (status = 400, description = "Name already taken or bad request", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn create_tree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreateFamilyTree>,
) -> Result<(StatusCode, Json<FamilyTree>), AppError> {
    let db = db.lock().await;

    // Enforce subscription tree limit
    if let Some(limit) = auth.tree_limit() {
        let existing: Vec<FamilyTree> = db
            .query("SELECT * FROM family_tree WHERE owner = $owner")
            .bind(("owner", payload.owner.clone()))
            .await?
            .take(0)?;
        if existing.len() >= limit {
            return Err(AppError::Forbidden(format!(
                "Tree limit of {limit} reached for your plan. Upgrade to create more trees."
            )));
        }
    }

    // Enforce unique name
    let existing: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", payload.name.clone()))
        .await?
        .take(0)?;
    if existing.is_some() {
        return Err(AppError::BadRequest(format!(
            "A family tree with name '{}' already exists",
            payload.name
        )));
    }

    // If this will be primary, clear is_primary on existing trees for this owner first
    if payload.is_primary {
        db.query("UPDATE family_tree SET is_primary = false WHERE owner = $owner")
            .bind(("owner", payload.owner.clone()))
            .await?;
    }

    let tree: Option<FamilyTree> = db.create("family_tree").content(payload).await?;
    let tree = tree.ok_or(AppError::BadRequest("Failed to create family tree".to_string()))?;

    Ok((StatusCode::CREATED, Json(tree)))
}

#[utoipa::path(
    get,
    path = "/api/trees",
    params(TreeFilter),
    responses(
        (status = 200, description = "List of family trees", body = Vec<FamilyTree>),
    ),
    tag = "trees"
)]
pub async fn list_trees(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Query(filter): Query<TreeFilter>,
) -> Result<Json<Vec<FamilyTree>>, AppError> {
    let db = db.lock().await;
    let trees: Vec<FamilyTree> = match (&filter.owner, &filter.team_id) {
        (_, Some(team_id)) => {
            // Only active team members may list a team's trees.
            // In JWT mode (user_id non-empty) verify the team appears in claims;
            // in dev mode (no JWT_SECRET, user_id empty) skip the check.
            let is_member = auth.user_id.is_empty() || auth.teams.contains_key(team_id.as_str());
            if !is_member {
                return Err(AppError::Forbidden("Not a member of this team".to_string()));
            }
            db.query("SELECT * FROM family_tree WHERE team_id = $team_id")
                .bind(("team_id", team_id.clone()))
                .await?
                .take(0)?
        }
        (Some(owner), None) => db
            .query("SELECT * FROM family_tree WHERE owner = $owner")
            .bind(("owner", owner.clone()))
            .await?
            .take(0)?,
        (None, None) => db.select("family_tree").await?,
    };
    Ok(Json(trees))
}

#[utoipa::path(
    get,
    path = "/api/trees/{name}",
    params(
        ("name" = String, Path, description = "Unique tree name")
    ),
    responses(
        (status = 200, description = "Family tree found", body = FamilyTree),
        (status = 404, description = "Family tree not found", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn get_tree(
    State(db): State<Db>,
    Path(name): Path<String>,
) -> Result<Json<FamilyTree>, AppError> {
    let db = db.lock().await;
    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name))
        .await?
        .take(0)?;
    Ok(Json(tree.ok_or(AppError::NotFound)?))
}

#[utoipa::path(
    patch,
    path = "/api/trees/{name}/set-primary",
    params(
        ("name" = String, Path, description = "Unique tree name")
    ),
    responses(
        (status = 200, description = "Tree set as primary", body = FamilyTree),
        (status = 404, description = "Family tree not found", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn set_primary_tree(
    State(db): State<Db>,
    Path(name): Path<String>,
) -> Result<Json<FamilyTree>, AppError> {
    let db = db.lock().await;

    // Verify the tree exists and get its owner
    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    let tree = tree.ok_or(AppError::NotFound)?;

    // Clear is_primary on all trees for this owner, then set this one
    db.query(
        "UPDATE family_tree SET is_primary = false WHERE owner = $owner;
         UPDATE family_tree SET is_primary = true  WHERE name  = $name;",
    )
    .bind(("owner", tree.owner.clone()))
    .bind(("name", name.clone()))
    .await?;

    let updated: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name))
        .await?
        .take(0)?;
    Ok(Json(updated.ok_or(AppError::NotFound)?))
}

#[utoipa::path(
    patch,
    path = "/api/trees/{name}",
    params(
        ("name" = String, Path, description = "Unique tree name")
    ),
    request_body = UpdateFamilyTree,
    responses(
        (status = 200, description = "Tree updated", body = FamilyTree),
        (status = 404, description = "Family tree not found", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn update_tree(
    State(db): State<Db>,
    Path(name): Path<String>,
    Json(payload): Json<UpdateFamilyTree>,
) -> Result<Json<FamilyTree>, AppError> {
    let db = db.lock().await;

    let existing: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    existing.ok_or(AppError::NotFound)?;

    db.query("UPDATE family_tree SET display_name = $display_name WHERE name = $name")
        .bind(("display_name", payload.display_name))
        .bind(("name", name.clone()))
        .await?;

    let updated: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name))
        .await?
        .take(0)?;
    Ok(Json(updated.ok_or(AppError::NotFound)?))
}

#[utoipa::path(
    patch,
    path = "/api/trees/{name}/team",
    params(
        ("name" = String, Path, description = "Unique tree name")
    ),
    request_body = SetTreeTeam,
    responses(
        (status = 200, description = "Tree team association updated", body = FamilyTree),
        (status = 404, description = "Family tree not found", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn set_tree_team(
    State(db): State<Db>,
    Path(name): Path<String>,
    Json(payload): Json<SetTreeTeam>,
) -> Result<Json<FamilyTree>, AppError> {
    let db = db.lock().await;

    let existing: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    let existing = existing.ok_or(AppError::NotFound)?;

    let updated: Option<FamilyTree> = db
        .update(existing.id)
        .merge(serde_json::json!({ "team_id": payload.team_id }))
        .await?;
    Ok(Json(updated.ok_or(AppError::NotFound)?))
}

#[utoipa::path(
    delete,
    path = "/api/trees/{name}",
    params(
        ("name" = String, Path, description = "Unique tree name")
    ),
    responses(
        (status = 204, description = "Family tree and all its persons deleted"),
        (status = 404, description = "Family tree not found", body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn delete_tree(
    State(db): State<Db>,
    Path(name): Path<String>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    // Verify the tree exists
    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    tree.ok_or(AppError::NotFound)?;

    // Cascade: delete all proxies for this tree with their edges and life events,
    // then auto-delete any canonical persons that have no remaining proxies.
    db.query(
        "LET $proxy_ids = (SELECT VALUE id FROM person_proxy WHERE tree = $name); \
         LET $canonical_ids = (SELECT VALUE person_id FROM person_proxy WHERE tree = $name); \
         DELETE has_father  WHERE in IN $proxy_ids OR out IN $proxy_ids; \
         DELETE has_mother  WHERE in IN $proxy_ids OR out IN $proxy_ids; \
         DELETE has_sibling WHERE in IN $proxy_ids OR out IN $proxy_ids; \
         DELETE has_spouse  WHERE in IN $proxy_ids OR out IN $proxy_ids; \
         DELETE life_event  WHERE person_proxy_id IN $proxy_ids; \
         DELETE person_proxy WHERE id IN $proxy_ids; \
         DELETE person WHERE id IN $canonical_ids \
           AND (SELECT count() AS n FROM person_proxy WHERE person_id = person.id GROUP ALL)[0].n = 0; \
         DELETE family_tree WHERE name = $name;",
    )
    .bind(("name", name))
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
