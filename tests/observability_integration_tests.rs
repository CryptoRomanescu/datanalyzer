/// Integration tests for WebSocket observability features
///
/// These tests verify:
/// - Metrics collection
/// - Healthcheck endpoints
/// - Connection state tracking
/// - Reconnection with metrics

#[cfg(test)]
mod observability_tests {
    use datanalyzer::{
        healthcheck::AppState, metrics::WebSocketMetrics, websocket::WebSocketManager,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn test_websocket_manager_with_metrics() {
        let metrics = Arc::new(WebSocketMetrics::new().unwrap());
        let _manager = WebSocketManager::with_metrics(
            "wss://api.mainnet-beta.solana.com".to_string(),
            1000,
            Arc::clone(&metrics),
        );

        // Metrics should be initialized
        assert_eq!(metrics.connections_total.get(), 0);
        assert_eq!(metrics.connection_state.get(), 0);
    }

    #[tokio::test]
    async fn test_metrics_on_connection_attempt() {
        let metrics = Arc::new(WebSocketMetrics::new().unwrap());
        let mut manager = WebSocketManager::with_metrics(
            "wss://invalid-endpoint-for-testing.example.com".to_string(),
            1000,
            Arc::clone(&metrics),
        );

        // Attempt to connect (will fail)
        let _ = manager.connect().await;

        // Should have recorded a connection attempt
        assert_eq!(metrics.connections_total.get(), 1);
        // Should have recorded a failure (since endpoint is invalid)
        assert_eq!(metrics.connections_failed.get(), 1);
        // Connection state should be disconnected
        assert_eq!(metrics.connection_state.get(), 0);
    }

    #[tokio::test]
    async fn test_app_state_updates() {
        let registry = Arc::new(prometheus::Registry::new());
        let state = AppState::new(registry);

        // Initial state
        assert_eq!(*state.websocket_connected.read().await, false);
        assert_eq!(*state.active_subscriptions.read().await, 0);
        assert_eq!(*state.problematic_pools.read().await, 0);

        // Update state
        state.set_websocket_connected(true).await;
        state.set_active_subscriptions(5).await;
        state.set_problematic_pools(2).await;

        // Verify updates
        assert_eq!(*state.websocket_connected.read().await, true);
        assert_eq!(*state.active_subscriptions.read().await, 5);
        assert_eq!(*state.problematic_pools.read().await, 2);
    }

    #[tokio::test]
    async fn test_metrics_gather() {
        let metrics = WebSocketMetrics::new().unwrap();

        // Record some metrics
        metrics.record_connection_attempt();
        metrics.record_connection_success();
        metrics.record_subscription_attempt();
        metrics.set_active_subscriptions(3);
        metrics.record_notification_received();
        metrics.record_notification_throttled();

        // Gather metrics
        let output = metrics.gather();

        // Verify metrics are present in output
        assert!(output.contains("websocket_connections_total 1"));
        assert!(output.contains("websocket_connections_successful 1"));
        assert!(output.contains("websocket_subscriptions_total 1"));
        assert!(output.contains("websocket_active_subscriptions 3"));
        assert!(output.contains("websocket_notifications_received 1"));
        assert!(output.contains("websocket_notifications_throttled 1"));
    }

    #[tokio::test]
    async fn test_reconnection_metrics() {
        let metrics = Arc::new(WebSocketMetrics::new().unwrap());

        // Simulate reconnection attempts
        metrics.record_reconnection_attempt();
        metrics.record_reconnection_failure();
        metrics.record_reconnection_attempt();
        metrics.record_reconnection_success();

        assert_eq!(metrics.reconnections_total.get(), 2);
        assert_eq!(metrics.reconnections_failed.get(), 1);
        assert_eq!(metrics.reconnections_successful.get(), 1);
    }

    #[tokio::test]
    async fn test_cache_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();

        // Record cache operations
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        assert_eq!(metrics.cache_hits.get(), 2);
        assert_eq!(metrics.cache_misses.get(), 1);
    }

    #[tokio::test]
    async fn test_latency_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();

        // Record some latencies
        metrics.record_operation_latency(0.5);
        metrics.record_operation_latency(0.1);
        metrics.record_reconnection_delay(2.0);

        // Just verify these don't panic - histogram values are more complex to test
        let output = metrics.gather();
        assert!(output.contains("websocket_operation_latency_seconds"));
        assert!(output.contains("websocket_reconnection_delay_seconds"));
    }

    #[tokio::test]
    async fn test_problematic_pools_metric() {
        let metrics = WebSocketMetrics::new().unwrap();

        // Initially no problematic pools
        assert_eq!(metrics.problematic_pools.get(), 0);

        // Mark some pools as problematic
        metrics.set_problematic_pools(3);
        assert_eq!(metrics.problematic_pools.get(), 3);

        // Clear problematic pools
        metrics.set_problematic_pools(0);
        assert_eq!(metrics.problematic_pools.get(), 0);
    }

    #[test]
    fn test_metrics_default() {
        let metrics = WebSocketMetrics::default();

        // Should be initialized with zeros
        assert_eq!(metrics.connections_total.get(), 0);
        assert_eq!(metrics.connection_state.get(), 0);
        assert_eq!(metrics.active_subscriptions.get(), 0);
    }

    #[tokio::test]
    async fn test_reconnection_with_jitter() {
        use datanalyzer::websocket::ReconnectStrategy;
        use std::time::Duration;

        let mut strategy = ReconnectStrategy::new();

        // Get multiple delays and verify they have jitter
        let delay1 = strategy.next_delay();
        let delay2 = strategy.next_delay();
        let delay3 = strategy.next_delay();

        // Delays should be within expected ranges (with jitter)
        assert!(delay1 >= Duration::from_millis(800));
        assert!(delay1 <= Duration::from_millis(1200));

        assert!(delay2 >= Duration::from_millis(1600));
        assert!(delay2 <= Duration::from_millis(2400));

        assert!(delay3 >= Duration::from_millis(3200));
        assert!(delay3 <= Duration::from_millis(4800));
    }

    #[tokio::test]
    async fn test_state_uptime() {
        let registry = Arc::new(prometheus::Registry::new());
        let state = AppState::new(registry);

        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Uptime should be greater than or equal to 0
        let uptime = state.start_time.elapsed().as_millis();
        assert!(uptime >= 100); // Should be at least 100ms since we waited
    }
}
