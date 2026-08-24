use axum::{
    extract::{DefaultBodyLimit, Request},
    http::{header, Method, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{delete, get, patch, post},
    Extension, Json, Router,
};
use std::convert::Infallible;

use crate::{
    auth::jwt_middleware,
    db::Db,
    handlers::{
        contact_request::{
            accept_contact_request, append_contact_message, create_contact_requests,
            get_pending_count, ignore_contact_request, list_contact_requests,
        },
        family_tree::{create_tree, delete_tree, get_tree, list_trees, set_primary_tree, set_tree_team, update_tree},
        image::{get_image, get_life_image, get_tree_image, upload_image, upload_life_image, upload_tree_image},
        life_event::{create_life_event, delete_life_event, get_life_event, list_life_events, promote_life_event, update_life_event},
        merge::{
            accept_merge_proposal, create_merge_proposal,
            get_merge_proposal, list_merge_proposals, reject_merge_proposal,
        },
        person::{add_person_to_tree, collapse_same_tree_duplicate, create_person, delete_person, find_duplicates, get_person, list_persons, list_proxy_links, remove_person_from_tree, update_canonical, update_person},
        relationship::{
            add_relationship, delete_relationship, get_family_tree, get_relationships,
            update_relationship_pedigree, update_spouse_dates,
        },
        chat_session::{create_session, delete_session, list_sessions, list_session_messages, append_message},
        research_folder::{create_folder, delete_folder, list_folders, rename_folder},
        research_note::{create_note_reply, create_research_note, delete_research_note, get_research_note, list_note_replies, list_research_notes, set_note_folder, update_research_note},
        tree_editor::{add_tree_editor, get_my_tree_access, list_tree_editors, remove_tree_editor},
        user_ai_settings::{delete_ai_settings, get_ai_settings, upsert_ai_settings},
    },
    openapi::{openapi_json, swagger_ui},
};

/// Configuration for the MCP endpoint and RFC 9728 protected resource metadata.
pub struct McpConfig {
    /// Canonical URI of this resource server — used as the OAuth2 audience.
    pub canonical_uri: String,
    /// UUM issuer URL — the authorization server for this resource.
    pub authorization_server: String,
    /// JWKS URI — included in the protected resource metadata response.
    pub jwks_url: String,
}

/// Intercepts GET /mcp and returns a persistent SSE keepalive stream.
///
/// The MCP SDK's StreamableHTTPClientTransport always opens a GET SSE channel
/// for server-to-client events after any successful POST. Without this handler
/// the server returns 404, which triggers a disconnect after 2 mcp-remote
/// retries. Clann has no server-initiated events, so we hold the connection
/// open with 15-second keepalive comments and let the client close it.
async fn mcp_sse_keepalive(req: Request, next: Next) -> Response {
    if req.method() == Method::GET {
        let stream = futures_util::stream::pending::<Result<Event, Infallible>>();
        return Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
            .into_response();
    }
    next.run(req).await
}

/// Axum middleware that validates the MCP bearer token (RS256, audience-bound).
///
/// Returns a `WWW-Authenticate` header on 401 so MCP clients (Claude Code,
/// Claude Desktop) can auto-discover the Authorization Server via RFC 9728.
async fn mcp_auth_middleware(
    req: Request,
    next: Next,
    validator: ullav_mcp_auth::TokenValidator,
    canonical_uri: String,
) -> Response {
    let resource_metadata_url =
        format!("{canonical_uri}/.well-known/oauth-protected-resource");
    let www_auth_base = format!(
        r#"Bearer resource_metadata="{resource_metadata_url}", scope="mcp:tools""#
    );

    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    match token {
        None => (
            StatusCode::UNAUTHORIZED,
            [(axum::http::header::WWW_AUTHENTICATE, www_auth_base)],
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response(),
        Some(t) => match validator.validate_as::<serde_json::Value>(&t).await {
            Err(_) => (
                StatusCode::UNAUTHORIZED,
                [(
                    axum::http::header::WWW_AUTHENTICATE,
                    format!(
                        r#"Bearer resource_metadata="{resource_metadata_url}", scope="mcp:tools", error="invalid_token""#
                    ),
                )],
                Json(serde_json::json!({ "error": "invalid_token" })),
            )
                .into_response(),
            Ok(claims) => {
                let username = claims
                    .get("username")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                crate::mcp::server::AUTHENTICATED_USERNAME
                    .scope(username, next.run(req))
                    .await
            }
        },
    }
}

pub fn build_router(
    db: Db,
    upload_dir: String,
    enable_docs: bool,
    validator: Option<ullav_mcp_auth::TokenValidator>,
    mcp: Option<McpConfig>,
    tack_client: crate::tack_client::TackClient,
) -> Router {
    let mut router = Router::new();

    if enable_docs {
        router = router
            .route("/api-docs/openapi.json", get(openapi_json))
            .route("/swagger-ui", get(swagger_ui));
    }

    let auth_layer = middleware::from_fn(move |req, next| {
        let v = validator.clone();
        async move { jwt_middleware(req, next, v).await }
    });

    // Image GET routes are public — browsers fetch <img src> without Authorization headers.
    let public_routes = Router::new()
        .route("/api/persons/{id}/image", get(get_image))
        .route("/api/persons/{id}/life-image", get(get_life_image))
        .route("/api/trees/{name}/image", get(get_tree_image));

    let protected_routes = Router::new()
        // Family trees
        .route("/api/trees", post(create_tree).get(list_trees))
        .route("/api/trees/{name}", get(get_tree).patch(update_tree).delete(delete_tree))
        .route("/api/trees/{name}/set-primary", patch(set_primary_tree))
        .route("/api/trees/{name}/team", patch(set_tree_team))
        .route("/api/trees/{name}/editors", get(list_tree_editors).post(add_tree_editor))
        .route("/api/trees/{name}/editors/{user_id}", delete(remove_tree_editor))
        .route("/api/trees/{name}/my-access", get(get_my_tree_access))
        .route("/api/trees/{name}/image", post(upload_tree_image)
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024)))
        // Persons
        .route("/api/persons", post(create_person).get(list_persons))
        .route(
            "/api/persons/{id}",
            get(get_person).put(update_person).delete(delete_person),
        )
        .route("/api/persons/{id}/canonical", patch(update_canonical))
        .route("/api/persons/{id}/linked-proxies", get(list_proxy_links))
        .route("/api/persons/{id}/collapse-into/{survivor_id}", post(collapse_same_tree_duplicate))
        .route("/api/persons/{id}/find-duplicates", get(find_duplicates))
        .route("/api/persons/{id}/trees", post(add_person_to_tree))
        .route("/api/persons/{id}/trees/{tree_name}", delete(remove_person_from_tree))
        // Image uploads (GET is in public_routes above).
        // Body limits are set per-route so Axum enforces them before the handler runs.
        .route("/api/persons/{id}/image", post(upload_image)
            .layer(DefaultBodyLimit::max(2 * 1024 * 1024)))
        .route("/api/persons/{id}/life-image", post(upload_life_image)
            .layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
        // Relationships
        .route(
            "/api/persons/{id}/relationships",
            post(add_relationship).get(get_relationships),
        )
        .route(
            "/api/persons/{id}/relationships/{rel_type}/{related_id}",
            delete(delete_relationship).patch(update_relationship_pedigree),
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
        .route("/api/life-events/{event_id}/promote", patch(promote_life_event))
        // Contact requests (cross-user duplicate communication, precedes merge)
        .route("/api/contact-requests", post(create_contact_requests).get(list_contact_requests))
        .route("/api/contact-requests/pending-count", get(get_pending_count))
        .route("/api/contact-requests/{id}/accept", patch(accept_contact_request))
        .route("/api/contact-requests/{id}/ignore", patch(ignore_contact_request))
        .route("/api/contact-requests/{id}/messages", post(append_contact_message))
        // Merge proposals
        .route("/api/merge-proposals", post(create_merge_proposal).get(list_merge_proposals))
        .route("/api/merge-proposals/{id}", get(get_merge_proposal))
        .route("/api/merge-proposals/{id}/accept", patch(accept_merge_proposal))
        .route("/api/merge-proposals/{id}/reject", patch(reject_merge_proposal))
        // Research Notes
        .route("/api/notes", post(create_research_note).get(list_research_notes))
        .route(
            "/api/notes/{note_id}",
            get(get_research_note).put(update_research_note).delete(delete_research_note),
        )
        .route("/api/notes/{note_id}/folder", patch(set_note_folder))
        .route("/api/notes/{note_id}/replies", get(list_note_replies).post(create_note_reply))
        // Research Folders
        .route("/api/folders", post(create_folder).get(list_folders))
        .route("/api/folders/{id}", patch(rename_folder).delete(delete_folder))
        // AI Settings (encrypted BYOK; webapp encrypts, server stores opaque blobs)
        .route(
            "/api/ai-settings",
            get(get_ai_settings).put(upsert_ai_settings).delete(delete_ai_settings),
        )
        // Chat Sessions
        .route("/api/chat/sessions", post(create_session).get(list_sessions))
        .route("/api/chat/sessions/{id}", delete(delete_session))
        .route("/api/chat/sessions/{id}/messages", get(list_session_messages).post(append_message))
        .layer(auth_layer);

    let mut router = router
        .merge(public_routes)
        .merge(protected_routes)
        .layer(Extension(upload_dir));

    if let Some(mcp_cfg) = mcp {
        let mcp_validator = ullav_mcp_auth::TokenValidator::new(
            mcp_cfg.jwks_url.clone(),
            mcp_cfg.authorization_server.clone(),
            mcp_cfg.canonical_uri.clone(),
        );

        let canonical_uri = mcp_cfg.canonical_uri.clone();
        let auth_server = mcp_cfg.authorization_server.clone();
        let jwks_url = mcp_cfg.jwks_url.clone();

        let pr_router = Router::new().route(
            "/.well-known/oauth-protected-resource",
            get(move || async move {
                axum::Json(serde_json::json!({
                    "resource": canonical_uri,
                    "authorization_servers": [auth_server],
                    "bearer_methods_supported": ["header"],
                    "jwks_uri": jwks_url,
                }))
            }),
        );

        let canonical_uri_for_mw = mcp_cfg.canonical_uri.clone();
        let canonical_host = url::Url::parse(&mcp_cfg.canonical_uri)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let mcp_router = Router::new()
            .route_service("/mcp", crate::mcp::server::make_mcp_service(db.clone(), canonical_host))
            .layer(middleware::from_fn(mcp_sse_keepalive))
            .layer(middleware::from_fn(move |req, next| {
                let v = mcp_validator.clone();
                let u = canonical_uri_for_mw.clone();
                async move { mcp_auth_middleware(req, next, v, u).await }
            }));

        router = router.merge(pr_router).merge(mcp_router);
    }

    // Extension, not State -- research_note.rs/research_folder.rs are the
    // only consumers, same reasoning ClannAuth itself already uses (a
    // cross-cutting dependency layered on top, not threaded through every
    // handler's State<Db> signature).
    router.layer(Extension(tack_client)).with_state(db)
}
