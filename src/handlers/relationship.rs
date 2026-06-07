use std::{future::Future, pin::Pin};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use surrealdb::types::{RecordId, RecordIdKey, SurrealValue};

use crate::{
    db::{Db, DbConn},
    error::{AppError, ErrorResponse},
    handlers::person::{PersonFilter, check_ownership},
    models::{
        person::Person,
        relationship::{
            AddRelationshipRequest, FamilyTreeNode, ParentInfo, Pedigree, RelationshipType,
            RelationshipsResponse, SiblingInfo, SiblingType, SpouseInfo, UpdateRelationshipRequest,
            UpdateSpouseDatesRequest,
        },
    },
};

/// Parse a "table:id" string into a SurrealDB `RecordId`.
/// Binding RecordId directly avoids needing `type::thing()` in SurrealQL,
/// which is unsupported in the embedded engine.
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
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    request_body = AddRelationshipRequest,
    responses(
        (status = 201, description = "Relationship added"),
        (status = 400, description = "Bad request — e.g. missing sibling_type", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn add_relationship(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<AddRelationshipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    if filter.created_by.is_some() {
        let person: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&person.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&payload.related_id)?;

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
            let vpid = payload.via_parent_id;
            db.query(
                "RELATE $from->has_sibling->$to CONTENT { sibling_type: $st, pedigree: $ped, via_parent_id: $vpid }"
            )
                .bind(("from", from))
                .bind(("to", to))
                .bind(("st", st))
                .bind(("ped", ped))
                .bind(("vpid", vpid))
                .await?;
        }
        RelationshipType::Spouse => {
            // Both directions in a single query — no await gap between them.
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
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (status = 200, description = "Grouped relationships", body = RelationshipsResponse),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_relationships(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<RelationshipsResponse>, AppError> {
    let db = db.lock().await;

    if filter.created_by.is_some() {
        let person: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&person.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    let person_id = RecordId::new("person", id.as_str());

    let father = fetch_parent_edges(&*db, &person_id, "has_father").await?;
    let mother = fetch_parent_edges(&*db, &person_id, "has_mother").await?;

    let siblings = fetch_sibling_edges(&*db, &person_id).await?;

    // Spouses: bidirectional edges exist, so forward query is sufficient.
    let spouse = fetch_spouses(&*db, &person_id).await?;

    Ok(Json(RelationshipsResponse { father, mother, siblings, spouse }))
}

#[utoipa::path(
    delete,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("rel_type" = String, Path, description = "Relationship table: `has_father`, `has_mother`, `has_sibling`, or `has_spouse`"),
        ("related_id" = String, Path, description = "Full related record ID, e.g. `person:01jd4a8xyz`"),
    ),
    responses(
        (status = 204, description = "Relationship deleted"),
        (status = 400, description = "Invalid relationship type", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn delete_relationship(
    State(db): State<Db>,
    Path((id, rel_type, related_id)): Path<(String, String, String)>,
    Query(filter): Query<PersonFilter>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    if filter.created_by.is_some() {
        let person: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&person.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    // Validate rel_type against the whitelist before embedding it in the query string.
    let valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let from = RecordId::new("person", id.as_str());
    let to = parse_record_id(&related_id)?;

    if matches!(valid, RelationshipType::Spouse) {
        // Spouse edges are bidirectional — delete both directions.
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
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)")
    ),
    responses(
        (
            status = 200,
            description = "Family tree up to 2 generations deep. \
                           Each node: `{ id, family_name, first_name, \
                           father: [node…], mother: [node…], children: [node…], spouse: [node…] }`"
        ),
        (status = 404, description = "Person not found", body = ErrorResponse),
    ),
    tag = "relationships"
)]
pub async fn get_family_tree(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(filter): Query<PersonFilter>,
) -> Result<Json<FamilyTreeNode>, AppError> {
    let person = {
        let conn = db.lock().await;
        let p: Option<Person> = conn.select(("person", id.as_str())).await?;
        p
    };
    let person = person.ok_or(AppError::NotFound)?;
    check_ownership(&person, &filter.created_by)?;

    let node = build_tree_node(db, person, 2, true).await?;
    Ok(Json(node))
}

/// Recursively build a `FamilyTreeNode` up to `depth` generations.
/// `include_root_extras` is true only for the root call — fetches children and spouses.
/// The `Db` (Arc<Mutex<DbConn>>) is locked for each batch of queries and released
/// before any recursive call, preventing deadlocks.
fn build_tree_node(
    db: Db,
    person: Person,
    depth: u8,
    include_root_extras: bool,
) -> Pin<Box<dyn Future<Output = Result<FamilyTreeNode, AppError>> + Send>> {
    Box::pin(async move {
        let (children, spouse, siblings) = if include_root_extras {
            let child_persons = {
                let conn = db.lock().await;
                fetch_children(&*conn, &person.id).await?
            };
            let spouse_persons = {
                let conn = db.lock().await;
                fetch_spouses(&*conn, &person.id).await?
            };
            let sibling_persons = {
                let conn = db.lock().await;
                fetch_siblings(&*conn, &person.id).await?
            };

            let mut child_nodes = Vec::new();
            for (p, ped) in child_persons {
                let mut node = build_tree_node(db.clone(), p, 0, false).await?;
                node.pedigree = Some(ped);
                child_nodes.push(node);
            }
            let mut spouse_nodes = Vec::new();
            for s in spouse_persons {
                spouse_nodes.push(build_tree_node(db.clone(), s.person, 0, false).await?);
            }
            let mut sibling_nodes = Vec::new();
            for (p, ped, vpid) in sibling_persons {
                let mut node = build_tree_node(db.clone(), p, 0, false).await?;
                node.pedigree = Some(ped);
                node.via_parent_id = vpid;
                sibling_nodes.push(node);
            }
            (child_nodes, spouse_nodes, sibling_nodes)
        } else {
            (vec![], vec![], vec![])
        };

        if depth == 0 {
            return Ok(FamilyTreeNode {
                id: person.id,
                family_name: person.family_name,
                first_name: person.first_name,
                sex: Some(person.sex),
                date_of_birth: person.date_of_birth,
                place_of_birth: person.place_of_birth,
                biography: person.biography,
                image_path: person.image_path,
                pedigree: None,
                via_parent_id: None,
                father: vec![],
                mother: vec![],
                children,
                spouse,
                siblings,
            });
        }

        let father_edges = {
            let conn = db.lock().await;
            fetch_parent_edges(&*conn, &person.id, "has_father").await?
        };
        let mother_edges = {
            let conn = db.lock().await;
            fetch_parent_edges(&*conn, &person.id, "has_mother").await?
        };

        let mut father = Vec::new();
        for pi in father_edges {
            let mut node = build_tree_node(db.clone(), pi.person, depth - 1, false).await?;
            node.pedigree = Some(pi.pedigree);
            father.push(node);
        }

        let mut mother = Vec::new();
        for pi in mother_edges {
            let mut node = build_tree_node(db.clone(), pi.person, depth - 1, false).await?;
            node.pedigree = Some(pi.pedigree);
            mother.push(node);
        }

        Ok(FamilyTreeNode {
            id: person.id,
            family_name: person.family_name,
            first_name: person.first_name,
            sex: Some(person.sex),
            date_of_birth: person.date_of_birth,
            place_of_birth: person.place_of_birth,
            biography: person.biography,
            image_path: person.image_path,
            pedigree: None,
            via_parent_id: None,
            father,
            mother,
            children,
            spouse,
            siblings,
        })
    })
}

/// Fetch siblings with pedigree and via_parent_id for the family-tree endpoint.
pub async fn fetch_siblings(db: &DbConn, person_id: &RecordId) -> Result<Vec<(Person, Pedigree, Option<String>)>, AppError> {
    fetch_sibling_edges(db, person_id)
        .await
        .map(|v| v.into_iter().map(|s| (s.person, s.pedigree, s.via_parent_id)).collect())
}

/// Fetch siblings with pedigree and via_parent_id from the edge.
/// Queries both outgoing and incoming edges to cover the one-directional `has_sibling` table.
async fn fetch_sibling_edges(db: &DbConn, person_id: &RecordId) -> Result<Vec<SiblingInfo>, AppError> {
    #[derive(serde::Deserialize, surrealdb::types::SurrealValue)]
    struct SiblingEdgeRow {
        person: Person,
        pedigree: Option<String>,
        via_parent_id: Option<String>,
    }

    // Outgoing: current person is `in`, sibling is `out`
    let q_out = "SELECT \
        { id: out.id, family_name: out.family_name, first_name: out.first_name, \
          middle_name: out.middle_name, sex: out.sex, date_of_birth: out.date_of_birth, \
          place_of_birth: out.place_of_birth, date_of_death: out.date_of_death, \
          place_of_death: out.place_of_death, image_path: out.image_path, \
          nickname: out.nickname, username: out.username, email: out.email, \
          verified: out.verified, biography: out.biography, \
          created_by: out.created_by, trees: out.trees } AS person, \
        pedigree, via_parent_id FROM has_sibling WHERE in = $id";

    // Incoming: current person is `out`, sibling is `in`
    let q_in = "SELECT \
        { id: in.id, family_name: in.family_name, first_name: in.first_name, \
          middle_name: in.middle_name, sex: in.sex, date_of_birth: in.date_of_birth, \
          place_of_birth: in.place_of_birth, date_of_death: in.date_of_death, \
          place_of_death: in.place_of_death, image_path: in.image_path, \
          nickname: in.nickname, username: in.username, email: in.email, \
          verified: in.verified, biography: in.biography, \
          created_by: in.created_by, trees: in.trees } AS person, \
        pedigree, via_parent_id FROM has_sibling WHERE out = $id";

    let mut out_res = db.query(q_out).bind(("id", person_id.clone())).await?;
    let mut in_res  = db.query(q_in).bind(("id", person_id.clone())).await?;
    let out_rows: Vec<SiblingEdgeRow> = out_res.take(0)?;
    let in_rows: Vec<SiblingEdgeRow>  = in_res.take(0)?;

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

/// Fetch parents with pedigree from a `has_father` or `has_mother` edge table.
/// Edges that pre-date the `pedigree` field default to `Pedigree::Birth`.
async fn fetch_parent_edges(db: &DbConn, person_id: &RecordId, rel: &str) -> Result<Vec<ParentInfo>, AppError> {
    // Use Option<String> for pedigree — SurrealValue enum deserialization is unreliable;
    // reading as a plain string and converting manually matches how sibling_type/spouse_from work.
    #[derive(serde::Deserialize, surrealdb::types::SurrealValue)]
    struct ParentEdgeRow {
        person: Person,
        pedigree: Option<String>,
    }

    let query = format!(
        "SELECT {{ \
            id: out.id, family_name: out.family_name, first_name: out.first_name, \
            middle_name: out.middle_name, sex: out.sex, date_of_birth: out.date_of_birth, \
            place_of_birth: out.place_of_birth, date_of_death: out.date_of_death, \
            place_of_death: out.place_of_death, image_path: out.image_path, \
            nickname: out.nickname, username: out.username, email: out.email, \
            verified: out.verified, biography: out.biography, \
            created_by: out.created_by, trees: out.trees }} AS person, \
         pedigree \
         FROM {} WHERE in = $id",
        rel
    );

    let mut res = db.query(query).bind(("id", person_id.clone())).await?;
    let rows: Vec<ParentEdgeRow> = res.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| ParentInfo {
            person: r.person,
            pedigree: r.pedigree.as_deref().map(Pedigree::from_db_str).unwrap_or(Pedigree::Birth),
        })
        .collect())
}

/// Fetch all children of a parent with the pedigree of their relationship to this parent.
/// The `pedigree` on each returned item reflects the child→parent edge for this specific parent.
async fn fetch_children(db: &DbConn, parent_id: &RecordId) -> Result<Vec<(Person, Pedigree)>, AppError> {
    #[derive(serde::Deserialize, surrealdb::types::SurrealValue)]
    struct ChildEdgeRow {
        person: Person,
        pedigree: Option<String>,
    }

    // Children stored as child→has_father→parent (or has_mother); `in` is child, `out` is parent.
    let child_fields = "{ id: in.id, family_name: in.family_name, first_name: in.first_name, \
        middle_name: in.middle_name, sex: in.sex, date_of_birth: in.date_of_birth, \
        place_of_birth: in.place_of_birth, date_of_death: in.date_of_death, \
        place_of_death: in.place_of_death, image_path: in.image_path, \
        nickname: in.nickname, username: in.username, email: in.email, \
        verified: in.verified, biography: in.biography, \
        created_by: in.created_by, trees: in.trees } AS person, pedigree";

    let q_father = format!("SELECT {} FROM has_father WHERE out = $id", child_fields);
    let q_mother = format!("SELECT {} FROM has_mother WHERE out = $id", child_fields);

    let mut father_res = db.query(q_father).bind(("id", parent_id.clone())).await?;
    let mut mother_res = db.query(q_mother).bind(("id", parent_id.clone())).await?;

    let mut all: Vec<ChildEdgeRow> = father_res.take::<Vec<ChildEdgeRow>>(0)?.into_iter()
        .chain(mother_res.take::<Vec<ChildEdgeRow>>(0)?.into_iter())
        .collect();

    // Deduplicate by ULID key (a child may have both parents in this tree).
    let mut seen = std::collections::HashSet::new();
    all.retain(|r| seen.insert(record_id_key(&r.person.id)));

    Ok(all.into_iter().map(|r| (
        r.person,
        r.pedigree.as_deref().map(Pedigree::from_db_str).unwrap_or(Pedigree::Birth),
    )).collect())
}

/// Fetch spouses with edge date attributes.
/// Because `has_spouse` edges are added bidirectionally, a forward query is sufficient.
async fn fetch_spouses(db: &DbConn, person_id: &RecordId) -> Result<Vec<SpouseInfo>, AppError> {
    #[derive(serde::Deserialize, surrealdb::types::SurrealValue)]
    struct SpouseEdgeRow {
        person: Person,
        spouse_from: Option<String>,
        spouse_to: Option<String>,
    }

    // Build a sub-object for the person to avoid field name collisions with the edge's own `id`.
    let query = "SELECT \
        { id: out.id, family_name: out.family_name, first_name: out.first_name, \
          middle_name: out.middle_name, sex: out.sex, date_of_birth: out.date_of_birth, \
          place_of_birth: out.place_of_birth, date_of_death: out.date_of_death, \
          place_of_death: out.place_of_death, image_path: out.image_path, \
          nickname: out.nickname, username: out.username, email: out.email, \
          verified: out.verified, biography: out.biography, \
          created_by: out.created_by, trees: out.trees } AS person, \
        spouse_from, spouse_to \
        FROM has_spouse WHERE in = $id";

    let mut res = db.query(query).bind(("id", person_id.clone())).await?;
    let rows: Vec<SpouseEdgeRow> = res.take(0)?;
    Ok(rows
        .into_iter()
        .map(|r| SpouseInfo { person: r.person, spouse_from: r.spouse_from, spouse_to: r.spouse_to })
        .collect())
}

#[utoipa::path(
    patch,
    path = "/api/persons/{id}/relationships/{rel_type}/{related_id}",
    params(
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("rel_type" = String, Path, description = "Relationship table: `has_father`, `has_mother`, or `has_sibling`"),
        ("related_id" = String, Path, description = "Full related record ID, e.g. `person:01jd4a8xyz`"),
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
    Path((id, rel_type, related_id)): Path<(String, String, String)>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<UpdateRelationshipRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    if filter.created_by.is_some() {
        let person: Option<crate::models::person::Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&person.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }

    let valid = RelationshipType::from_str(&rel_type)
        .ok_or_else(|| AppError::InvalidRelType(format!("Unknown relationship type: {}", rel_type)))?;

    let new_ped = payload.pedigree.as_db_str();

    match valid {
        RelationshipType::Father | RelationshipType::Mother => {
            // Update the parent edge pedigree.
            let q = format!(
                "UPDATE {} SET pedigree = $ped WHERE in = $from AND out = $to",
                rel_type
            );
            let from = RecordId::new("person", id.as_str());
            let to = parse_record_id(&related_id)?;
            db.query(q)
                .bind(("ped", new_ped))
                .bind(("from", from.clone()))
                .bind(("to", to))
                .await?;

            // Cascade: update sibling edges whose via_parent_id points to this parent.
            // Step/adopted/foster parent → step siblings; birth parent → birth siblings.
            let sib_ped = if payload.pedigree != Pedigree::Birth { "step" } else { "birth" };
            db.query(
                "UPDATE has_sibling SET pedigree = $sib_ped \
                 WHERE (in = $person OR out = $person) AND via_parent_id = $vpid"
            )
            .bind(("sib_ped", sib_ped))
            .bind(("person", from))
            .bind(("vpid", related_id))
            .await?;
        }
        RelationshipType::Sibling => {
            let from = RecordId::new("person", id.as_str());
            let to = parse_record_id(&related_id)?;
            // Update pedigree; also update via_parent_id if provided.
            db.query(
                "UPDATE has_sibling SET pedigree = $ped, via_parent_id = $vpid \
                 WHERE (in = $from AND out = $to) OR (in = $to AND out = $from)"
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
        ("id" = String, Path, description = "Person record ID (without the `person:` prefix)"),
        ("related_id" = String, Path, description = "Full related record ID, e.g. `person:01jd4a8xyz`"),
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
    Path((id, related_id)): Path<(String, String)>,
    Query(filter): Query<PersonFilter>,
    Json(payload): Json<UpdateSpouseDatesRequest>,
) -> Result<StatusCode, AppError> {
    let db = db.lock().await;

    if filter.created_by.is_some() {
        let person: Option<Person> = db.select(("person", id.as_str())).await?;
        check_ownership(&person.ok_or(AppError::NotFound)?, &filter.created_by)?;
    }
    let from = RecordId::new("person", id.as_str());
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
