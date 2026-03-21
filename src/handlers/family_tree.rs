use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{
    db::Db,
    error::{AppError, ErrorResponse},
    models::family_tree::{CreateFamilyTree, FamilyTree},
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TreeFilter {
    /// When present, only return trees with this owner.
    pub owner: Option<String>,
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
    Json(payload): Json<CreateFamilyTree>,
) -> Result<(StatusCode, Json<FamilyTree>), AppError> {
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
    Query(filter): Query<TreeFilter>,
) -> Result<Json<Vec<FamilyTree>>, AppError> {
    let trees: Vec<FamilyTree> = match filter.owner {
        Some(ref owner) => db
            .query("SELECT * FROM family_tree WHERE owner = $owner")
            .bind(("owner", owner.clone()))
            .await?
            .take(0)?,
        None => db.select("family_tree").await?,
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
    // Verify the tree exists
    let tree: Option<FamilyTree> = db
        .query("SELECT * FROM family_tree WHERE name = $name LIMIT 1")
        .bind(("name", name.clone()))
        .await?
        .take(0)?;
    tree.ok_or(AppError::NotFound)?;

    // Cascade: delete relationship edges involving persons in this tree, then the persons
    db.query(
        "LET $pids = (SELECT VALUE id FROM person WHERE tree = $name);
         DELETE has_father  WHERE in IN $pids OR out IN $pids;
         DELETE has_mother  WHERE in IN $pids OR out IN $pids;
         DELETE has_sibling WHERE in IN $pids OR out IN $pids;
         DELETE has_spouse  WHERE in IN $pids OR out IN $pids;
         DELETE person WHERE tree = $name;
         DELETE family_tree WHERE name = $name;",
    )
    .bind(("name", name))
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
