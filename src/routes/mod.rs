use axum::{
    routing::{delete, get, patch, post},
    Extension, Router,
};

use crate::{
    db::Db,
    handlers::{
        family_tree::{create_tree, delete_tree, get_tree, list_trees},
        image::{get_image, upload_image},
        person::{create_person, delete_person, get_person, list_persons, update_person},
        relationship::{
            add_relationship, delete_relationship, get_family_tree, get_relationships,
            update_spouse_dates,
        },
    },
    openapi::{openapi_json, swagger_ui},
};

pub fn build_router(db: Db, upload_dir: String) -> Router {
    Router::new()
        // API docs
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/swagger-ui", get(swagger_ui))
        // Family trees
        .route("/api/trees", post(create_tree).get(list_trees))
        .route("/api/trees/{name}", get(get_tree).delete(delete_tree))
        // Persons
        .route("/api/persons", post(create_person).get(list_persons))
        .route(
            "/api/persons/{id}",
            get(get_person).put(update_person).delete(delete_person),
        )
        // Images
        .route(
            "/api/persons/{id}/image",
            post(upload_image).get(get_image),
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
        .route(
            "/api/persons/{id}/spouse-dates/{related_id}",
            patch(update_spouse_dates),
        )
        .layer(Extension(upload_dir))
        .with_state(db)
}
