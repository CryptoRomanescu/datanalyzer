# WebSocket Observability and Reliability Features

This document describes the observability, metrics, and reliability features implemented in Stage 2.

## Features

### 1. Prometheus Metrics

Comprehensive metrics collection for monitoring WebSocket operations:

#### Connection Metrics
- `websocket_connections_total` - Total connection attempts
- `websocket_connections_successful` - Successful connections
- `websocket_connections_failed` - Failed connection attempts
- `websocket_connection_state` - Current connection state (1=connected, 0=disconnected)

#### Reconnection Metrics
- `websocket_reconnections_total` - Total reconnection attempts
- `websocket_reconnections_successful` - Successful reconnections
- `websocket_reconnections_failed` - Failed reconnection attempts
- `websocket_reconnection_delay_seconds` - Histogram of reconnection delays

#### Subscription Metrics
- `websocket_active_subscriptions` - Current number of active subscriptions
- `websocket_subscriptions_total` - Total subscription attempts
- `websocket_subscription_failures` - Failed subscription attempts
- `websocket_problematic_pools` - Number of pools marked as problematic

#### Notification Metrics
- `websocket_notifications_received` - Total notifications received
- `websocket_notifications_throttled` - Notifications skipped due to throttling
- `websocket_notifications_dropped` - Notifications dropped due to errors

#### Cache Metrics (Price Fetcher)
- `price_fetcher_cache_hits` - Total cache hits
- `price_fetcher_cache_misses` - Total cache misses

#### Performance Metrics
- `websocket_operation_latency_seconds` - Histogram of operation latencies

### 2. Healthcheck Endpoints

HTTP endpoints for container orchestration and monitoring:

#### `/health` or `/healthz` (Liveness Probe)
Returns the health status of the application:
- `200 OK` - System is healthy or degraded but operational
- `503 Service Unavailable` - System is unhealthy

Example response:
```json
{
  "status": "Healthy",
  "message": "All systems operational",
  "timestamp": "2025-10-25T12:00:00Z",
  "details": {
    "websocket_connected": true,
    "active_subscriptions": 5,
    "problematic_pools": 0,
    "uptime_seconds": 3600
  }
}
```

#### `/ready` or `/readyz` (Readiness Probe)
Returns the readiness status:
- `200 OK` - Ready to accept traffic
- `503 Service Unavailable` - Not ready

Example response:
```json
{
  "status": "Ready",
  "message": "Service is ready to accept traffic",
  "timestamp": "2025-10-25T12:00:00Z"
}
```

#### `/metrics` (Prometheus Metrics)
Returns metrics in Prometheus text format for scraping.

### 3. Reconnection Strategy with Jitter

Improved reconnection strategy with:
- **Exponential backoff**: Delays increase exponentially (1s, 2s, 4s, 8s, ...)
- **Jitter**: Random variation (±20%) to prevent thundering herd
- **Max delay cap**: Maximum delay of 30 seconds
- **Configurable parameters**: Customize initial delay, max delay, and multiplier

### 4. Structured Logging

Replaced `log` with `tracing` for:
- Structured, machine-readable logs
- Configurable log levels via `RUST_LOG` environment variable
- Better integration with distributed tracing systems
- Contextual information in log messages

### 5. Connection State Management

Unified source of truth for connection state:
- `is_connected` flag synchronized with `client` Option
- Metrics automatically updated on state changes
- Consistent state tracking across all operations

## Usage Examples

### Basic Setup with Metrics

```rust
use datanalyzer::{
    metrics::WebSocketMetrics,
    websocket::WebSocketManager,
    healthcheck::{self, AppState},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Create metrics
    let metrics = Arc::new(WebSocketMetrics::new()?);

    // Create WebSocket manager with metrics
    let mut ws_manager = WebSocketManager::with_metrics(
        "wss://api.mainnet-beta.solana.com".to_string(),
        1000,
        Arc::clone(&metrics),
    );

    // Create app state for health checks
    let app_state = Arc::new(AppState::new(metrics.registry()));

    // Start healthcheck server
    let healthcheck_addr = "0.0.0.0:3000".parse()?;
    tokio::spawn({
        let state = Arc::clone(&app_state);
        async move {
            healthcheck::start_server(healthcheck_addr, state).await
        }
    });

    // Connect and monitor
    ws_manager.connect().await?;
    app_state.set_websocket_connected(true).await;

    Ok(())
}
```

### Custom Reconnection Strategy

```rust
use datanalyzer::websocket::ReconnectStrategy;
use std::time::Duration;

// Custom backoff parameters
let strategy = ReconnectStrategy::with_params(
    Duration::from_millis(500),  // initial delay
    Duration::from_secs(60),      // max delay
    3.0,                          // multiplier
);
```

### Accessing Metrics

```rust
// Get metrics as Prometheus text format
let metrics_text = metrics.gather();
println!("{}", metrics_text);

// Record custom metrics
metrics.record_notification_received();
metrics.record_cache_hit();
metrics.set_active_subscriptions(10);
```

### Configuring Log Levels

Set the `RUST_LOG` environment variable:

```bash
# Show all info-level logs
export RUST_LOG=info

# Show debug logs for datanalyzer, info for others
export RUST_LOG=datanalyzer=debug,info

# Show only errors
export RUST_LOG=error
```

## Kubernetes/Docker Integration

### Liveness Probe Configuration

```yaml
livenessProbe:
  httpGet:
    path: /healthz
    port: 3000
  initialDelaySeconds: 10
  periodSeconds: 30
  timeoutSeconds: 5
  failureThreshold: 3
```

### Readiness Probe Configuration

```yaml
readinessProbe:
  httpGet:
    path: /readyz
    port: 3000
  initialDelaySeconds: 5
  periodSeconds: 10
  timeoutSeconds: 3
  failureThreshold: 3
```

### Prometheus Scraping Configuration

```yaml
scrape_configs:
  - job_name: 'datanalyzer'
    static_configs:
      - targets: ['datanalyzer:3000']
    metrics_path: '/metrics'
    scrape_interval: 15s
```

## Monitoring Dashboards

### Key Metrics to Monitor

1. **Connection Stability**
   - `websocket_connection_state`
   - `websocket_reconnections_total`
   - `websocket_connections_failed`

2. **Performance**
   - `websocket_operation_latency_seconds`
   - `websocket_notifications_throttled / websocket_notifications_received` (throttle rate)
   - `price_fetcher_cache_hits / (price_fetcher_cache_hits + price_fetcher_cache_misses)` (cache hit rate)

3. **Reliability**
   - `websocket_active_subscriptions`
   - `websocket_problematic_pools`
   - `websocket_subscription_failures`

4. **Notifications**
   - `websocket_notifications_received`
   - `websocket_notifications_dropped`
   - `websocket_notifications_throttled`

## Testing

Run the observability demo:

```bash
cargo run --example observability_demo
```

Then access the endpoints:

```bash
# Health check
curl http://localhost:3000/health

# Readiness check
curl http://localhost:3000/ready

# Prometheus metrics
curl http://localhost:3000/metrics
```

Run integration tests:

```bash
cargo test observability
```

## Acceptance Criteria Status

- ✅ Connection stability on long tests (exponential backoff with jitter, reconnection metrics)
- ✅ Prometheus metrics export (comprehensive metrics module)
- ✅ Healthcheck OK (liveness and readiness endpoints)
- ✅ WebSocket doesn't lose subscriptions on reconnect (resubscribe_all mechanism)
- ✅ Readable logs with separated levels (structured tracing with configurable levels)
- ✅ Source of truth for connection state unified (is_connected synchronized with client)
- ✅ Reconnection statistics tracking (reconnection metrics)
- ✅ Jitter added to backoff strategy (prevents thundering herd)

## Future Enhancements

Potential improvements for future stages:

1. **Distributed Tracing**: Integration with OpenTelemetry for end-to-end tracing
2. **Alerting**: Webhook notifications for critical events
3. **Dashboard Templates**: Pre-built Grafana dashboards
4. **Circuit Breaker**: Automatic failover for problematic endpoints
5. **Adaptive Throttling**: Dynamic adjustment based on load
