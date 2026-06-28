use axum::{response::Html, Json};
use utoipa::OpenApi;

use crate::{
    error::ErrorResponse,
    models::{
        contact_request::{
            AppendContactMessage, CreateContactRequest, DuplicateSearchResult,
            MergeContactRequest, UnreadContactCount,
        },
        family_tree::{CreateFamilyTree, FamilyTree, SetTreeTeam, UpdateFamilyTree},
        life_event::{CreateLifeEvent, EventType, LifeEvent, UpdateLifeEvent},
        merge_proposal::{AcceptMergeProposalRequest, CreateMergeProposalRequest, MergeProposal, MergeResolution},
        person::{
            CreatePerson, Person, PersonProxy, PersonProxyResponse, PersonProxyStub,
            PersonResponse, Sex, TreeMembershipRequest, UpdateCanonicalPerson, UpdatePersonProxy,
        },
        relationship::{
            AddRelationshipRequest, RelationshipType,
            RelationshipsResponse, SiblingType, UpdateRelationshipRequest,
        },
    },
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "clann-server API",
        version = "26.3.0",
        description = "REST API for ancestry and family tree data management (person-proxy architecture), backed by SurrealDB."
    ),
    paths(
        crate::handlers::family_tree::create_tree,
        crate::handlers::family_tree::list_trees,
        crate::handlers::family_tree::get_tree,
        crate::handlers::family_tree::update_tree,
        crate::handlers::family_tree::delete_tree,
        crate::handlers::family_tree::set_primary_tree,
        crate::handlers::family_tree::set_tree_team,
        crate::handlers::person::create_person,
        crate::handlers::person::list_persons,
        crate::handlers::person::get_person,
        crate::handlers::person::update_person,
        crate::handlers::person::update_canonical,
        crate::handlers::person::delete_person,
        crate::handlers::person::add_person_to_tree,
        crate::handlers::person::remove_person_from_tree,
        crate::handlers::person::list_proxy_links,
        crate::handlers::person::collapse_same_tree_duplicate,
        crate::handlers::image::upload_image,
        crate::handlers::image::get_image,
        crate::handlers::image::upload_life_image,
        crate::handlers::image::get_life_image,
        crate::handlers::relationship::add_relationship,
        crate::handlers::relationship::get_relationships,
        crate::handlers::relationship::delete_relationship,
        crate::handlers::relationship::get_family_tree,
        crate::handlers::relationship::update_spouse_dates,
        crate::handlers::relationship::update_relationship_pedigree,
        crate::handlers::life_event::create_life_event,
        crate::handlers::life_event::list_life_events,
        crate::handlers::life_event::get_life_event,
        crate::handlers::life_event::update_life_event,
        crate::handlers::life_event::delete_life_event,
        crate::handlers::life_event::promote_life_event,
        crate::handlers::merge::create_merge_proposal,
        crate::handlers::merge::list_merge_proposals,
        crate::handlers::merge::get_merge_proposal,
        crate::handlers::merge::accept_merge_proposal,
        crate::handlers::merge::reject_merge_proposal,
        crate::handlers::person::find_duplicates,
        crate::handlers::contact_request::create_contact_requests,
        crate::handlers::contact_request::list_contact_requests,
        crate::handlers::contact_request::get_pending_count,
        crate::handlers::contact_request::accept_contact_request,
        crate::handlers::contact_request::ignore_contact_request,
        crate::handlers::contact_request::append_contact_message,
    ),
    components(
        schemas(
            FamilyTree, CreateFamilyTree, UpdateFamilyTree, SetTreeTeam,
            Person, PersonProxy, PersonProxyResponse, PersonProxyStub, PersonResponse,
            CreatePerson, UpdatePersonProxy, UpdateCanonicalPerson, TreeMembershipRequest, Sex,
            AddRelationshipRequest, RelationshipsResponse,
            SiblingType, RelationshipType,
            crate::models::relationship::SpouseInfo,
            crate::models::relationship::ParentInfo,
            crate::models::relationship::SiblingInfo,
            crate::models::relationship::UpdateSpouseDatesRequest,
            UpdateRelationshipRequest,
            LifeEvent, CreateLifeEvent, UpdateLifeEvent, EventType,
            MergeProposal, CreateMergeProposalRequest, AcceptMergeProposalRequest, MergeResolution,
            MergeContactRequest, CreateContactRequest, AppendContactMessage,
            DuplicateSearchResult, UnreadContactCount,
            ErrorResponse,
        )
    ),
    tags(
        (name = "trees",         description = "Create, read and delete family trees"),
        (name = "persons",       description = "Create, read, update and delete person proxies and canonicals"),
        (name = "relationships", description = "Manage family relationships and traverse the family tree"),
        (name = "life-events",   description = "Create, read, update, delete, and promote life events"),
        (name = "merge",            description = "Cross-tree canonical merge proposals"),
        (name = "contact-requests", description = "Cross-user duplicate contact requests and conversation threads"),
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
