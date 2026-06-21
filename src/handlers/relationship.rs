use std::{future::Future, pin::Pin};

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};

use crate::{
    auth::ClannAuth,
    db::{Db, DbConn},
    error::{AppError, ErrorResponse},
    handlers::person::{PersonFilter, can_write_to_proxy, fetch_proxy_and_canonical},
    models::{
        person::PersonProxy,
        relationship::{
            AddRelationshipRequest, FamilyTreeNode, ParentInfo, Pedigree, PersonProxyNode,
            PROXY_NODE_IN, PROXY_NODE_OUT, RelationshipType, RelationshipsResponse, SiblingInfo,
            SiblingType, SpouseInfo, UpdateRelationshipRequest, UpdateSpouseDatesRequest,
        },
    },
};

fn parse_record_id(s: &str) -> Result<RecordId, AppError> {
    let (tb, id) = s
        .split_once(':')
        .ok_or_else(|| AppError::BadRequest(format!("Expected 'table:id', got: {s}")))?;
    Ok(RecordId::new(tb, id))
}

fn record_id_key(id: &RecordId) -> String {
    match &id.key {
        RecordIdKey::String(k) => k.clone(),
        other => format!("{other:?}"),
    }
}

#[utoipa::path(
    post,
    path = "/api/persons/{id}/relationships",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    request_body = AddRelationshipRequest,
    responses(
        (status = 201, description = "Relationship added"),
        (status = 400, description = "Bad request or cross-tree relationship attempt", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn add_relationship(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path(id): Path<String>,
    Json(payload): Json<AddRelationshipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let (proxy, _) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let from = RecordId::new("person_proxy", id.as_str());
    let to = parse_record_id(&payload.related_id)?;

    // Validate the target proxy is in the same tree.
    if to.table.to_string() != "person_proxy" {
        return Err(AppError::BadRequest(
            "related_id must be a person_proxy record ID".to_string(),
        ));
    }
    let to_proxy: Option<PersonProxy> = db.select(to.clone()).await?;
    let to_proxy = to_proxy.ok_or(AppError::NotFound)?;
    if to_proxy.tree != proxy.tree {
        return Err(AppError::BadRequest(
            "Relationships must be within the same tree. Use a merge proposal for cross-tree connections.".to_string(),
        ));
    }

    match payload.rel_type {
        RelationshipType::Father => {
            let ped = payload.pedigree.as_db_str();
            db.query("RELATE $from->has_father->$to CONTENT { pedigree: $ped }")
                .bind(("from", from))
                .bind(("to", to))
                .bind(("ped", ped))
                .await?;
        }
        RelationshipType::Mother => {
            let ped = payload.pedigree.as_db_str();
            db.query("RELATE $from->has_mother->$to CONTENT { pedigree: $ped }")
                .bind(("from", from))
                .bind(("to", to))
                .bind(("ped", ped))
                .await?;
        }
        RelationshipType::Sibling => {
            let sibling_type = payload.sibling_type.ok_or_else(|| {
                AppError::BadRequest("sibling_type required for Sibling relationship".to_string())
            })?;
            let st = match sibling_type {
                SiblingType::Brother => "Brother",
                SiblingType::Sister => "Sister",
            };
            let ped = payload.pedigree.as_db_str();
            let vpid_opt = payload.via_parent_id;
            let q = if vpid_opt.is_some() {
                "RELATE $from->has_sibling->$to CONTENT { sibling_type: $st, pedigree: $ped, via_parent_id: $vpid }"
            } else {
                "RELATE $from->has_sibling->$to CONTENT { sibling_type: $st, pedigree: $ped }"
            };
            let mut qb = db.query(q)
                .bind(("from", from))
                .bind(("to", to))
                .bind(("st", st))
                .bind(("ped", ped));
            if let Some(v) = vpid_opt {
                qb = qb.bind(("vpid", v));
            }
            qb.await?;
        }
        RelationshipType::Spouse => {
            let sp_from = payload.spouse_from.as_deref().unwrap_or_default().to_string();
            let sp_to = payload.spouse_to.as_deref().unwrap_or_default().to_string();
            db.query(
                "RELATE $from->has_spouse->$to CONTENT { spouse_from: $sf, spouse_to: $st }; \
                 RELATE $to->has_spouse->$from CONTENT { spouse_from: $sf, spouse_to: $st };",
            )
            .bind(("from", from))
            .bind(("to", to))
            .bind(("sf", sp_from))
            .bind(("st", sp_to))
            .await?;
        }
    }

    Ok(StatusCode::CREATED)
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/relationships",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    responses(
        (status = 200, description = "Grouped relationships", body = RelationshipsResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_relationships(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(_filter): Query<PersonFilter>,
) -> Result<Json<RelationshipsResponse>, AppError> {
    let db = db.lock().await;

    let proxy: Option<PersonProxy> = db.select(("person_proxy", id.as_str())).await?;
    proxy.as_ref().ok_or(AppError::NotFound)?;

    let proxy_id = RecordId::new("person_proxy", id.as_str());

    let father = fetch_parent_edges(&*db, &proxy_id, "has_father").await?;
    let mother = fetch_parent_edges(&*db, &proxy_id, "has_mother").await?;
    let siblings = fetch_sibling_edges(&*db, &proxy_id).await?;
    let spouse = fetch_spouses(&*db, &proxy_id).await?;

    Ok(Json(RelationshipsResponse { father, mother, siblings, spouse }))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Proxy ULID"),
        ("rel_type" = String, Path, description = "`has_father`, `has_mother`, `has_sibling`, or `has_spouse`"),
        ("related_id" = String, Path, description = "Full related proxy ID, e.g. `person_proxy:01jd4a8xyz`"),
    ),
    responses(
        (status = 204, description = "Relationship deleted"),
        (status = 400, description = "Invalid relationship type", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn delete_relationship(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, rel_type, related_id)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    let valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let db = db.lock().await;

    let (proxy, _) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let from = RecordId::new("person_proxy", id.as_str());
    let to = parse_record_id(&related_id)?;

    if matches!(valid, RelationshipType::Spouse) {
        let q = format!(
            "DELETE {rel_type} WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)"
        );
        db.query(q).bind(("from", from)).bind(("to", to)).await?;
    } else {
        let query = format!("DELETE {} WHERE in = $from AND out = $to", rel_type);
        db.query(query).bind(("from", from)).bind(("to", to)).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/persons/{id}/family-tree",
    params(
        ("id" = String, Path, description = "Proxy ULID (without the `person_proxy:` prefix)")
    ),
    responses(
        (status = 200, description = "Family tree up to 2 generations deep"),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_family_tree(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<FamilyTreeNode>, AppError> {
    let root_node = {
        let conn = db.lock().await;
        let proxy: Option<PersonProxy> = conn.select(("person_proxy", id.as_str())).await?;
        let proxy = proxy.ok_or(AppError::NotFound)?;
        let canonical: Option<crate::models::person::Person> = conn.select(proxy.person_id.clone()).await?;
        let canonical = canonical.ok_or(AppError::NotFound)?;
        PersonProxyNode {
            id: proxy.id,
            person_id: proxy.person_id,
            tree: proxy.tree,
            family_name: proxy.preferred_family_name
                .clone()
                .unwrap_or_else(|| canonical.family_name.clone()),
            first_name: proxy.preferred_first_name
                .clone()
                .unwrap_or_else(|| canonical.first_name.clone()),
            middle_name: proxy.preferred_middle_name
                .clone()
                .or_else(|| canonical.middle_name.clone()),
            sex: canonical.sex,
            date_of_birth: canonical.date_of_birth,
            place_of_birth: canonical.place_of_birth,
            date_of_death: canonical.date_of_death,
            place_of_death: canonical.place_of_death,
            biography: proxy.biography,
            image_path: proxy.image_path,
            is_private: proxy.is_private,
            created_by: proxy.created_by,
        }
    };

    let node = build_tree_node(db, root_node, 2, true).await?;
    Ok(Json(node))
}

fn build_tree_node(
    db: Db,
    node: PersonProxyNode,
    depth: u8,
    include_root_extras: bool,
) -> Pin<Box<dyn Future<Output = Result<FamilyTreeNode, AppError>> + Send>> {
    Box::pin(async move {
        let (children, spouse, siblings) = if include_root_extras {
            let child_nodes = {
                let conn = db.lock().await;
                fetch_children(&*conn, &node.id).await?
            };
            let spouse_nodes = {
                let conn = db.lock().await;
                fetch_spouses(&*conn, &node.id).await?
            };
            let sibling_nodes = {
                let conn = db.lock().await;
                fetch_sibling_edges(&*conn, &node.id).await?
            };

            let mut children = Vec::new();
            for (child, ped) in child_nodes {
                let mut cn = build_tree_node(db.clone(), child, 0, false).await?;
                cn.pedigree = Some(ped);
                children.push(cn);
            }
            let mut spouses = Vec::new();
            for s in spouse_nodes {
                spouses.push(build_tree_node(db.clone(), s.person, 0, false).await?);
            }
            let mut sibs = Vec::new();
            for s in sibling_nodes {
                let mut sn = build_tree_node(db.clone(), s.person, 0, false).await?;
                sn.pedigree = Some(s.pedigree);
                sn.via_parent_id = s.via_parent_id;
                sibs.push(sn);
            }
            (children, spouses, sibs)
        } else {
            (vec![], vec![], vec![])
        };

        if depth == 0 {
            let mut n = FamilyTreeNode::from_proxy_node(node);
            n.children = children;
            n.spouse = spouse;
            n.siblings = siblings;
            return Ok(n);
        }

        let father_edges = {
            let conn = db.lock().await;
            fetch_parent_edges(&*conn, &node.id, "has_father").await?
        };
        let mother_edges = {
            let conn = db.lock().await;
            fetch_parent_edges(&*conn, &node.id, "has_mother").await?
        };

        let mut father = Vec::new();
        for pi in father_edges {
            let mut fn_node = build_tree_node(db.clone(), pi.person, depth - 1, false).await?;
            fn_node.pedigree = Some(pi.pedigree);
            father.push(fn_node);
        }

        let mut mother = Vec::new();
        for pi in mother_edges {
            let mut mn_node = build_tree_node(db.clone(), pi.person, depth - 1, false).await?;
            mn_node.pedigree = Some(pi.pedigree);
            mother.push(mn_node);
        }

        let mut result = FamilyTreeNode::from_proxy_node(node);
        result.father = father;
        result.mother = mother;
        result.children = children;
        result.spouse = spouse;
        result.siblings = siblings;
        Ok(result)
    })
}

/// Fetch sibling `PersonProxyNode`s with pedigree and via_parent_id for the family-tree endpoint.
pub async fn fetch_siblings(
    db: &DbConn,
    proxy_id: &RecordId,
) -> Result<Vec<(PersonProxyNode, Pedigree, Option<String>)>, AppError> {
    fetch_sibling_edges(db, proxy_id)
        .await
        .map(|v| v.into_iter().map(|s| (s.person, s.pedigree, s.via_parent_id)).collect())
}

async fn fetch_sibling_edges(db: &DbConn, proxy_id: &RecordId) -> Result<Vec<SiblingInfo>, AppError> {
    #[derive(serde::Deserialize, SurrealValue)]
    struct SiblingEdgeRow {
        person: PersonProxyNode,
        pedigree: Option<String>,
        #[serde(default)]
        via_parent_id: Option<String>,
    }

    let q_out = format!(
        "SELECT {} AS person, pedigree, via_parent_id FROM has_sibling WHERE in = $id",
        PROXY_NODE_OUT
    );
    let q_in = format!(
        "SELECT {} AS person, pedigree, via_parent_id FROM has_sibling WHERE out = $id",
        PROXY_NODE_IN
    );

    let mut out_res = db.query(q_out).bind(("id", proxy_id.clone())).await?;
    let mut in_res  = db.query(q_in).bind(("id", proxy_id.clone())).await?;
    let out_rows: Vec<SiblingEdgeRow> = out_res.take(0)?;
    let in_rows:  Vec<SiblingEdgeRow> = in_res.take(0)?;

    let mut seen = std::collections::HashSet::new();
    let siblings = out_rows.into_iter().chain(in_rows)
        .filter(|r| seen.insert(record_id_key(&r.person.id)))
        .map(|r| SiblingInfo {
            pedigree: r.pedigree.as_deref().map(Pedigree::from_db_str).unwrap_or(Pedigree::Birth),
            via_parent_id: r.via_parent_id,
            person: r.person,
        })
        .collect();
    Ok(siblings)
}

async fn fetch_parent_edges(
    db: &DbConn,
    proxy_id: &RecordId,
    rel: &str,
) -> Result<Vec<ParentInfo>, AppError> {
    #[derive(serde::Deserialize, SurrealValue)]
    struct ParentEdgeRow {
        person: PersonProxyNode,
        pedigree: Option<String>,
    }

    let query = format!(
        "SELECT {} AS person, pedigree FROM {} WHERE in = $id",
        PROXY_NODE_OUT, rel
    );

    let mut res = db.query(query).bind(("id", proxy_id.clone())).await?;
    let rows: Vec<ParentEdgeRow> = res.take(0)?;
    Ok(rows.into_iter().map(|r| ParentInfo {
        person: r.person,
        pedigree: r.pedigree.as_deref().map(Pedigree::from_db_str).unwrap_or(Pedigree::Birth),
    }).collect())
}

async fn fetch_children(
    db: &DbConn,
    parent_id: &RecordId,
) -> Result<Vec<(PersonProxyNode, Pedigree)>, AppError> {
    #[derive(serde::Deserialize, SurrealValue)]
    struct ChildEdgeRow {
        person: PersonProxyNode,
        pedigree: Option<String>,
    }

    let q_father = format!(
        "SELECT {} AS person, pedigree FROM has_father WHERE out = $id",
        PROXY_NODE_IN
    );
    let q_mother = format!(
        "SELECT {} AS person, pedigree FROM has_mother WHERE out = $id",
        PROXY_NODE_IN
    );

    let mut father_res = db.query(q_father).bind(("id", parent_id.clone())).await?;
    let mut mother_res = db.query(q_mother).bind(("id", parent_id.clone())).await?;

    let mut all: Vec<ChildEdgeRow> = father_res
        .take::<Vec<ChildEdgeRow>>(0)?
        .into_iter()
        .chain(mother_res.take::<Vec<ChildEdgeRow>>(0)?.into_iter())
        .collect();

    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(record_id_key(&r.person.id)));

    Ok(all.into_iter().map(|r| (
        r.person,
        r.pedigree.as_deref().map(Pedigree::from_db_str).unwrap_or(Pedigree::Birth),
    )).collect())
}

async fn fetch_spouses(db: &DbConn, proxy_id: &RecordId) -> Result<Vec<SpouseInfo>, AppError> {
    #[derive(serde::Deserialize, SurrealValue)]
    struct SpouseEdgeRow {
        person: PersonProxyNode,
        spouse_from: Option<String>,
        spouse_to: Option<String>,
    }

    let query = format!(
        "SELECT {} AS person, spouse_from, spouse_to FROM has_spouse WHERE in = $id",
        PROXY_NODE_OUT
    );

    let mut res = db.query(query).bind(("id", proxy_id.clone())).await?;
    let rows: Vec<SpouseEdgeRow> = res.take(0)?;
    Ok(rows.into_iter().map(|r| SpouseInfo {
        person: r.person,
        spouse_from: r.spouse_from,
        spouse_to: r.spouse_to,
    }).collect())
}

#[utoipa::path(
    patch,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Proxy ULID"),
        ("rel_type" = String, Path, description = "`has_father`, `has_mother`, or `has_sibling`"),
        ("related_id" = String, Path, description = "Full related proxy ID"),
    ),
    request_body = UpdateRelationshipRequest,
    responses(
        (status = 204, description = "Pedigree updated"),
        (status = 400, description = "Invalid request", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn update_relationship_pedigree(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, rel_type, related_id)): Path<(String, String, String)>,
    Json(payload): Json<UpdateRelationshipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let (proxy, _) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let new_ped = payload.pedigree.as_db_str();

    match valid {
        RelationshipType::Father | RelationshipType::Mother => {
            let q = format!(
                "UPDATE {} SET pedigree = $ped WHERE in = $from AND out = $to",
                rel_type
            );
            let from = RecordId::new("person_proxy", id.as_str());
            let to = parse_record_id(&related_id)?;
            db.query(q)
                .bind(("ped", new_ped))
                .bind(("from", from.clone()))
                .bind(("to", to))
                .await?;

            let sib_ped = match payload.pedigree {
                Pedigree::Adopted => "adopted",
                _ => "birth",
            };
            db.query(
                "UPDATE has_sibling SET pedigree = $sib_ped \
                 WHERE (in = $person OR out = $person) AND via_parent_id = $vpid",
            )
            .bind(("sib_ped", sib_ped))
            .bind(("person", from))
            .bind(("vpid", related_id))
            .await?;
        }
        RelationshipType::Sibling => {
            let from = RecordId::new("person_proxy", id.as_str());
            let to = parse_record_id(&related_id)?;
            db.query(
                "UPDATE has_sibling SET pedigree = $ped, via_parent_id = $vpid \
                 WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)",
            )
            .bind(("ped", new_ped))
            .bind(("vpid", payload.via_parent_id))
            .bind(("from", from))
            .bind(("to", to))
            .await?;
        }
        _ => {
            return Err(AppError::BadRequest(
                "Pedigree update not supported for this relationship type".to_string(),
            ));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/persons/{id}/spouse-dates/{related_id}",
    params(
        ("id" = String, Path, description = "Proxy ULID"),
        ("related_id" = String, Path, description = "Full related proxy ID"),
    ),
    request_body = UpdateSpouseDatesRequest,
    responses(
        (status = 204, description = "Spouse dates updated"),
        (status = 400, description = "Bad request", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn update_spouse_dates(
    State(db): State<Db>,
    Extension(auth): Extension<ClannAuth>,
    Path((id, related_id)): Path<(String, String)>,
    Json(payload): Json<UpdateSpouseDatesRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    let (proxy, _) = fetch_proxy_and_canonical(&db, &id).await?;
    can_write_to_proxy(&proxy, &auth, &db).await?;

    let from = RecordId::new("person_proxy", id.as_str());
    let to = parse_record_id(&related_id)?;
    db.query(
        "UPDATE has_spouse SET spouse_from = $sf, spouse_to = $st \
         WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)",
    )
    .bind(("from", from))
    .bind(("to", to))
    .bind(("sf", payload.spouse_from))
    .bind(("st", payload.spouse_to))
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
