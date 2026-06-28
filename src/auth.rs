use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use ullav_mcp_auth::TokenValidator;

// ── JWT claims (mirrors ullav-user-management) ────────────────────────────────

#[derive(Debug, Deserialize)]
struct SubscriptionClaim {
    pub tier: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct TeamClaim {
    pub role: String,
    /// Per-member product roles: product_slug → role (e.g. "clann" → "owner"|"member").
    #[serde(default)]
    pub product_roles: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawClaims {
    pub sub: String,
    /// Login username — used for ownership checks without an extra DB lookup.
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub subscriptions: HashMap<String, SubscriptionClaim>,
    /// Active team memberships keyed by team UUID string.
    #[serde(default)]
    pub teams: HashMap<String, TeamClaim>,
}

// ── ClannAuth — injected into every request via middleware ────────────────────

/// Authenticated user context attached to every request by the JWT middleware.
///
/// Authenticated user context attached to every request by the JWT middleware.
#[derive(Debug, Clone)]
pub struct ClannAuth {
    /// JWT `sub` — the user's UUID string.
    pub user_id: String,
    /// Login username from the JWT `username` claim.
    /// Empty for tokens issued before this claim was added (very old tokens).
    pub username: String,
    /// Plan tier: "individual", "family", "professional", "enterprise".
    pub tier: String,
    /// Active team memberships: map of team UUID → positional role ("owner"|"leader"|"member").
    pub teams: HashMap<String, String>,
    /// Per-member Clann product roles: map of team UUID → "owner"|"member".
    /// Empty for tokens issued before product roles were added (pre-UUM migration 014).
    pub clann_roles: HashMap<String, String>,
}

impl ClannAuth {
    /// Returns the user's Clann product role for the given team UUID, if any.
    pub fn clann_role_for_team(&self, team_id: &str) -> Option<&str> {
        self.clann_roles.get(team_id).map(|s| s.as_str())
    }

    /// Returns true if the user has any form of Clann access in the given team.
    pub fn has_clann_access_for_team(&self, team_id: &str) -> bool {
        self.teams.contains_key(team_id)
    }
}

impl ClannAuth {
    /// Maximum number of family trees for this plan. `None` = unlimited.
    pub fn tree_limit(&self) -> Option<usize> {
        match self.tier.as_str() {
            "professional" | "enterprise" => None,
            "family" => Some(10),
            _ => Some(2), // individual or unknown
        }
    }

    /// Maximum total number of persons for this plan. `None` = unlimited.
    pub fn member_limit(&self) -> Option<usize> {
        match self.tier.as_str() {
            "professional" | "enterprise" => None,
            "family" => Some(1000),
            _ => Some(100), // individual
        }
    }

    /// Maximum total media storage in bytes for this plan. `None` = unlimited.
    pub fn storage_limit_bytes(&self) -> Option<i64> {
        match self.tier.as_str() {
            "enterprise" => None,
            "professional" => Some(50 * 1024 * 1024 * 1024), // 50 GB
            "family" => Some(5 * 1024 * 1024 * 1024),         // 5 GB
            _ => Some(100 * 1024 * 1024),                     // individual: 100 MB
        }
    }

    /// Returns `true` if the given MIME type is allowed for life-story media uploads.
    /// Individual/family plans are image-only; professional/enterprise allow all media.
    pub fn life_media_type_allowed(&self, mime: &str) -> bool {
        match self.tier.as_str() {
            "professional" | "enterprise" => {
                mime.starts_with("image/")
                    || mime.starts_with("video/")
                    || mime.starts_with("audio/")
                    || mime == "application/pdf"
            }
            _ => mime.starts_with("image/"),
        }
    }
}

// ── Middleware ────────────────────────────────────────────────────────────────

/// Axum middleware that validates the `Authorization: Bearer` RS256 JWT
/// and inserts a `ClannAuth` extension into every request.
///
/// When `validator` is `None` (dev / test mode), auth is skipped and an
/// enterprise-tier `ClannAuth` is injected so plan limits are never triggered.
pub async fn jwt_middleware(
    mut request: Request,
    next: Next,
    validator: Option<TokenValidator>,
) -> Response {
    let auth = match validator {
        None => ClannAuth {
            user_id: String::new(),
            username: String::new(),
            tier: "enterprise".to_string(),
            teams: HashMap::new(),
            clann_roles: HashMap::new(),
        },
        Some(v) => {
            let token = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|s| s.to_string());

            match token {
                None => return unauthorized("Missing Authorization header"),
                Some(token) => {
                    match v.validate_as::<RawClaims>(&token).await {
                        Err(e) => return unauthorized(&format!("Invalid token: {e}")),
                        Ok(raw) => {
                            let clann_sub = raw.subscriptions.get("clann");
                            let tier = match clann_sub {
                                None => "individual".to_string(),
                                Some(sub)
                                    if sub.status != "active" && sub.status != "trialing" =>
                                {
                                    "individual".to_string()
                                }
                                Some(sub) => sub.tier.clone(),
                            };
                            let mut teams = HashMap::new();
                            let mut clann_roles = HashMap::new();
                            for (id, claim) in raw.teams {
                                if let Some(role) = claim.product_roles.get("clann") {
                                    clann_roles.insert(id.clone(), role.clone());
                                }
                                teams.insert(id, claim.role);
                            }
                            ClannAuth { user_id: raw.sub, username: raw.username, tier, teams, clann_roles }
                        }
                    }
                }
            }
        }
    };

    request.extensions_mut().insert(auth);
    next.run(request).await
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": msg })),
    )
        .into_response()
}
