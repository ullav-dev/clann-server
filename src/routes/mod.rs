use axum::{
    routing::{delete, get, patch, post},
    Extension, Router,
};

use crate::{
    db::Db,
    handlers::{
        family_tree::{create_tree, delete_tree, get_tree, list_trees, set_primary_tree},
        image::{get_image, get_life_image, upload_image, upload_life_image},
        person::{add_person_to_tree, create_person, delete_person, get_person, list_persons, remove_person_from_tree, update_person},
        relationship::{
            add_relationship, delete_relationship, get_family_tree, get_relationships,
            update_spouse_dates,
        },
    },
    openapi::{openapi_json, swagger_ui},
};

pub fn build_router(db: Db, upload_dir: String, enable_docs: bool) -> Router {
    let mut router = Router::new();

    if enable_docs {
        router = router
            .route("/api-docs/openapi.json", get(openapi_json))
            .route("/swagger-ui", get(swagger_ui));
    }

    router
        // Family trees
        .route("/api/trees", post(create_tree).get(list_trees))
        .route("/api/trees/{name}", get(get_tree).delete(delete_tree))
        .route("/api/trees/{name}/set-primary", patch(set_primary_tree))
        // Persons
        .route("/api/persons", post(create_person).get(list_persons))
        .route(
            "/api/persons/{id}",
            get(get_person).put(update_person).delete(delete_person),
        )
        .route("/api/persons/{id}/trees", post(add_person_to_tree))
        .route("/api/persons/{id}/trees/{tree_name}", delete(remove_person_from_tree))
        // Images
        .route(
            "/api/persons/{id}/image",
            post(upload_image).get(get_image),
        )
        .route(
            "/api/persons/{id}/life-image",
            post(upload_life_image).get(get_life_image),
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
