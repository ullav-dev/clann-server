use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::{
    db::Db,
    handlers::{
        person::{create_person, delete_person, get_person, list_persons, update_person},
        relationship::{add_relationship, delete_relationship, get_family_tree, get_relationships},
    },
    openapi::{openapi_json, swagger_ui},
};

pub fn build_router(db: Db) -> Router {
    Router::new()
        // API docs
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/swagger-ui", get(swagger_ui))
        // Persons
        .route("/api/persons", post(create_person).get(list_persons))
        .route(
            "/api/persons/{id}",
            get(get_person).put(update_person).delete(delete_person),
        )
        // Relationships
        .route(
            "/api/persons/{id}/relationships",
            post(add_relationship).get(get_relationships),
        )
        .route(
            "/api/persons/{id}/relationships/{rel_type}/{related_id}",
            delete(delete_relationship),
        )
        .route("/api/persons/{id}/family-tree", get(get_family_tree))
        .with_state(db)
}
