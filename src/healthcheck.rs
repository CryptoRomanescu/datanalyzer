/// Health check and readiness module for container orchestration.
///
/// This module provides HTTP endpoints for:
/// - Health checks (liveness probe)
/// - Readiness checks (readiness probe)
/// - Prometheus metrics export
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Health status of the application
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Service is healthy and ready to serve requests
    Healthy,
    /// Service is unhealthy and should not receive traffic
    Unhealthy,
    /// Service is degraded but still functional
    Degraded,
}

/// Readiness status of the application
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadinessStatus {
    /// Service is ready to accept traffic
    Ready,
    /// Service is not ready to accept traffic
    NotReady,
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Overall health status
    pub status: HealthStatus,
    /// Human-readable message
    pub message: String,
    /// Timestamp of the check
    pub timestamp: String,
    /// Additional details
    pub details: HealthDetails,
}

/// Detailed health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthDetails {
    /// WebSocket connection status
    pub websocket_connected: bool,
    /// Number of active subscriptions
    pub active_subscriptions: u64,
    /// Number of problematic pools
    pub problematic_pools: u64,
    /// Uptime in seconds
    pub uptime_seconds: u64,
}

/// Readiness check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessResponse {
    /// Readiness status
    pub status: ReadinessStatus,
    /// Human-readable message
    pub message: String,
    /// Timestamp of the check
    pub timestamp: String,
}

/// Shared application state for health checks
#[derive(Clone)]
pub struct AppState {
    pub websocket_connected: Arc<RwLock<bool>>,
    pub active_subscriptions: Arc<RwLock<u64>>,
    pub problematic_pools: Arc<RwLock<u64>>,
    pub start_time: std::time::Instant,
    pub metrics_registry: Arc<prometheus::Registry>,
}

impl AppState {
    /// Create a new AppState instance
    pub fn new(metrics_registry: Arc<prometheus::Registry>) -> Self {
        Self {
            websocket_connected: Arc::new(RwLock::new(false)),
            active_subscriptions: Arc::new(RwLock::new(0)),
            problematic_pools: Arc::new(RwLock::new(0)),
            start_time: std::time::Instant::now(),
            metrics_registry,
        }
    }

    /// Update WebSocket connection status
    pub async fn set_websocket_connected(&self, connected: bool) {
        *self.websocket_connected.write().await = connected;
    }

    /// Update active subscriptions count
    pub async fn set_active_subscriptions(&self, count: u64) {
        *self.active_subscriptions.write().await = count;
    }

    /// Update problematic pools count
    pub async fn set_problematic_pools(&self, count: u64) {
        *self.problematic_pools.write().await = count;
    }
}

/// Health check endpoint handler
async fn health_check(State(state): State<Arc<AppState>>) -> Response {
    let websocket_connected = *state.websocket_connected.read().await;
    let active_subscriptions = *state.active_subscriptions.read().await;
    let problematic_pools = *state.problematic_pools.read().await;
    let uptime = state.start_time.elapsed().as_secs();

    let (status, message) = if websocket_connected {
        if problematic_pools > 0 {
            (
                HealthStatus::Degraded,
                "WebSocket connected but some pools are problematic",
            )
        } else {
            (HealthStatus::Healthy, "All systems operational")
        }
    } else {
        (HealthStatus::Unhealthy, "WebSocket not connected")
    };

    let response = HealthResponse {
        status: status.clone(),
        message: message.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        details: HealthDetails {
            websocket_connected,
            active_subscriptions,
            problematic_pools,
            uptime_seconds: uptime,
        },
    };

    let status_code = match status {
        HealthStatus::Healthy => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Unhealthy => StatusCode::SERVICE_UNAVAILABLE,
    };

    (status_code, Json(response)).into_response()
}

/// Readiness check endpoint handler
async fn readiness_check(State(state): State<Arc<AppState>>) -> Response {
    let websocket_connected = *state.websocket_connected.read().await;
    let active_subscriptions = *state.active_subscriptions.read().await;

    let (status, message, status_code) = if websocket_connected && active_subscriptions > 0 {
        (
            ReadinessStatus::Ready,
            "Service is ready to accept traffic",
            StatusCode::OK,
        )
    } else if websocket_connected {
        (
            ReadinessStatus::NotReady,
            "WebSocket connected but no active subscriptions",
            StatusCode::SERVICE_UNAVAILABLE,
        )
    } else {
        (
            ReadinessStatus::NotReady,
            "WebSocket not connected",
            StatusCode::SERVICE_UNAVAILABLE,
        )
    };

    let response = ReadinessResponse {
        status,
        message: message.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    (status_code, Json(response)).into_response()
}

/// Metrics endpoint handler
async fn metrics_handler(State(state): State<Arc<AppState>>) -> Response {
    use prometheus::Encoder;

    let encoder = prometheus::TextEncoder::new();
    let metric_families = state.metrics_registry.gather();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {}", e),
        )
            .into_response();
    }

    match String::from_utf8(buffer) {
        Ok(metrics_text) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; version=0.0.4",
            )],
            metrics_text,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to convert metrics to string: {}", e),
        )
            .into_response(),
    }
}

/// Create the health check router
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/healthz", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/readyz", get(readiness_check))
        .route("/metrics", get(metrics_handler))
        .with_state(state)
}

/// Start the health check server
///
/// # Arguments
///
/// * `addr` - The address to bind to (e.g., "0.0.0.0:3000")
/// * `state` - Shared application state
///
/// # Returns
///
/// A future that runs the server
pub async fn start_server(
    addr: SocketAddr,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = create_router(state);

    tracing::info!("Starting health check server on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn create_test_state() -> Arc<AppState> {
        let registry = Arc::new(prometheus::Registry::new());
        Arc::new(AppState::new(registry))
    }

    #[tokio::test]
    async fn test_health_check_healthy() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;
        state.set_active_subscriptions(5).await;
        state.set_problematic_pools(0).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_check_unhealthy() {
        let state = create_test_state();
        state.set_websocket_connected(false).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_health_check_degraded() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;
        state.set_problematic_pools(3).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_readiness_check_ready() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;
        state.set_active_subscriptions(1).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_readiness_check_not_ready_no_connection() {
        let state = create_test_state();
        state.set_websocket_connected(false).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_readiness_check_not_ready_no_subscriptions() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;
        state.set_active_subscriptions(0).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_healthz_endpoint() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_readyz_endpoint() {
        let state = create_test_state();
        state.set_websocket_connected(true).await;
        state.set_active_subscriptions(1).await;

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let state = create_test_state();

        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
