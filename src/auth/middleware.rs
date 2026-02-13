//! Axum middleware for authentication and authorization.
//!
//! Provides route-layer middleware functions that check Bearer tokens
//! against the AuthStore and verify capabilities.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::types::{AuthContext, Capability};
use crate::service::AppState;

/// Extract Bearer token from Authorization header.
fn extract_bearer_token(req: &Request) -> Option<&str> {
    let header = req.headers().get("authorization")?.to_str().ok()?;
    header.strip_prefix("Bearer ")
}

/// Middleware: require a valid session with `DhtRead` capability.
pub async fn require_dht_read(State(state): State<AppState>, req: Request, next: Next) -> Response {
    check_capability(state, req, next, Capability::DhtRead).await
}

/// Middleware: require a valid session with `DhtWrite` capability.
pub async fn require_dht_write(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    check_capability(state, req, next, Capability::DhtWrite).await
}

/// Middleware: require a valid session with `K2` capability.
pub async fn require_k2(State(state): State<AppState>, req: Request, next: Next) -> Response {
    check_capability(state, req, next, Capability::K2).await
}

/// Middleware: require the admin secret as Bearer token.
pub async fn require_admin_secret(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let Some(token) = extract_bearer_token(&req) else {
        return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
    };

    let Some(ref secret) = state.configuration.admin_secret else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Auth not configured").into_response();
    };

    if token != secret {
        return (StatusCode::UNAUTHORIZED, "Invalid admin secret").into_response();
    }

    next.run(req).await
}

/// Check that the request has a valid session token with the required capability.
async fn check_capability(
    state: AppState,
    mut req: Request,
    next: Next,
    required: Capability,
) -> Response {
    let Some(auth_store) = &state.auth_store else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Auth not configured").into_response();
    };

    let Some(token) = extract_bearer_token(&req) else {
        return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
    };

    let Some(session) = auth_store.validate_session(token).await else {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired session token").into_response();
    };

    if !session.has_capability(required) {
        return (
            StatusCode::FORBIDDEN,
            "Insufficient capabilities for this endpoint",
        )
            .into_response();
    }

    // Inject AuthContext into request extensions
    req.extensions_mut().insert(AuthContext {
        agent_pubkey: session.agent_pubkey,
        capabilities: session.capabilities,
    });

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AllowedAgent, AuthStore};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;
    use holochain_types::prelude::AgentPubKey;
    use std::collections::HashSet;
    use std::time::Duration;
    use tower::ServiceExt;

    fn test_agent(seed: u8) -> AgentPubKey {
        AgentPubKey::from_raw_32(vec![seed; 32])
    }

    /// Build a minimal AppState with auth enabled for testing.
    fn test_app_state(auth_store: AuthStore) -> AppState {
        use crate::agent_proxy::AgentProxyManager;
        use crate::config::Configuration;
        use crate::routes::kitsune::KitsuneState;
        use std::sync::Arc;

        let config = Configuration {
            admin_secret: Some("test-secret".to_string()),
            ..Default::default()
        };

        AppState {
            configuration: config,
            agent_proxy: AgentProxyManager::new(),
            gateway_kitsune: None,
            kitsune_state: Arc::new(KitsuneState {
                enabled: false,
                bootstrap_url: None,
                relay_url: None,
                kitsune: None,
            }),
            app_conn: None,
            temp_op_store: None,
            #[cfg(not(feature = "conductor-dht"))]
            dht_query: None,
            auth_store: Some(auth_store),
        }
    }

    async fn dummy_handler() -> &'static str {
        "ok"
    }

    fn build_test_router(state: AppState) -> Router {
        Router::new()
            .route(
                "/read",
                get(dummy_handler).route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    require_dht_read,
                )),
            )
            .route(
                "/write",
                get(dummy_handler).route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    require_dht_write,
                )),
            )
            .route(
                "/k2",
                get(dummy_handler).route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    require_k2,
                )),
            )
            .route(
                "/admin",
                get(dummy_handler).route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    require_admin_secret,
                )),
            )
            .with_state(state)
    }

    #[tokio::test]
    async fn test_missing_auth_header_returns_401() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/read")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_invalid_token_returns_401() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/read")
                    .header("authorization", "Bearer bogus-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_expired_token_returns_401() {
        let store = AuthStore::new(Duration::from_millis(1));
        store
            .add_agent(AllowedAgent {
                agent_pubkey: test_agent(1),
                capabilities: HashSet::from([Capability::DhtRead]),
                label: None,
            })
            .await;
        let token = store.create_session(&test_agent(1)).await.unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;

        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/read")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_wrong_capability_returns_403() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(AllowedAgent {
                agent_pubkey: test_agent(1),
                capabilities: HashSet::from([Capability::DhtRead]),
                label: None,
            })
            .await;
        let token = store.create_session(&test_agent(1)).await.unwrap();

        let state = test_app_state(store);
        let app = build_test_router(state);

        // Has DhtRead but trying to access DhtWrite
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/write")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_valid_token_with_correct_capability_passes() {
        let store = AuthStore::new(Duration::from_secs(3600));
        store
            .add_agent(AllowedAgent {
                agent_pubkey: test_agent(1),
                capabilities: HashSet::from([Capability::DhtRead]),
                label: None,
            })
            .await;
        let token = store.create_session(&test_agent(1)).await.unwrap();

        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/read")
                    .header("authorization", format!("Bearer {}", token.as_str()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_secret_valid() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/admin")
                    .header("authorization", "Bearer test-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_secret_invalid() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/admin")
                    .header("authorization", "Bearer wrong-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_admin_missing_header() {
        let store = AuthStore::new(Duration::from_secs(3600));
        let state = test_app_state(store);
        let app = build_test_router(state);

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
