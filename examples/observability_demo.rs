/// Example demonstrating WebSocket observability features
///
/// This example spins up WebSocket metrics, attempts a connection, records
/// reconnection metrics with jittered backoff, and prints Prometheus-formatted
/// output. It is aligned with src/websocket.rs, src/metrics.rs and tests.
///
/// Usage: cargo run --example observability_demo

use datanalyzer::{
    healthcheck::AppState,
    metrics::WebSocketMetrics,
    websocket::{ReconnectStrategy, WebSocketManager},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Initialize metrics and app state
    let metrics = Arc::new(WebSocketMetrics::new()?);
    let registry = Arc::new(prometheus::Registry::new());
    let state = AppState::new(Arc::clone(&registry));

    // Ensure initial state is reflected in metrics
    metrics.set_problematic_pools(0);
    metrics.set_active_subscriptions(0);

    // Create a WebSocket manager with metrics (use a known-invalid URL to avoid network dependency)
    let mut manager = WebSocketManager::with_metrics(
        "wss://invalid-endpoint-for-testing.example.com".to_string(),
        1000,
        Arc::clone(&metrics),
    );

    // Attempt a connection; this will fail and increment failure counters
    let _ = manager.connect().await;

    // Update health state based on metrics
    let connected = metrics.connection_state.get() == 1;
    state.set_websocket_connected(connected).await;
    state.set_active_subscriptions(metrics.active_subscriptions.get() as u64).await;
    state.set_problematic_pools(metrics.problematic_pools.get() as u64).await;

    // Demonstrate reconnection strategy with jitter
    let mut strategy = ReconnectStrategy::new();
    let delay1 = strategy.next_delay();
    let delay2 = strategy.next_delay();
    let delay3 = strategy.next_delay();
    log::info!(
        "Reconnection delays with jitter: {:?}, {:?}, {:?}",
        delay1, delay2, delay3
    );

    // Record reconnection metrics
    metrics.record_reconnection_attempt();
    metrics.record_reconnection_failure();
    metrics.record_reconnection_attempt();
    metrics.record_reconnection_success();

    // Simulate some cache activity and latencies
    metrics.record_cache_hit();
    metrics.record_cache_miss();
    metrics.record_operation_latency(0.12);
    metrics.record_reconnection_delay(1.8);

    // Print Prometheus exposition format to stdout
    let exposition = metrics.gather();
    println!("\n=== Prometheus Metrics ===\n{}", exposition);

    // Print simple health snapshot
    let ws_ok = *state.websocket_connected.read().await;
    let subs = *state.active_subscriptions.read().await;
    let probs = *state.problematic_pools.read().await;
    println!(
        "\n=== Health ===\nconnected: {}\nactive_subscriptions: {}\nproblematic_pools: {}\nuptime_ms: {}",
        ws_ok,
        subs,
        probs,
        state.start_time.elapsed().as_millis()
    );

    Ok(())
}