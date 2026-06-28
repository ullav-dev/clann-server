use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use surrealdb::types::RecordId;

use crate::{
    auth::ClannAuth,
    db::Db,
    error::{AppError, ErrorResponse},
    handlers::person::can_write_to_proxy,
    models::{
        merge_proposal::{
            AcceptMergeProposalRequest, CreateMergeProposalRequest, MergeProposal,
        },
        person::{Person, PersonProxy},
    },
};

fn parse_proxy_id(s: &str) -> Result<RecordId, AppError> {
    let (tb, id) = s
        .split_once(':')
        .ok_or_else(|| AppError::BadRequest(format!("Expected 'table:id', got: {s}")))?;
    if tb != "person_proxy" {
        return Err(AppError::BadRequest(format!(
            "Expected person_proxy record ID, got table: {tb}"
        )));
    }
    Ok(RecordId::new(tb, id))
}

#[utoipa::path(
    post,
    path = "/api/merge-proposals",
    request_body = CreateMergeProposalRequest,
    responses(
        (status = 201, description = "Proposal created", body = MergeProposal),
        (status = 400, description = "Invalid proxies or duplicate proposal", body = ErrorResponse),
        (status = 404, description = "Proxy not found", body = ErrorResponse),
    ),
    tag = "merge"
)]
pub async fn create_merge_proposal(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Json(payload): Json<CreateMergeProposalRequest>,
) -> Result<(StatusCode, Json<MergeProposal>), AppError> {
    let db = db.lock().await;

    let from_rid = parse_proxy_id(&payload.from_proxy_id)?;
    let to_rid   = parse_proxy_id(&payload.to_proxy_id)?;

    let from_proxy: Option<PersonProxy> = db.select(from_rid.clone()).await?;
    let from_proxy = from_proxy.ok_or(AppError::NotFound)?;
    let to_proxy: Option<PersonProxy> = db.select(to_rid.clone()).await?;
    let to_proxy = to_proxy.ok_or(AppError::NotFound)?;

    // Validate: different trees, different canonicals.
    if from_proxy.tree == to_proxy.tree {
        return Err(AppError::BadRequest(
            "Use collapse-into for same-tree deduplication".to_string(),
        ));
    }
    if from_proxy.person_id == to_proxy.person_id {
        return Err(AppError::BadRequest(
            "These proxies already share the same canonical person".to_string(),
        ));
    }

    can_write_to_proxy(&from_proxy, &auth, &db).await?;

    // Reject duplicate pending proposals.
    let dup: Option<serde_json::Value> = db
        .query(
            "SELECT id FROM person_merge_proposal \
             WHERE status = 'pending' \
             AND ((from_proxy_id = $fp AND to_proxy_id = $tp) \
               OR (from_proxy_id = $tp AND to_proxy_id = $fp)) LIMIT 1",
        )
        .bind(("fp", from_rid.clone()))
        .bind(("tp", to_rid.clone()))
        .await?
        .take(0)?;
    if dup.is_some() {
        return Err(AppError::BadRequest("A pending proposal already exists for these proxies".to_string()));
    }

    let proposer = if auth.username.is_empty() {
        from_proxy.created_by.clone().unwrap_or_default()
    } else {
        auth.username.clone()
    };

    let proposals: Vec<MergeProposal> = db
        .query(
            "CREATE person_merge_proposal SET \
             from_proxy_id = $fp, \
             to_proxy_id   = $tp, \
             proposed_by   = $proposer",
        )
        .bind(("fp",       from_rid))
        .bind(("tp",       to_rid))
        .bind(("proposer", proposer))
        .await?
        .take(0)?;

    proposals
        .into_iter()
        .next()
        .map(|p| (StatusCode::CREATED, Json(p)))
        .ok_or_else(|| AppError::BadRequest("Failed to create merge proposal".to_string()))
}

#[utoipa::path(
    get,
    path = "/api/merge-proposals",
    responses(
        (status = 200, description = "List of proposals involving the caller's proxies", body = Vec<MergeProposal>),
    ),
    tag = "merge"
)]
pub async fn list_merge_proposals(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
) -> Result<Json<Vec<MergeProposal>>, AppError> {
    let db = db.lock().await;

    // List proposals where the caller owns either the from or to proxy.
    // Includes pending incoming proposals (recipient hasn't responded yet → responded_by is null).
    let proposals: Vec<MergeProposal> = if auth.username.is_empty() {
        db.select("person_merge_proposal").await?
    } else {
        db.query(
            "SELECT * FROM person_merge_proposal \
             WHERE proposed_by = $user \
                OR responded_by = $user \
                OR from_proxy_id IN (SELECT id FROM person_proxy WHERE created_by = $user) \
                OR to_proxy_id   IN (SELECT id FROM person_proxy WHERE created_by = $user) \
             ORDER BY proposed_at DESC",
        )
        .bind(("user", auth.username.clone()))
        .await?
        .take(0)?
    };

    Ok(Json(proposals))
}

#[utoipa::path(
    get,
    path = "/api/merge-proposals/{id}",
    params(("id" = String, Path, description = "Proposal ULID")),
    responses(
        (status = 200, description = "Proposal detail", body = MergeProposal),
        (status = 404, body = ErrorResponse),
    ),
    tag = "merge"
)]
pub async fn get_merge_proposal(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<MergeProposal>, AppError> {
    let db = db.lock().await;
    let eid = RecordId::new("person_merge_proposal", id.as_str());
    let proposal: Option<MergeProposal> = db.select(eid).await?;
    proposal.map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    patch,
    path = "/api/merge-proposals/{id}/accept",
    params(("id" = String, Path, description = "Proposal ULID")),
    request_body = AcceptMergeProposalRequest,
    responses(
        (status = 200, description = "Accepted — canonicals merged", body = MergeProposal),
        (status = 400, description = "Already decided or invalid", body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, description = "Canonical conflicts — resolution required"),
    ),
    tag = "merge"
)]
pub async fn accept_merge_proposal(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<AcceptMergeProposalRequest>,
) -> Result<Json<MergeProposal>, AppError> {
    let db = db.lock().await;

    let pid = RecordId::new("person_merge_proposal", id.as_str());
    let proposal: Option<MergeProposal> = db.select(pid.clone()).await?;
    let proposal = proposal.ok_or(AppError::NotFound)?;

    if proposal.status != "pending" {
        return Err(AppError::BadRequest(format!(
            "Proposal is already {}", proposal.status
        )));
    }

    let to_proxy: Option<PersonProxy> = db.select(proposal.to_proxy_id.clone()).await?;
    let to_proxy = to_proxy.ok_or(AppError::NotFound)?;
    can_write_to_proxy(&to_proxy, &auth, &db).await?;

    let from_proxy: Option<PersonProxy> = db.select(proposal.from_proxy_id.clone()).await?;
    let from_proxy = from_proxy.ok_or(AppError::NotFound)?;

    let canonical_a: Option<Person> = db.select(from_proxy.person_id.clone()).await?;
    let canonical_a = canonical_a.ok_or(AppError::NotFound)?;
    let canonical_b: Option<Person> = db.select(to_proxy.person_id.clone()).await?;
    let canonical_b = canonical_b.ok_or(AppError::NotFound)?;

    // Detect conflicting hard fields.
    let mut conflicts: Vec<String> = Vec::new();
    macro_rules! check_conflict {
        ($field:ident) => {
            if canonical_a.$field.is_some()
                && canonical_b.$field.is_some()
                && canonical_a.$field != canonical_b.$field
            {
                conflicts.push(stringify!($field).to_string());
            }
        };
    }
    check_conflict!(date_of_birth);
    check_conflict!(place_of_birth);
    check_conflict!(date_of_death);
    check_conflict!(place_of_death);
    check_conflict!(birth_cert_authority);
    check_conflict!(birth_cert_number);

    if !conflicts.is_empty() && payload.resolution.is_none() {
        return Err(AppError::Conflict(serde_json::json!({
            "error": "Canonical conflict — supply resolution to proceed",
            "conflicts": conflicts
        })));
    }

    // Winning canonical: canonical_a (from_proxy.person_id). Apply resolution.
    let mut update = serde_json::Map::new();
    if let Some(res) = &payload.resolution {
        if let Some(v) = &res.family_name { update.insert("family_name".into(), v.clone().into()); }
        if let Some(v) = &res.first_name  { update.insert("first_name".into(),  v.clone().into()); }
        if let Some(v) = &res.middle_name { update.insert("middle_name".into(), v.clone().into()); }
        if let Some(v) = &res.sex         { update.insert("sex".into(),         v.clone().into()); }
        if let Some(v) = &res.date_of_birth   { update.insert("date_of_birth".into(),   v.clone().into()); }
        if let Some(v) = &res.place_of_birth  { update.insert("place_of_birth".into(),  v.clone().into()); }
        if let Some(v) = &res.date_of_death   { update.insert("date_of_death".into(),   v.clone().into()); }
        if let Some(v) = &res.place_of_death  { update.insert("place_of_death".into(),  v.clone().into()); }
        if let Some(v) = &res.birth_cert_authority { update.insert("birth_cert_authority".into(), v.clone().into()); }
        if let Some(v) = &res.birth_cert_number    { update.insert("birth_cert_number".into(),    v.clone().into()); }
        if !update.is_empty() {
            let _: Option<Person> = db
                .update(canonical_a.id.clone())
                .merge(serde_json::Value::Object(update))
                .await?;
        }
    }

    // Re-point all of canonical_b's proxies to canonical_a.
    db.query("UPDATE person_proxy SET person_id = $a WHERE person_id = $b")
        .bind(("a", canonical_a.id.clone()))
        .bind(("b", canonical_b.id.clone()))
        .await?;

    // Delete canonical_b.
    let _: Option<Person> = db.delete(canonical_b.id).await?;

    let responder = if auth.username.is_empty() {
        to_proxy.created_by.unwrap_or_default()
    } else {
        auth.username.clone()
    };

    let updated: Vec<MergeProposal> = db
        .query(
            "UPDATE $pid SET status = 'accepted', responded_by = $user, \
             responded_at = <string>time::now(), resolution = $res",
        )
        .bind(("pid",  pid))
        .bind(("user", responder))
        .bind(("res",  serde_json::to_value(&payload.resolution).ok()))
        .await?
        .take(0)?;

    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}

#[utoipa::path(
    patch,
    path = "/api/merge-proposals/{id}/reject",
    params(("id" = String, Path, description = "Proposal ULID")),
    responses(
        (status = 200, description = "Rejected", body = MergeProposal),
        (status = 404, body = ErrorResponse),
    ),
    tag = "merge"
)]
pub async fn reject_merge_proposal(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
) -> Result<Json<MergeProposal>, AppError> {
    let db = db.lock().await;

    let pid = RecordId::new("person_merge_proposal", id.as_str());
    let proposal: Option<MergeProposal> = db.select(pid.clone()).await?;
    let proposal = proposal.ok_or(AppError::NotFound)?;

    if proposal.status != "pending" {
        return Err(AppError::BadRequest(format!("Proposal is already {}", proposal.status)));
    }

    let to_proxy: Option<PersonProxy> = db.select(proposal.to_proxy_id.clone()).await?;
    let to_proxy = to_proxy.ok_or(AppError::NotFound)?;
    can_write_to_proxy(&to_proxy, &auth, &db).await?;

    let responder = if auth.username.is_empty() {
        to_proxy.created_by.unwrap_or_default()
    } else {
        auth.username.clone()
    };

    let updated: Vec<MergeProposal> = db
        .query(
            "UPDATE $pid SET status = 'rejected', responded_by = $user, \
             responded_at = <string>time::now()",
        )
        .bind(("pid",  pid))
        .bind(("user", responder))
        .await?
        .take(0)?;

    updated.into_iter().next().map(Json).ok_or(AppError::NotFound)
}
