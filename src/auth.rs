use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

// ── JWT claims (mirrors ullav-user-management) ────────────────────────────────

#[derive(Debug, Deserialize)]
struct SubscriptionClaim {
    pub tier: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
struct TeamClaim {
    pub role: String,
}

#[derive(Debug, Deserialize)]
struct RawClaims {
    pub sub: String,
    #[serde(default)]
    pub subscriptions: HashMap<String, SubscriptionClaim>,
    /// Active team memberships keyed by team UUID string.
    #[serde(default)]
    pub teams: HashMap<String, TeamClaim>,
}

// ── ClannAuth — injected into every request via middleware ────────────────────

/// Authenticated user context attached to every request by the JWT middleware.
///
/// When `JWT_SECRET` is not set (tests / local dev), a default `ClannAuth`
/// with `tier = "enterprise"` is inserted so all handlers work without auth.
#[derive(Debug, Clone)]
pub struct ClannAuth {
    /// JWT `sub` — the user's UUID string.
    pub user_id: String,
    /// Plan tier: "individual", "family", "professional", "enterprise".
    pub tier: String,
    /// Active team memberships: map of team UUID → role ("owner"|"leader"|"member").
    pub teams: HashMap<String, String>,
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

/// Axum middleware that validates the `Authorization: Bearer` JWT when
/// `JWT_SECRET` is set, and inserts a `ClannAuth` extension into every request.
///
/// If `jwt_secret` is `None` (dev / test mode), auth is skipped and an
/// enterprise-tier `ClannAuth` is injected so plan limits are never triggered.
pub async fn jwt_middleware(
    mut request: Request,
    next: Next,
    jwt_secret: Option<String>,
) -> Response {
    let auth = match jwt_secret {
        None => {
            // Dev/test mode — no auth enforcement, unlimited plan
            ClannAuth {
                user_id: String::new(),
                tier: "enterprise".to_string(),
                teams: HashMap::new(),
            }
        }
        Some(secret) => {
            let token = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|s| s.to_string());

            match token {
                None => return unauthorized("Missing Authorization header"),
                Some(token) => {
                    match decode::<RawClaims>(
                        &token,
                        &DecodingKey::from_secret(secret.as_bytes()),
                        &Validation::default(),
                    ) {
                        Err(e) => return unauthorized(&format!("Invalid token: {e}")),
                        Ok(data) => {
                            let raw = data.claims;
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
                            let teams = raw
                                .teams
                                .into_iter()
                                .map(|(id, claim)| (id, claim.role))
                                .collect();
                            ClannAuth { user_id: raw.sub, tier, teams }
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
