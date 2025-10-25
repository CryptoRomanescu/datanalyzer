/// Example demonstrating WebSocket observability features
///
/// This example shows how to:
/// - Set up WebSocket manager with metrics
/// - Start healthcheck server
/// - Monitor connection health
/// - Export Prometheus metrics

use datanalyzer::{healthcheck, metrics::WebSocketMetrics, websocket::WebSocketManager};
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    tracing::info!("Starting WebSocket observability demo");

    // Create metrics collector
    let metrics = Arc::new(WebSocketMetrics::new()?);

    // Create WebSocket manager with metrics
    let mut ws_manager = WebSocketManager::with_metrics(
        "wss://api.mainnet-beta.solana.com".to_string(),
        1000, // 1 second snapshot interval
        Arc::clone(&metrics),
    );

    // Create application state for health checks
    let app_state = Arc::new(healthcheck::AppState::new(metrics.registry()));

    // Start healthcheck server on port 3000
    let healthcheck_addr: SocketAddr = "0.0.0.0:3000".parse()?;
    let state_clone = Arc::clone(&app_state);
    tokio::spawn(async move {
        if let Err(e) = healthcheck::start_server(healthcheck_addr, state_clone).await {
            tracing::error!("Health check server error: {}", e);
        }
    });

    tracing::info!("Health check server started on http://0.0.0.0:3000");
    tracing::info!("  - Health: http://0.0.0.0:3000/health");
    tracing::info!("  - Readiness: http://0.0.0.0:3000/ready");
    tracing::info!("  - Metrics: http://0.0.0.0:3000/metrics");

    // Connect to WebSocket
    match ws_manager.connect().await {
        Ok(()) => {
            tracing::info!("Connected to WebSocket successfully");
            app_state.set_websocket_connected(true).await;

            // In a real application, you would subscribe to pools here
            // For demo, we'll just show the connection is working
            
            // Keep the program running to allow checking the endpoints
            tracing::info!("Demo running. Press Ctrl+C to exit.");
            tracing::info!("Try accessing the health endpoints:");
            tracing::info!("  curl http://localhost:3000/health");
            tracing::info!("  curl http://localhost:3000/ready");
            tracing::info!("  curl http://localhost:3000/metrics");
            
            tokio::signal::ctrl_c().await?;
            tracing::info!("Shutting down...");
        }
        Err(e) => {
            tracing::error!("Failed to connect to WebSocket: {}", e);
            app_state.set_websocket_connected(false).await;
        }
    }

    // Print final metrics
    tracing::info!("=== Final Metrics ===");
    let metrics_output = metrics.gather();
    println!("{}", metrics_output);

    Ok(())
}
