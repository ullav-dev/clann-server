use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use clann_server::{db::Db, routes::build_router};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use surrealdb::{engine::any, opt::auth::Root};
use tower::ServiceExt;

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

async fn setup() -> axum::Router {
    let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ns = format!("test_ns_{}", n);
    let db_name = format!("test_db_{}", n);

    let db: Db = any::connect("mem://").await.unwrap();
    db.signin(Root {
        username: "root".to_string(),
        password: "root".to_string(),
    })
    .await
    .ok(); // embedded engine may not require auth
    db.use_ns(ns).use_db(db_name).await.unwrap();

    let schema = include_str!("../migrations/schema.surql");
    db.query(schema).await.unwrap();

    build_router(db, std::env::temp_dir().to_string_lossy().into_owned())
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn put_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn delete(uri: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn patch_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Extract the bare record ID from a SurrealDB Thing, however it is serialized.
///
/// SurrealDB v2 serializes `Thing` as an object:
///   `{"tb": "person", "id": "01jXXX"}`          (string ID)
///   `{"tb": "person", "id": {"String": "01jXXX"}}` (enum-wrapped ID)
/// Older/string form: `"person:01jXXX"`.
fn record_id(body: &Value) -> String {
    let id = &body["id"];

    // Plain string: "person:01jXXX"
    if let Some(s) = id.as_str() {
        return s.split(':').nth(1).unwrap_or_default().to_string();
    }

    // Object form – unwrap the inner `id` field.
    let inner = &id["id"];

    // Direct string: {"tb": "person", "id": "01jXXX"}
    if let Some(s) = inner.as_str() {
        return s.to_string();
    }

    // Enum-wrapped: {"tb": "person", "id": {"String": "01jXXX"}}
    if inner.is_object() {
        if let Some(s) = inner.as_object().unwrap().values().next().and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }

    panic!("Cannot extract record ID from: {}", body["id"]);
}

// ── helpers ──────────────────────────────────────────────────────────────────

async fn create_person_req(app: axum::Router, payload: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(post_json("/api/persons", payload))
        .await
        .unwrap();
    let status = response.status();
    let body = response_json(response).await;
    (status, body)
}

// ── OpenAPI docs ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_openapi_spec_is_valid_json() {
    let app = setup().await;
    let response = app.oneshot(get("/api-docs/openapi.json")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["info"]["title"], "clann-server API");
    assert!(body["paths"].is_object());
    assert!(body["components"]["schemas"].is_object());
}

#[tokio::test]
async fn test_swagger_ui_is_served() {
    let app = setup().await;
    let response = app.oneshot(get("/swagger-ui")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(html.contains("swagger-ui"));
    assert!(html.contains("/api-docs/openapi.json"));
}

// ── Person CRUD ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_person_returns_201_with_body() {
    let app = setup().await;
    let (status, body) = create_person_req(
        app,
        json!({"family_name": "Smith", "first_name": "John", "sex": "Male"}),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["family_name"], "Smith");
    assert_eq!(body["first_name"], "John");
    assert_eq!(body["sex"], "Male");
    assert!(!body["id"].is_null());
}

#[tokio::test]
async fn test_create_person_with_optional_fields() {
    let app = setup().await;
    let (status, body) = create_person_req(
        app,
        json!({
            "family_name": "Doe",
            "first_name": "Jane",
            "sex": "Female",
            "middle_name": "Marie",
            "date_of_birth": "1990-05-15",
            "place_of_birth": "Dublin",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["middle_name"], "Marie");
    assert_eq!(body["date_of_birth"], "1990-05-15");
    assert_eq!(body["place_of_birth"], "Dublin");
}

#[tokio::test]
async fn test_create_person_missing_required_field_returns_422() {
    let app = setup().await;
    // missing `sex`
    let response = app
        .oneshot(post_json(
            "/api/persons",
            json!({"family_name": "Smith", "first_name": "John"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_list_persons_empty() {
    let app = setup().await;
    let response = app.oneshot(get("/api/persons")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn test_list_persons_returns_all() {
    let app = setup().await;

    for name in ["Alice", "Bob", "Carol"] {
        let response = app
            .clone()
            .oneshot(post_json(
                "/api/persons",
                json!({"family_name": "Test", "first_name": name, "sex": "Female"}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let response = app.oneshot(get("/api/persons")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn test_get_person_by_id() {
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({"family_name": "Walsh", "first_name": "Eoin", "sex": "Male"}),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .oneshot(get(&format!("/api/persons/{}", id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["first_name"], "Eoin");
}

#[tokio::test]
async fn test_get_person_not_found_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(get("/api/persons/nonexistent_id"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert!(body["error"].as_str().is_some());
}

#[tokio::test]
async fn test_update_person_merges_fields() {
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({"family_name": "Murphy", "first_name": "Seán", "sex": "Male"}),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/persons/{}", id),
            json!({"family_name": "O'Murphy", "date_of_birth": "1980-01-01"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["family_name"], "O'Murphy");
    assert_eq!(body["first_name"], "Seán"); // unchanged
    assert_eq!(body["date_of_birth"], "1980-01-01");
}

#[tokio::test]
async fn test_update_person_not_found_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(put_json(
            "/api/persons/no_such_record",
            json!({"first_name": "Ghost"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_person_returns_204() {
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({"family_name": "Brennan", "first_name": "Aisling", "sex": "Female"}),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .clone()
        .oneshot(delete(&format!("/api/persons/{}", id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm gone
    let response = app
        .oneshot(get(&format!("/api/persons/{}", id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_person_returns_204() {
    // DELETE is idempotent — 204 even when record does not exist
    let app = setup().await;
    let response = app
        .oneshot(delete("/api/persons/phantom"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ── Relationships ─────────────────────────────────────────────────────────────

async fn make_person(app: axum::Router, name: &str, sex: &str) -> String {
    let (_, body) = create_person_req(
        app,
        json!({"family_name": "Test", "first_name": name, "sex": sex}),
    )
    .await;
    record_id(&body)
}

#[tokio::test]
async fn test_add_father_relationship_returns_201() {
    let app = setup().await;
    let child_id = make_person(app.clone(), "Child", "Male").await;
    let father_id = format!("person:{}", make_person(app.clone(), "Father", "Male").await);

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", child_id),
            json!({"type": "Father", "related_id": father_id}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_add_mother_relationship_returns_201() {
    let app = setup().await;
    let child_id = make_person(app.clone(), "Child", "Female").await;
    let mother_id = format!("person:{}", make_person(app.clone(), "Mother", "Female").await);

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", child_id),
            json!({"type": "Mother", "related_id": mother_id}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_add_sibling_relationship_returns_201() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Sibling1", "Male").await;
    let sibling_id = format!("person:{}", make_person(app.clone(), "Sibling2", "Female").await);

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", person_id),
            json!({"type": "Sibling", "related_id": sibling_id, "sibling_type": "Sister"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_add_sibling_without_sibling_type_returns_400() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "P1", "Male").await;
    let other_id = format!("person:{}", make_person(app.clone(), "P2", "Male").await);

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", person_id),
            json!({"type": "Sibling", "related_id": other_id}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body["error"].as_str().unwrap().contains("sibling_type"));
}

#[tokio::test]
async fn test_get_relationships_returns_grouped_persons() {
    let app = setup().await;
    let child_id = make_person(app.clone(), "Child", "Male").await;
    let father_id = make_person(app.clone(), "Dad", "Male").await;
    let mother_id = make_person(app.clone(), "Mum", "Female").await;
    let sib_id = make_person(app.clone(), "Sis", "Female").await;

    // Add relationships
    for (rel_type, related_id, extra) in [
        ("Father", format!("person:{}", father_id), json!({})),
        ("Mother", format!("person:{}", mother_id), json!({})),
        (
            "Sibling",
            format!("person:{}", sib_id),
            json!({"sibling_type": "Sister"}),
        ),
    ] {
        let mut payload = json!({"type": rel_type, "related_id": related_id});
        if let Some(obj) = extra.as_object() {
            payload.as_object_mut().unwrap().extend(obj.clone());
        }
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/persons/{}/relationships", child_id),
                payload,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", child_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["father"].as_array().unwrap().len(), 1);
    assert_eq!(body["mother"].as_array().unwrap().len(), 1);
    assert_eq!(body["siblings"].as_array().unwrap().len(), 1);
    assert_eq!(body["father"][0]["first_name"], "Dad");
    assert_eq!(body["mother"][0]["first_name"], "Mum");
    assert_eq!(body["siblings"][0]["first_name"], "Sis");
}

#[tokio::test]
async fn test_sibling_relationship_is_bidirectional() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Alpha", "Male").await;
    let b_id = make_person(app.clone(), "Beta", "Male").await;

    // Relate A -> B as sibling
    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({"type": "Sibling", "related_id": format!("person:{}", b_id), "sibling_type": "Brother"}),
        ))
        .await
        .unwrap();

    // Getting B's relationships should show A as sibling
    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", b_id)))
        .await
        .unwrap();
    let body = response_json(response).await;
    assert_eq!(body["siblings"].as_array().unwrap().len(), 1);
    assert_eq!(body["siblings"][0]["first_name"], "Alpha");
}

#[tokio::test]
async fn test_delete_relationship_returns_204() {
    let app = setup().await;
    let child_id = make_person(app.clone(), "Child", "Male").await;
    let father_id = make_person(app.clone(), "Father", "Male").await;
    let father_full = format!("person:{}", father_id);

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", child_id),
            json!({"type": "Father", "related_id": father_full}),
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(delete(&format!(
            "/api/persons/{}/relationships/has_father/person:{}",
            child_id, father_id
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Relationship is gone
    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", child_id)))
        .await
        .unwrap();
    let body = response_json(response).await;
    assert_eq!(body["father"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_delete_relationship_invalid_type_returns_400() {
    let app = setup().await;
    let response = app
        .oneshot(delete("/api/persons/abc/relationships/has_uncle/person:xyz"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body["error"].as_str().is_some());
}

// ── Family tree ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_family_tree_not_found_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(get("/api/persons/no_one/family-tree"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_family_tree_returns_nested_ancestors() {
    let app = setup().await;

    // Build: grandpa -> dad -> child
    let grandpa_id = make_person(app.clone(), "Grandpa", "Male").await;
    let dad_id = make_person(app.clone(), "Dad", "Male").await;
    let child_id = make_person(app.clone(), "Child", "Male").await;

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", dad_id),
            json!({"type": "Father", "related_id": format!("person:{}", grandpa_id)}),
        ))
        .await
        .unwrap();

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", child_id),
            json!({"type": "Father", "related_id": format!("person:{}", dad_id)}),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get(&format!("/api/persons/{}/family-tree", child_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["first_name"], "Child");
    assert_eq!(body["father"][0]["first_name"], "Dad");
    assert_eq!(body["father"][0]["father"][0]["first_name"], "Grandpa");
}

// ── created_by field and list filter ─────────────────────────────────────────

#[tokio::test]
async fn test_create_person_with_created_by() {
    let app = setup().await;
    let (status, body) = create_person_req(
        app,
        json!({"family_name": "Nolan", "first_name": "Brian", "sex": "Male", "created_by": "admin"}),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["created_by"], "admin");
}

#[tokio::test]
async fn test_list_persons_filter_by_created_by() {
    let app = setup().await;

    // Create two persons by "alice" and one by "bob"
    for name in ["Anna", "Amy"] {
        create_person_req(
            app.clone(),
            json!({"family_name": "Test", "first_name": name, "sex": "Female", "created_by": "alice"}),
        )
        .await;
    }
    create_person_req(
        app.clone(),
        json!({"family_name": "Test", "first_name": "Bob", "sex": "Male", "created_by": "bob"}),
    )
    .await;

    let response = app
        .clone()
        .oneshot(get("/api/persons?created_by=alice"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let persons = body.as_array().unwrap();
    assert_eq!(persons.len(), 2);
    assert!(persons.iter().all(|p| p["created_by"] == "alice"));

    // Unfiltered list returns all three
    let response = app.oneshot(get("/api/persons")).await.unwrap();
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn test_list_persons_filter_no_matches_returns_empty() {
    let app = setup().await;
    create_person_req(
        app.clone(),
        json!({"family_name": "Test", "first_name": "Only", "sex": "Male", "created_by": "alice"}),
    )
    .await;

    let response = app
        .oneshot(get("/api/persons?created_by=nobody"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

// ── Person profile fields (nickname / username / email) ──────────────────────

#[tokio::test]
async fn test_create_person_with_profile_fields() {
    let app = setup().await;
    let (status, body) = create_person_req(
        app,
        json!({
            "family_name": "Kelly",
            "first_name": "Niamh",
            "sex": "Female",
            "nickname": "Neve",
            "username": "nkelly",
            "email": "niamh@example.com",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["nickname"], "Neve");
    assert_eq!(body["username"], "nkelly");
    assert_eq!(body["email"], "niamh@example.com");
}

#[tokio::test]
async fn test_profile_fields_are_null_when_not_set() {
    let app = setup().await;
    let (_, body) = create_person_req(
        app,
        json!({"family_name": "Ryan", "first_name": "Cian", "sex": "Male"}),
    )
    .await;

    assert!(body["nickname"].is_null());
    assert!(body["username"].is_null());
    assert!(body["email"].is_null());
}

#[tokio::test]
async fn test_update_person_profile_fields() {
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({"family_name": "Burke", "first_name": "Aoife", "sex": "Female"}),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/persons/{}", id),
            json!({"nickname": "Aoif", "username": "aburke", "email": "aoife@example.com"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["nickname"], "Aoif");
    assert_eq!(body["username"], "aburke");
    assert_eq!(body["email"], "aoife@example.com");
    assert_eq!(body["first_name"], "Aoife"); // other fields unchanged
}

#[tokio::test]
async fn test_update_omits_null_fields_leaving_existing_values_intact() {
    // UpdatePerson uses skip_serializing_if = "Option::is_none", so null fields
    // are dropped from the MERGE payload and existing DB values are preserved.
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({
            "family_name": "Flynn",
            "first_name": "Oisín",
            "sex": "Male",
            "nickname": "Osh",
        }),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/persons/{}", id),
            json!({"nickname": null, "first_name": "Oisín Updated"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    // null was skipped — nickname retains its original value
    assert_eq!(body["nickname"], "Osh");
    assert_eq!(body["first_name"], "Oisín Updated");
}

// ── Spouse relationship ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_add_spouse_relationship_returns_201() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Bride", "Female").await;
    let b_id = format!("person:{}", make_person(app.clone(), "Groom", "Male").await);

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({"type": "Spouse", "related_id": b_id}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_spouse_relationship_is_bidirectional() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Alice", "Female").await;
    let b_id = make_person(app.clone(), "Bob", "Male").await;

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({"type": "Spouse", "related_id": format!("person:{}", b_id)}),
        ))
        .await
        .unwrap();

    // Bob's relationships should list Alice as spouse
    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", b_id)))
        .await
        .unwrap();
    let body = response_json(response).await;
    assert_eq!(body["spouse"].as_array().unwrap().len(), 1);
    assert_eq!(body["spouse"][0]["first_name"], "Alice");
}

#[tokio::test]
async fn test_spouse_relationship_with_dates() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Pat", "Male").await;
    let b_id = make_person(app.clone(), "Sam", "Female").await;

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({
                "type": "Spouse",
                "related_id": format!("person:{}", b_id),
                "spouse_from": "2000-07-14",
                "spouse_to": "2015-03-01",
            }),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", a_id)))
        .await
        .unwrap();
    let body = response_json(response).await;
    let spouse = &body["spouse"][0];
    assert_eq!(spouse["first_name"], "Sam");
    assert_eq!(spouse["spouse_from"], "2000-07-14");
    assert_eq!(spouse["spouse_to"], "2015-03-01");
}

#[tokio::test]
async fn test_update_spouse_dates() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Jamie", "Male").await;
    let b_id = make_person(app.clone(), "Robin", "Female").await;
    let b_full = format!("person:{}", b_id);

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({"type": "Spouse", "related_id": b_full.clone()}),
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(patch_json(
            &format!("/api/persons/{}/spouse-dates/{}", a_id, b_full),
            json!({"spouse_from": "1999-05-20", "spouse_to": null}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm dates persisted
    let response = app
        .oneshot(get(&format!("/api/persons/{}/relationships", a_id)))
        .await
        .unwrap();
    let body = response_json(response).await;
    assert_eq!(body["spouse"][0]["spouse_from"], "1999-05-20");
}

#[tokio::test]
async fn test_delete_spouse_relationship_removes_both_directions() {
    let app = setup().await;
    let a_id = make_person(app.clone(), "Lee", "Male").await;
    let b_id = make_person(app.clone(), "Jordan", "Female").await;
    let b_full = format!("person:{}", b_id);

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", a_id),
            json!({"type": "Spouse", "related_id": b_full.clone()}),
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(delete(&format!(
            "/api/persons/{}/relationships/has_spouse/{}",
            a_id, b_full
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Neither side should show the other as spouse
    for id in [&a_id, &b_id] {
        let response = app
            .clone()
            .oneshot(get(&format!("/api/persons/{}/relationships", id)))
            .await
            .unwrap();
        let body = response_json(response).await;
        assert_eq!(body["spouse"].as_array().unwrap().len(), 0, "spouse list should be empty for {id}");
    }
}

// ── Family tree with spouse and children ─────────────────────────────────────

#[tokio::test]
async fn test_family_tree_includes_spouse_and_children() {
    let app = setup().await;
    let parent_id = make_person(app.clone(), "Parent", "Male").await;
    let spouse_id = make_person(app.clone(), "Spouse", "Female").await;
    let child_id = make_person(app.clone(), "Child", "Male").await;

    // parent married spouse
    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", parent_id),
            json!({"type": "Spouse", "related_id": format!("person:{}", spouse_id)}),
        ))
        .await
        .unwrap();

    // child's father is parent
    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/relationships", child_id),
            json!({"type": "Father", "related_id": format!("person:{}", parent_id)}),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get(&format!("/api/persons/{}/family-tree", parent_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["first_name"], "Parent");
    assert_eq!(body["spouse"].as_array().unwrap().len(), 1);
    assert_eq!(body["spouse"][0]["first_name"], "Spouse");
    assert_eq!(body["children"].as_array().unwrap().len(), 1);
    assert_eq!(body["children"][0]["first_name"], "Child");
}
