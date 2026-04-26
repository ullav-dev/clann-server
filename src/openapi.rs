use axum::{response::Html, Json};
use utoipa::OpenApi;

use crate::{
    error::ErrorResponse,
    models::{
        family_tree::{CreateFamilyTree, FamilyTree, UpdateFamilyTree},
        life_event::{CreateLifeEvent, EventType, LifeEvent, UpdateLifeEvent},
        person::{CreatePerson, Person, Sex, UpdatePerson},
        relationship::{AddRelationshipRequest, RelationshipType, RelationshipsResponse, SiblingType},
    },
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "clann-server API",
        version = "0.1.0",
        description = "REST API for ancestry and family tree data management, backed by SurrealDB."
    ),
    paths(
        crate::handlers::family_tree::create_tree,
        crate::handlers::family_tree::list_trees,
        crate::handlers::family_tree::get_tree,
        crate::handlers::family_tree::update_tree,
        crate::handlers::family_tree::delete_tree,
        crate::handlers::family_tree::set_primary_tree,
        crate::handlers::person::create_person,
        crate::handlers::person::list_persons,
        crate::handlers::person::get_person,
        crate::handlers::person::update_person,
        crate::handlers::person::delete_person,
        crate::handlers::image::upload_image,
        crate::handlers::image::get_image,
        crate::handlers::relationship::add_relationship,
        crate::handlers::relationship::get_relationships,
        crate::handlers::relationship::delete_relationship,
        crate::handlers::relationship::get_family_tree,
        crate::handlers::relationship::update_spouse_dates,
        crate::handlers::life_event::create_life_event,
        crate::handlers::life_event::list_life_events,
        crate::handlers::life_event::get_life_event,
        crate::handlers::life_event::update_life_event,
        crate::handlers::life_event::delete_life_event,
    ),
    components(
        schemas(
            FamilyTree, CreateFamilyTree, UpdateFamilyTree,
            Person, CreatePerson, UpdatePerson, Sex,
            AddRelationshipRequest, RelationshipsResponse,
            SiblingType, RelationshipType,
            crate::models::relationship::SpouseInfo,
            crate::models::relationship::UpdateSpouseDatesRequest,
            LifeEvent, CreateLifeEvent, UpdateLifeEvent, EventType,
            ErrorResponse,
        )
    ),
    tags(
        (name = "trees",         description = "Create, read and delete family trees"),
        (name = "persons",       description = "Create, read, update and delete person records"),
        (name = "relationships", description = "Manage family relationships and traverse the family tree"),
        (name = "life-events",   description = "Create, read, update and delete life events for persons"),
    )
)]
pub struct ApiDoc;

/// Serve the OpenAPI JSON spec.
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// Serve Swagger UI (loads assets from unpkg CDN).
pub async fn swagger_ui() -> Html<&'static str> {
    Html(SWAGGER_HTML)
}

static SWAGGER_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>clann-server API Docs</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
</head>
<body>
<div id="swagger-ui"></div>
<script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>
  window.onload = () => {
    SwaggerUIBundle({
      url: '/api-docs/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
      layout: 'BaseLayout',
    });
  };
</script>
</body>
</html>
"#;
