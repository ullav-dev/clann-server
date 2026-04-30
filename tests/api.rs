use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use clann_server::{db::Db, routes::build_router};
use http_body_util::BodyExt;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{atomic::{AtomicU64, Ordering}, Arc};
use surrealdb::{engine::any, opt::auth::Root};
use tokio::sync::Mutex;
use tower::ServiceExt;

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

const TEST_TREE: &str = "test-tree";
const TEST_JWT_SECRET: &str = "test-secret";

// ── JWT token builder ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: u64,
    subscriptions: Value,
}

/// Build a signed JWT with the given Clann subscription tier and status.
/// Uses `TEST_JWT_SECRET` as the signing key.
fn make_clann_jwt(tier: &str, status: &str) -> String {
    let claims = TestClaims {
        sub: "test-user-id".to_string(),
        exp: 9_999_999_999,
        subscriptions: json!({ "clann": { "tier": tier, "status": status } }),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .unwrap()
}

/// Build a signed JWT with no subscriptions claim at all.
fn make_jwt_no_subs() -> String {
    #[derive(Serialize)]
    struct NoClaims { sub: String, exp: u64 }
    let claims = NoClaims { sub: "test-user-id".to_string(), exp: 9_999_999_999 };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes())).unwrap()
}

// ── Auth-enabled setup ────────────────────────────────────────────────────────

/// Like `setup()` but with JWT enforcement enabled.
/// Returns both the router (for HTTP requests) and the raw DB handle
/// (for direct data seeding in limit tests).
async fn setup_with_auth() -> (axum::Router, Db) {
    let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let conn = any::connect("mem://").await.unwrap();
    conn.signin(Root { username: "root".to_string(), password: "root".to_string() })
        .await
        .ok();
    conn.use_ns(format!("test_ns_{n}")).use_db(format!("test_db_{n}")).await.unwrap();
    conn.query(include_str!("../migrations/schema.surql")).await.unwrap();
    conn.query(
        "CREATE family_tree CONTENT { name: $name, display_name: 'Test Tree', owner: 'testowner', is_primary: true }",
    )
    .bind(("name", TEST_TREE))
    .await
    .unwrap();

    let db: Db = Arc::new(Mutex::new(conn));
    let router = build_router(
        db.clone(),
        std::env::temp_dir().to_string_lossy().into_owned(),
        true,
        Some(TEST_JWT_SECRET.to_string()),
    );
    (router, db)
}

// ── Request helpers (auth-enabled variants) ───────────────────────────────────

fn post_json_auth(uri: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn setup() -> axum::Router {
    let n = DB_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ns = format!("test_ns_{}", n);
    let db_name = format!("test_db_{}", n);

    let conn = any::connect("mem://").await.unwrap();
    conn.signin(Root {
        username: "root".to_string(),
        password: "root".to_string(),
    })
    .await
    .ok(); // embedded engine may not require auth
    conn.use_ns(ns).use_db(db_name).await.unwrap();

    let schema = include_str!("../migrations/schema.surql");
    conn.query(schema).await.unwrap();

    // Seed a default family tree so tests can create persons without needing their own tree setup
    conn.query(
        "CREATE family_tree CONTENT { name: $name, display_name: 'Test Tree', owner: 'testowner', is_primary: true }",
    )
    .bind(("name", TEST_TREE))
    .await
    .unwrap();

    let db: Db = Arc::new(Mutex::new(conn));
    build_router(db, std::env::temp_dir().to_string_lossy().into_owned(), true, None)
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
        json!({"family_name": "Smith", "first_name": "John", "sex": "Male", "trees": [TEST_TREE]}),
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
            "trees": [TEST_TREE],
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
                json!({"family_name": "Test", "first_name": name, "sex": "Female", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Walsh", "first_name": "Eoin", "sex": "Male", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Murphy", "first_name": "Seán", "sex": "Male", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Brennan", "first_name": "Aisling", "sex": "Female", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Test", "first_name": name, "sex": sex, "trees": [TEST_TREE]}),
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
        json!({"family_name": "Nolan", "first_name": "Brian", "sex": "Male", "created_by": "admin", "trees": [TEST_TREE]}),
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
            json!({"family_name": "Test", "first_name": name, "sex": "Female", "created_by": "alice", "trees": [TEST_TREE]}),
        )
        .await;
    }
    create_person_req(
        app.clone(),
        json!({"family_name": "Test", "first_name": "Bob", "sex": "Male", "created_by": "bob", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Test", "first_name": "Only", "sex": "Male", "created_by": "alice", "trees": [TEST_TREE]}),
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

#[tokio::test]
async fn test_created_by_is_null_when_not_set() {
    let app = setup().await;
    let (_, body) = create_person_req(
        app,
        json!({"family_name": "Ghost", "first_name": "User", "sex": "Male", "trees": [TEST_TREE]}),
    )
    .await;

    assert!(body["created_by"].is_null());
}

#[tokio::test]
async fn test_update_person_created_by() {
    let app = setup().await;
    let (_, created) = create_person_req(
        app.clone(),
        json!({"family_name": "Casey", "first_name": "Pat", "sex": "Male", "trees": [TEST_TREE]}),
    )
    .await;
    let id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/persons/{}", id),
            json!({"created_by": "editor"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["created_by"], "editor");
    assert_eq!(body["first_name"], "Pat"); // other fields unchanged
}

#[tokio::test]
async fn test_list_filter_created_by_is_case_sensitive() {
    let app = setup().await;
    create_person_req(
        app.clone(),
        json!({"family_name": "Test", "first_name": "Lower", "sex": "Male", "created_by": "alice", "trees": [TEST_TREE]}),
    )
    .await;

    // "Alice" (capital A) should not match "alice"
    let response = app
        .oneshot(get("/api/persons?created_by=Alice"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_filter_created_by_only_returns_own_records() {
    let app = setup().await;
    create_person_req(
        app.clone(),
        json!({"family_name": "A", "first_name": "One", "sex": "Male", "created_by": "userA", "trees": [TEST_TREE]}),
    )
    .await;
    create_person_req(
        app.clone(),
        json!({"family_name": "B", "first_name": "Two", "sex": "Female", "created_by": "userB", "trees": [TEST_TREE]}),
    )
    .await;
    // No created_by set
    create_person_req(
        app.clone(),
        json!({"family_name": "C", "first_name": "Three", "sex": "Male", "trees": [TEST_TREE]}),
    )
    .await;

    let response = app
        .oneshot(get("/api/persons?created_by=userA"))
        .await
        .unwrap();
    let body = response_json(response).await;
    let persons = body.as_array().unwrap();
    assert_eq!(persons.len(), 1);
    assert_eq!(persons[0]["first_name"], "One");
    assert_eq!(persons[0]["created_by"], "userA");
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
            "trees": [TEST_TREE],
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
        json!({"family_name": "Ryan", "first_name": "Cian", "sex": "Male", "trees": [TEST_TREE]}),
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
        json!({"family_name": "Burke", "first_name": "Aoife", "sex": "Female", "trees": [TEST_TREE]}),
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
            "trees": [TEST_TREE],
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

// ── Family tree CRUD ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_family_tree_returns_201() {
    let app = setup().await;
    let response = app
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "O'Brien", "display_name": "The O'Brien Family", "owner": "user1"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await;
    assert_eq!(body["name"], "O'Brien");
    assert_eq!(body["display_name"], "The O'Brien Family");
    assert_eq!(body["owner"], "user1");
    assert_eq!(body["is_primary"], false);
}

#[tokio::test]
async fn test_create_family_tree_as_primary() {
    let app = setup().await;

    // Create two trees for the same owner; the second is primary
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "tree-alpha", "display_name": "Alpha", "owner": "alice", "is_primary": true}),
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "tree-beta", "display_name": "Beta", "owner": "alice", "is_primary": true}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // tree-alpha should no longer be primary; tree-beta should be
    let list: Value = response_json(
        app.oneshot(get("/api/trees?owner=alice")).await.unwrap(),
    )
    .await;
    let trees = list.as_array().unwrap();
    let alpha = trees.iter().find(|t| t["name"] == "tree-alpha").unwrap();
    let beta = trees.iter().find(|t| t["name"] == "tree-beta").unwrap();
    assert_eq!(alpha["is_primary"], false);
    assert_eq!(beta["is_primary"], true);
}

#[tokio::test]
async fn test_create_family_tree_duplicate_name_returns_400() {
    let app = setup().await;
    // TEST_TREE already exists from setup
    let response = app
        .oneshot(post_json(
            "/api/trees",
            json!({"name": TEST_TREE, "display_name": "Duplicate", "owner": "someone"}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn test_list_family_trees() {
    let app = setup().await;
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "tree-x", "display_name": "X", "owner": "owner1"}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "tree-y", "display_name": "Y", "owner": "owner2"}),
        ))
        .await
        .unwrap();

    let response = app.oneshot(get("/api/trees")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    // At least 3 (TEST_TREE + tree-x + tree-y)
    assert!(body.as_array().unwrap().len() >= 3);
}

#[tokio::test]
async fn test_list_family_trees_filter_by_owner() {
    let app = setup().await;
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "bob-tree-1", "display_name": "Bob 1", "owner": "bob"}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "bob-tree-2", "display_name": "Bob 2", "owner": "bob"}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "carol-tree", "display_name": "Carol", "owner": "carol"}),
        ))
        .await
        .unwrap();

    let response = app.oneshot(get("/api/trees?owner=bob")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let trees = body.as_array().unwrap();
    assert_eq!(trees.len(), 2);
    assert!(trees.iter().all(|t| t["owner"] == "bob"));
}

#[tokio::test]
async fn test_get_family_tree_by_name() {
    let app = setup().await;
    let response = app
        .oneshot(get(&format!("/api/trees/{}", TEST_TREE)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], TEST_TREE);
}

#[tokio::test]
async fn test_get_family_tree_not_found() {
    let app = setup().await;
    let response = app
        .oneshot(get("/api/trees/no-such-tree"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_family_tree_returns_204() {
    let app = setup().await;
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "to-delete", "display_name": "Gone", "owner": "user"}),
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(delete("/api/trees/to-delete"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm gone
    let response = app.oneshot(get("/api/trees/to-delete")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_family_tree_cascades_persons() {
    let app = setup().await;
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "cascade-tree", "display_name": "Cascade", "owner": "user"}),
        ))
        .await
        .unwrap();

    // Create a person in that tree
    let (_, person) = create_person_req(
        app.clone(),
        json!({"family_name": "Test", "first_name": "Gone", "sex": "Male", "trees": ["cascade-tree"]}),
    )
    .await;
    let person_id = record_id(&person);

    // Delete the tree
    app.clone()
        .oneshot(delete("/api/trees/cascade-tree"))
        .await
        .unwrap();

    // The person should be gone
    let response = app
        .oneshot(get(&format!("/api/persons/{}", person_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_tree_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(delete("/api/trees/phantom-tree"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_person_with_unknown_tree_returns_400() {
    let app = setup().await;
    let (status, body) = create_person_req(
        app,
        json!({"family_name": "Smith", "first_name": "John", "sex": "Male", "trees": ["no-such-tree"]}),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_list_persons_filter_by_tree() {
    let app = setup().await;

    // Create a second tree
    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "other-tree", "display_name": "Other", "owner": "user"}),
        ))
        .await
        .unwrap();

    create_person_req(
        app.clone(),
        json!({"family_name": "A", "first_name": "Alice", "sex": "Female", "trees": [TEST_TREE]}),
    )
    .await;
    create_person_req(
        app.clone(),
        json!({"family_name": "B", "first_name": "Bob", "sex": "Male", "trees": ["other-tree"]}),
    )
    .await;

    let response = app
        .oneshot(get(&format!("/api/persons?tree={}", TEST_TREE)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let persons = body.as_array().unwrap();
    assert_eq!(persons.len(), 1);
    assert_eq!(persons[0]["first_name"], "Alice");
}

// ── JWT middleware enforcement ────────────────────────────────────────────────

#[tokio::test]
async fn test_missing_jwt_returns_401() {
    let (app, _) = setup_with_auth().await;
    // POST /api/persons with no Authorization header
    let response = app
        .oneshot(post_json(
            "/api/persons",
            json!({"family_name": "Test", "first_name": "Nobody", "sex": "Male", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_invalid_jwt_returns_401() {
    let (app, _) = setup_with_auth().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/persons")
                .header("content-type", "application/json")
                .header("authorization", "Bearer this.is.not.valid")
                .body(Body::from(
                    json!({"family_name": "T", "first_name": "N", "sex": "Male", "trees": [TEST_TREE]})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_valid_jwt_allows_request() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");
    let response = app
        .oneshot(post_json_auth(
            "/api/persons",
            json!({"family_name": "Test", "first_name": "Valid", "sex": "Male", "trees": [TEST_TREE]}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_jwt_with_no_subscriptions_treated_as_individual() {
    // No subscriptions claim → defaults to individual (limit=2 trees / 100 members)
    // Just confirm the request is accepted (not 401)
    let (app, _) = setup_with_auth().await;
    let token = make_jwt_no_subs();
    let response = app
        .oneshot(post_json_auth(
            "/api/persons",
            json!({"family_name": "Test", "first_name": "NoSub", "sex": "Male", "trees": [TEST_TREE]}),
            &token,
        ))
        .await
        .unwrap();
    // Should be 201 (under individual member limit)
    assert_eq!(response.status(), StatusCode::CREATED);
}

// ── Tree limit enforcement ────────────────────────────────────────────────────

#[tokio::test]
async fn test_individual_tree_limit_is_two() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");
    const OWNER: &str = "limit-tree-user";

    // First two trees should succeed
    for name in ["tree-limit-a", "tree-limit-b"] {
        let resp = app
            .clone()
            .oneshot(post_json_auth(
                "/api/trees",
                json!({"name": name, "display_name": name, "owner": OWNER, "is_primary": false}),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "Expected 201 for tree {name}");
    }

    // Third tree should be rejected
    let resp = app
        .oneshot(post_json_auth(
            "/api/trees",
            json!({"name": "tree-limit-c", "display_name": "C", "owner": OWNER, "is_primary": false}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = response_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("Tree limit"));
}

#[tokio::test]
async fn test_professional_tree_limit_is_unlimited() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("professional", "active");
    const OWNER: &str = "pro-tree-user";

    // Create more than the individual limit (3 trees) — all should succeed
    for name in ["pro-tree-1", "pro-tree-2", "pro-tree-3"] {
        let resp = app
            .clone()
            .oneshot(post_json_auth(
                "/api/trees",
                json!({"name": name, "display_name": name, "owner": OWNER, "is_primary": false}),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "Expected 201 for {name}");
    }
}

#[tokio::test]
async fn test_family_tree_limit_is_ten() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("family", "active");
    const OWNER: &str = "family-tree-limit-user";

    // Seed 10 trees directly through the API (family limit = 10)
    for i in 0..10 {
        let name = format!("fam-tree-{i}");
        let resp = app
            .clone()
            .oneshot(post_json_auth(
                "/api/trees",
                json!({"name": name, "display_name": name, "owner": OWNER, "is_primary": false}),
                &token,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "Expected 201 for tree {i}");
    }

    // 11th should be rejected
    let resp = app
        .oneshot(post_json_auth(
            "/api/trees",
            json!({"name": "fam-tree-overflow", "display_name": "X", "owner": OWNER, "is_primary": false}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ── Member (person) limit enforcement ────────────────────────────────────────

#[tokio::test]
async fn test_individual_member_limit_is_100() {
    let (app, db) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");
    const CREATOR: &str = "limit-person-user";

    // Seed 100 persons directly in the DB to avoid 100 HTTP round-trips
    {
        let db = db.lock().await;
        for i in 0..100u32 {
            db.query("CREATE person CONTENT { family_name: 'Seed', first_name: $n, sex: 'Male', trees: [$tree], created_by: $creator }")
                .bind(("n", format!("Seed{i}")))
                .bind(("tree", TEST_TREE))
                .bind(("creator", CREATOR))
                .await
                .unwrap();
        }
    }

    // 101st person via the API should be rejected
    let resp = app
        .oneshot(post_json_auth(
            "/api/persons",
            json!({"family_name": "Over", "first_name": "Limit", "sex": "Male", "trees": [TEST_TREE], "created_by": CREATOR}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = response_json(resp).await;
    assert!(body["error"].as_str().unwrap().contains("Member limit"));
}

#[tokio::test]
async fn test_individual_member_limit_at_99_allows_one_more() {
    let (app, db) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");
    const CREATOR: &str = "limit-person-user-99";

    // Seed 99 persons
    {
        let db = db.lock().await;
        for i in 0..99u32 {
            db.query("CREATE person CONTENT { family_name: 'Seed', first_name: $n, sex: 'Male', trees: [$tree], created_by: $creator }")
                .bind(("n", format!("Seed{i}")))
                .bind(("tree", TEST_TREE))
                .bind(("creator", CREATOR))
                .await
                .unwrap();
        }
    }

    // 100th should succeed
    let resp = app
        .oneshot(post_json_auth(
            "/api/persons",
            json!({"family_name": "Last", "first_name": "Allowed", "sex": "Male", "trees": [TEST_TREE], "created_by": CREATOR}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_professional_member_limit_is_unlimited() {
    let (app, db) = setup_with_auth().await;
    let token = make_clann_jwt("professional", "active");
    const CREATOR: &str = "pro-person-user";

    // Seed 100 persons (individual limit) to confirm professional ignores it
    {
        let db = db.lock().await;
        for i in 0..100u32 {
            db.query("CREATE person CONTENT { family_name: 'Seed', first_name: $n, sex: 'Male', trees: [$tree], created_by: $creator }")
                .bind(("n", format!("Seed{i}")))
                .bind(("tree", TEST_TREE))
                .bind(("creator", CREATOR))
                .await
                .unwrap();
        }
    }

    // Should still succeed for professional
    let resp = app
        .oneshot(post_json_auth(
            "/api/persons",
            json!({"family_name": "Pro", "first_name": "Extra", "sex": "Male", "trees": [TEST_TREE], "created_by": CREATOR}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_inactive_subscription_treated_as_individual() {
    // A canceled/inactive subscription should default to individual tier limits
    let (app, _) = setup_with_auth().await;
    // "professional" tier but "canceled" status → should behave as individual
    let token = make_clann_jwt("professional", "canceled");
    const OWNER: &str = "canceled-sub-user";

    for name in ["canceled-tree-a", "canceled-tree-b"] {
        app.clone()
            .oneshot(post_json_auth(
                "/api/trees",
                json!({"name": name, "display_name": name, "owner": OWNER, "is_primary": false}),
                &token,
            ))
            .await
            .unwrap();
    }

    // Third tree should fail — treated as individual (limit=2)
    let resp = app
        .oneshot(post_json_auth(
            "/api/trees",
            json!({"name": "canceled-tree-c", "display_name": "C", "owner": OWNER, "is_primary": false}),
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_get_endpoints_accessible_with_valid_jwt() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");

    // GET /api/persons should be accessible (read operations need auth too)
    let resp = app
        .oneshot(get_auth("/api/persons", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_endpoints_blocked_without_jwt() {
    let (app, _) = setup_with_auth().await;
    let resp = app.oneshot(get("/api/persons")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── Life Events ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_life_event_returns_201() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    let response = app
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({
                "name": "Graduation",
                "event_type": "Graduation",
                "date": "2000-06-15",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await;
    assert_eq!(body["name"], "Graduation");
    assert_eq!(body["event_type"], "Graduation");
    assert_eq!(body["date"], "2000-06-15");
    assert!(!body["id"].is_null());
}

#[tokio::test]
async fn test_create_life_event_for_nonexistent_person_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(post_json(
            "/api/persons/nonexistent/life-events",
            json!({"name": "Birth", "event_type": "Birth"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_list_life_events_empty() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Empty", "Male").await;

    let response = app
        .oneshot(get(&format!("/api/persons/{}/life-events", person_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn test_list_life_events_returns_all_for_person() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    for (name, event_type) in [("Birth", "Birth"), ("Graduation", "Graduation")] {
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/persons/{}/life-events", person_id),
                json!({"name": name, "event_type": event_type}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let response = app
        .oneshot(get(&format!("/api/persons/{}/life-events", person_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_list_life_events_filter_by_created_by() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({"name": "Birth", "event_type": "Birth", "created_by": "alice"}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({"name": "Death", "event_type": "Death", "created_by": "bob"}),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get(&format!(
            "/api/persons/{}/life-events?created_by=alice",
            person_id
        )))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let events = body.as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["name"], "Birth");
}

#[tokio::test]
async fn test_get_life_event_by_id() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({"name": "Marriage", "event_type": "Marriage", "date": "1990-05-20"}),
        ))
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created = response_json(create_resp).await;
    let event_id = record_id(&created);

    let response = app
        .oneshot(get(&format!("/api/life-events/{}", event_id)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], "Marriage");
    assert_eq!(body["date"], "1990-05-20");
}

#[tokio::test]
async fn test_get_life_event_not_found_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(get("/api/life-events/nonexistent"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_life_event() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({"name": "Birth", "event_type": "Birth"}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let event_id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/life-events/{}", event_id),
            json!({
                "name": "Birth Updated",
                "event_type": "Birth",
                "date": "1985-03-01",
                "description": "Born in Dublin",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], "Birth Updated");
    assert_eq!(body["date"], "1985-03-01");
    assert_eq!(body["description"], "Born in Dublin");
}

#[tokio::test]
async fn test_update_life_event_not_found_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(put_json(
            "/api/life-events/nonexistent",
            json!({"name": "Ghost", "event_type": "Other"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_life_event_returns_204() {
    let app = setup().await;
    let person_id = make_person(app.clone(), "Ada", "Female").await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/persons/{}/life-events", person_id),
            json!({"name": "Birth", "event_type": "Birth"}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let event_id = record_id(&created);

    let response = app
        .clone()
        .oneshot(delete(&format!("/api/life-events/{}", event_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm gone
    let response = app
        .oneshot(get(&format!("/api/life-events/{}", event_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_life_event_returns_404() {
    let app = setup().await;
    let response = app
        .oneshot(delete("/api/life-events/phantom"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Research Notes ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_research_note_returns_201() {
    let app = setup().await;
    let response = app
        .oneshot(post_json(
            "/api/notes",
            json!({
                "title": "Clann na Gael",
                "description": "Notes on the O'Brien family",
                "trees": [TEST_TREE],
                "created_by": "researcher",
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await;
    assert_eq!(body["title"], "Clann na Gael");
    assert_eq!(body["description"], "Notes on the O'Brien family");
    assert!(!body["id"].is_null());
}

#[tokio::test]
async fn test_list_research_notes_empty() {
    let app = setup().await;
    let response = app.oneshot(get("/api/notes")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn test_list_research_notes_filter_by_tree() {
    let app = setup().await;

    app.clone()
        .oneshot(post_json(
            "/api/trees",
            json!({"name": "other-note-tree", "display_name": "Other", "owner": "user"}),
        ))
        .await
        .unwrap();

    app.clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Note A", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Note B", "trees": ["other-note-tree"]}),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get(&format!("/api/notes?tree={}", TEST_TREE)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let notes = body.as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["title"], "Note A");
}

#[tokio::test]
async fn test_list_research_notes_filter_by_created_by() {
    let app = setup().await;

    app.clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Alice Note", "trees": [TEST_TREE], "created_by": "alice"}),
        ))
        .await
        .unwrap();
    app.clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Bob Note", "trees": [TEST_TREE], "created_by": "bob"}),
        ))
        .await
        .unwrap();

    let response = app
        .oneshot(get("/api/notes?created_by=alice"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let notes = body.as_array().unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["title"], "Alice Note");
}

#[tokio::test]
async fn test_get_research_note_by_id() {
    let app = setup().await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Census 1911", "body": "Found in Roscommon", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let note_id = record_id(&created);

    let response = app
        .oneshot(get(&format!("/api/notes/{}", note_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["title"], "Census 1911");
    assert_eq!(body["body"], "Found in Roscommon");
}

#[tokio::test]
async fn test_get_research_note_not_found() {
    let app = setup().await;
    let response = app.oneshot(get("/api/notes/nonexistent")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_research_note() {
    let app = setup().await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Draft", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let note_id = record_id(&created);

    let response = app
        .clone()
        .oneshot(put_json(
            &format!("/api/notes/{}", note_id),
            json!({"title": "Revised Title", "body": "Updated body content", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["title"], "Revised Title");
    assert_eq!(body["body"], "Updated body content");
}

#[tokio::test]
async fn test_update_research_note_not_found() {
    let app = setup().await;
    let response = app
        .oneshot(put_json(
            "/api/notes/nonexistent",
            json!({"title": "Ghost Note"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_research_note_returns_204() {
    let app = setup().await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "To Delete", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let note_id = record_id(&created);

    let response = app
        .clone()
        .oneshot(delete(&format!("/api/notes/{}", note_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm gone
    let response = app
        .oneshot(get(&format!("/api/notes/{}", note_id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_research_note_not_found() {
    let app = setup().await;
    let response = app.oneshot(delete("/api/notes/phantom")).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_set_note_folder() {
    let app = setup().await;

    let create_resp = app
        .clone()
        .oneshot(post_json(
            "/api/notes",
            json!({"title": "Note", "trees": [TEST_TREE]}),
        ))
        .await
        .unwrap();
    let created = response_json(create_resp).await;
    let note_id = record_id(&created);

    // Assign to a folder
    let response = app
        .clone()
        .oneshot(patch_json(
            &format!("/api/notes/{}/folder", note_id),
            json!({"folder_id": "research_folder:abc123"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["folder_id"], "research_folder:abc123");

    // Unfile from folder
    let response = app
        .clone()
        .oneshot(patch_json(
            &format!("/api/notes/{}/folder", note_id),
            json!({"folder_id": null}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert!(body["folder_id"].is_null());
}

// ── User AI Settings ──────────────────────────────────────────────────────────

fn put_json_auth(uri: &str, body: Value, token: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn delete_auth(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn test_get_ai_settings_not_found_when_empty() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");
    let response = app
        .oneshot(get_auth("/api/ai-settings", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_upsert_ai_settings_creates_new() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");

    let response = app
        .oneshot(put_json_auth(
            "/api/ai-settings",
            json!({
                "provider": "anthropic",
                "model": "claude-opus-4-7",
                "encrypted_key": "encrypted_blob",
                "iv": "iv_blob",
                "auth_tag": "tag_blob",
            }),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["provider"], "anthropic");
    assert_eq!(body["model"], "claude-opus-4-7");
    assert_eq!(body["username"], "test-user-id");
}

#[tokio::test]
async fn test_upsert_ai_settings_is_idempotent() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");

    // First PUT
    app.clone()
        .oneshot(put_json_auth(
            "/api/ai-settings",
            json!({"provider": "openai", "model": "gpt-4"}),
            &token,
        ))
        .await
        .unwrap();

    // Second PUT with different model — should update, not create a duplicate
    let response = app
        .oneshot(put_json_auth(
            "/api/ai-settings",
            json!({"provider": "openai", "model": "gpt-4o"}),
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["model"], "gpt-4o");
}

#[tokio::test]
async fn test_delete_ai_settings_returns_204() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");

    // Create settings first
    app.clone()
        .oneshot(put_json_auth(
            "/api/ai-settings",
            json!({"provider": "anthropic", "model": "claude-opus-4-7"}),
            &token,
        ))
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(delete_auth("/api/ai-settings", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Confirm gone
    let response = app
        .oneshot(get_auth("/api/ai-settings", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_ai_settings_when_none_is_noop() {
    let (app, _) = setup_with_auth().await;
    let token = make_clann_jwt("individual", "active");

    // Delete with nothing stored — should still return 204
    let response = app
        .oneshot(delete_auth("/api/ai-settings", &token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
