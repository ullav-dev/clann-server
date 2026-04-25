use axum::{
    middleware,
    routing::{delete, get, patch, post},
    Extension, Router,
};

use crate::{
    auth::jwt_middleware,
    db::Db,
    handlers::{
        family_tree::{create_tree, delete_tree, get_tree, list_trees, set_primary_tree},
        image::{get_image, get_life_image, upload_image, upload_life_image},
        life_event::{create_life_event, delete_life_event, get_life_event, list_life_events, update_life_event},
        person::{add_person_to_tree, create_person, delete_person, get_person, list_persons, remove_person_from_tree, update_person},
        relationship::{
            add_relationship, delete_relationship, get_family_tree, get_relationships,
            update_spouse_dates,
        },
        research_note::{create_research_note, delete_research_note, get_research_note, list_research_notes, update_research_note},
        user_ai_settings::{delete_ai_settings, get_ai_settings, upsert_ai_settings},
    },
    openapi::{openapi_json, swagger_ui},
};

pub fn build_router(db: Db, upload_dir: String, enable_docs: bool, jwt_secret: Option<String>) -> Router {
    let mut router = Router::new();

    if enable_docs {
        router = router
            .route("/api-docs/openapi.json", get(openapi_json))
            .route("/swagger-ui", get(swagger_ui));
    }

    let secret = jwt_secret;
    let auth_layer = middleware::from_fn(move |req, next| {
        let secret = secret.clone();
        async move { jwt_middleware(req, next, secret).await }
    });

    // Image GET routes are public — browsers fetch <img src> without Authorization headers.
    let public_routes = Router::new()
        .route("/api/persons/{id}/image", get(get_image))
        .route("/api/persons/{id}/life-image", get(get_life_image));

    let protected_routes = Router::new()
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
        // Image uploads (GET is in public_routes above)
        .route("/api/persons/{id}/image", post(upload_image))
        .route("/api/persons/{id}/life-image", post(upload_life_image))
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
        // Life Events
        .route(
            "/api/persons/{id}/life-events",
            post(create_life_event).get(list_life_events),
        )
        .route(
            "/api/life-events/{event_id}",
            get(get_life_event).put(update_life_event).delete(delete_life_event),
        )
        // Research Notes
        .route("/api/notes", post(create_research_note).get(list_research_notes))
        .route(
            "/api/notes/{note_id}",
            get(get_research_note).put(update_research_note).delete(delete_research_note),
        )
        // AI Settings (encrypted BYOK; webapp encrypts, server stores opaque blobs)
        .route(
            "/api/ai-settings",
            get(get_ai_settings).put(upsert_ai_settings).delete(delete_ai_settings),
        )
        .layer(auth_layer);

    router
        .merge(public_routes)
        .merge(protected_routes)
        .layer(Extension(upload_dir))
        .with_state(db)
}
