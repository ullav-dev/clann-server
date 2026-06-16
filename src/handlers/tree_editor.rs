use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    models::{
        family_tree::FamilyTree,
        tree_editor::{AddTreeEditor, TreeAccessResponse, TreeEditor},
    },
};

/// Fetch the tree and confirm the caller is the team owner. Used by editor
/// management endpoints that require team-owner authority.
async fn require_team_owner(
    db: &crate::db::DbConn,
    tree_name: &str,
    auth: &ClannAuth,
) -> Result<FamilyTree, AppError> {
    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", tree_name.to_string()))
        .await?
        .take(0)?;
    let tree = tree.ok_or(AppError::NotFound)?;

    let team_id = tree.team_id.as_deref().ok_or_else(|| {
        AppError::Forbidden("This tree is not linked to a team".to_string())
    })?;

    if !auth.user_id.is_empty() {
        let role = auth.teams.get(team_id).map(|s| s.as_str());
        if role != Some("owner") {
            return Err(AppError::Forbidden(
                "Only the team owner can manage tree editors".to_string(),
            ));
        }
    }

    Ok(tree)
}

#[utoipa::path(
    get,
    path = "/api/trees/{name}/editors",
    params(("name" = String, Path, description = "Unique tree name")),
    responses(
        (status = 200, body = Vec<TreeEditor>),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn list_tree_editors(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(name): Path<String>,
) -> Result<Json<Vec<TreeEditor>>, AppError> {
    let db = db.lock().await;
    require_team_owner(&db, &name, &auth).await?;
    let editors: Vec<TreeEditor> = db
        .query("SELECT * FROM tree_editor WHERE tree_name = $name ORDER BY granted_at ASC")
        .bind(("name", name))
        .await?
        .take(0)?;
    Ok(Json(editors))
}

#[utoipa::path(
    post,
    path = "/api/trees/{name}/editors",
    params(("name" = String, Path, description = "Unique tree name")),
    request_body = AddTreeEditor,
    responses(
        (status = 201, body = TreeEditor),
        (status = 400, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn add_tree_editor(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(name): Path<String>,
    Json(payload): Json<AddTreeEditor>,
) -> Result<(StatusCode, Json<TreeEditor>), AppError> {
    let db = db.lock().await;
    require_team_owner(&db, &name, &auth).await?;

    // The UNIQUE index (tree_name, user_id) means a duplicate CREATE will error.
    // Handle that gracefully by checking first.
    let existing: Option<TreeEditor> = db
        .query("SELECT * FROM tree_editor WHERE tree_name = $tree AND user_id = $uid LIMIT 1")
        .bind(("tree", name.clone()))
        .bind(("uid", payload.user_id.clone()))
        .await?
        .take(0)?;
    if existing.is_some() {
        return Err(AppError::BadRequest("User is already an editor for this tree".to_string()));
    }

    let editors: Vec<TreeEditor> = db
        .query(
            "CREATE tree_editor SET \
             tree_name          = $tree, \
             user_id            = $uid, \
             granted_by_user_id = $grantor, \
             granted_at         = <string>time::now()",
        )
        .bind(("tree", name))
        .bind(("uid", payload.user_id))
        .bind(("grantor", auth.user_id.clone()))
        .await?
        .take(0)?;

    editors
        .into_iter()
        .next()
        .map(|e| (StatusCode::CREATED, Json(e)))
        .ok_or_else(|| AppError::BadRequest("Failed to grant editor access".to_string()))
}

#[utoipa::path(
    delete,
    path = "/api/trees/{name}/editors/{user_id}",
    params(
        ("name" = String, Path, description = "Unique tree name"),
        ("user_id" = String, Path, description = "UUID of the user to revoke"),
    ),
    responses(
        (status = 204, description = "Editor access revoked"),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn remove_tree_editor(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((name, user_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;
    require_team_owner(&db, &name, &auth).await?;
    db.query("DELETE tree_editor WHERE tree_name = $tree AND user_id = $uid")
        .bind(("tree", name))
        .bind(("uid", user_id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/trees/{name}/my-access",
    params(("name" = String, Path, description = "Unique tree name")),
    responses(
        (status = 200, body = TreeAccessResponse),
        (status = 404, body = ErrorResponse),
    ),
    tag = "trees"
)]
pub async fn get_my_tree_access(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(name): Path<String>,
) -> Result<Json<TreeAccessResponse>, AppError> {
    let db = db.lock().await;

    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    let tree = tree.ok_or(AppError::NotFound)?;

    // Dev mode: grant owner access so local testing works without JWT.
    if auth.user_id.is_empty() {
        return Ok(Json(TreeAccessResponse { role: "owner".to_string() }));
    }

    // Team owner has full write access.
    let is_team_owner = tree
        .team_id
        .as_deref()
        .and_then(|tid| auth.teams.get(tid))
        .map(|r| r == "owner")
        .unwrap_or(false);

    if is_team_owner {
        return Ok(Json(TreeAccessResponse { role: "owner".to_string() }));
    }

    // Check for a per-tree editor grant.
    let grant: Option<serde_json::Value> = db
        .query(
            "SELECT id FROM tree_editor \
             WHERE tree_name = $name AND user_id = $uid LIMIT 1",
        )
        .bind(("name", name))
        .bind(("uid", auth.user_id.clone()))
        .await?
        .take(0)?;

    let role = if grant.is_some() { "editor" } else { "viewer" };
    Ok(Json(TreeAccessResponse { role: role.to_string() }))
}
