/// Prometheus metrics module for WebSocket monitoring and observability.
///
/// This module provides metrics collection for:
/// - WebSocket connection lifecycle (connects, disconnects, reconnects)
/// - Throttling statistics (notifications skipped)
/// - Subscription management (active subscriptions, failures)
/// - Performance metrics (latency, cache hit/miss)

use prometheus::{
    Histogram, IntCounter, IntGauge, Opts, Registry,
};
use std::sync::Arc;

/// Metrics collector for WebSocket operations
#[derive(Clone)]
pub struct WebSocketMetrics {
    /// Total number of connection attempts
    pub connections_total: IntCounter,
    
    /// Total number of successful connections
    pub connections_successful: IntCounter,
    
    /// Total number of failed connection attempts
    pub connections_failed: IntCounter,
    
    /// Total number of reconnection attempts
    pub reconnections_total: IntCounter,
    
    /// Total number of successful reconnections
    pub reconnections_successful: IntCounter,
    
    /// Total number of failed reconnection attempts
    pub reconnections_failed: IntCounter,
    
    /// Current connection state (1 = connected, 0 = disconnected)
    pub connection_state: IntGauge,
    
    /// Number of currently active subscriptions
    pub active_subscriptions: IntGauge,
    
    /// Total number of subscription attempts
    pub subscriptions_total: IntCounter,
    
    /// Total number of subscription failures
    pub subscription_failures: IntCounter,
    
    /// Total number of notifications received
    pub notifications_received: IntCounter,
    
    /// Total number of notifications skipped due to throttling
    pub notifications_throttled: IntCounter,
    
    /// Total number of notifications dropped (errors)
    pub notifications_dropped: IntCounter,
    
    /// Number of problematic pools
    pub problematic_pools: IntGauge,
    
    /// Price fetcher cache hits
    pub cache_hits: IntCounter,
    
    /// Price fetcher cache misses
    pub cache_misses: IntCounter,
    
    /// WebSocket operation latency histogram (in seconds)
    pub operation_latency: Histogram,
    
    /// Reconnection delay histogram (in seconds)
    pub reconnection_delay: Histogram,
    
    /// Registry for all metrics
    registry: Arc<Registry>,
}

impl WebSocketMetrics {
    /// Create a new WebSocketMetrics instance with a custom registry
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Arc::new(Registry::new());
        
        let connections_total = IntCounter::with_opts(
            Opts::new("websocket_connections_total", "Total number of connection attempts")
        )?;
        registry.register(Box::new(connections_total.clone()))?;
        
        let connections_successful = IntCounter::with_opts(
            Opts::new("websocket_connections_successful", "Total number of successful connections")
        )?;
        registry.register(Box::new(connections_successful.clone()))?;
        
        let connections_failed = IntCounter::with_opts(
            Opts::new("websocket_connections_failed", "Total number of failed connection attempts")
        )?;
        registry.register(Box::new(connections_failed.clone()))?;
        
        let reconnections_total = IntCounter::with_opts(
            Opts::new("websocket_reconnections_total", "Total number of reconnection attempts")
        )?;
        registry.register(Box::new(reconnections_total.clone()))?;
        
        let reconnections_successful = IntCounter::with_opts(
            Opts::new("websocket_reconnections_successful", "Total number of successful reconnections")
        )?;
        registry.register(Box::new(reconnections_successful.clone()))?;
        
        let reconnections_failed = IntCounter::with_opts(
            Opts::new("websocket_reconnections_failed", "Total number of failed reconnection attempts")
        )?;
        registry.register(Box::new(reconnections_failed.clone()))?;
        
        let connection_state = IntGauge::with_opts(
            Opts::new("websocket_connection_state", "Current connection state (1=connected, 0=disconnected)")
        )?;
        registry.register(Box::new(connection_state.clone()))?;
        
        let active_subscriptions = IntGauge::with_opts(
            Opts::new("websocket_active_subscriptions", "Number of currently active subscriptions")
        )?;
        registry.register(Box::new(active_subscriptions.clone()))?;
        
        let subscriptions_total = IntCounter::with_opts(
            Opts::new("websocket_subscriptions_total", "Total number of subscription attempts")
        )?;
        registry.register(Box::new(subscriptions_total.clone()))?;
        
        let subscription_failures = IntCounter::with_opts(
            Opts::new("websocket_subscription_failures", "Total number of subscription failures")
        )?;
        registry.register(Box::new(subscription_failures.clone()))?;
        
        let notifications_received = IntCounter::with_opts(
            Opts::new("websocket_notifications_received", "Total number of notifications received")
        )?;
        registry.register(Box::new(notifications_received.clone()))?;
        
        let notifications_throttled = IntCounter::with_opts(
            Opts::new("websocket_notifications_throttled", "Total number of notifications skipped due to throttling")
        )?;
        registry.register(Box::new(notifications_throttled.clone()))?;
        
        let notifications_dropped = IntCounter::with_opts(
            Opts::new("websocket_notifications_dropped", "Total number of notifications dropped due to errors")
        )?;
        registry.register(Box::new(notifications_dropped.clone()))?;
        
        let problematic_pools = IntGauge::with_opts(
            Opts::new("websocket_problematic_pools", "Number of pools marked as problematic")
        )?;
        registry.register(Box::new(problematic_pools.clone()))?;
        
        let cache_hits = IntCounter::with_opts(
            Opts::new("price_fetcher_cache_hits", "Total number of cache hits")
        )?;
        registry.register(Box::new(cache_hits.clone()))?;
        
        let cache_misses = IntCounter::with_opts(
            Opts::new("price_fetcher_cache_misses", "Total number of cache misses")
        )?;
        registry.register(Box::new(cache_misses.clone()))?;
        
        let operation_latency = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "websocket_operation_latency_seconds",
                "WebSocket operation latency in seconds"
            ).buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0])
        )?;
        registry.register(Box::new(operation_latency.clone()))?;
        
        let reconnection_delay = Histogram::with_opts(
            prometheus::HistogramOpts::new(
                "websocket_reconnection_delay_seconds",
                "Reconnection delay in seconds"
            ).buckets(vec![0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0])
        )?;
        registry.register(Box::new(reconnection_delay.clone()))?;
        
        Ok(Self {
            connections_total,
            connections_successful,
            connections_failed,
            reconnections_total,
            reconnections_successful,
            reconnections_failed,
            connection_state,
            active_subscriptions,
            subscriptions_total,
            subscription_failures,
            notifications_received,
            notifications_throttled,
            notifications_dropped,
            problematic_pools,
            cache_hits,
            cache_misses,
            operation_latency,
            reconnection_delay,
            registry,
        })
    }
    
    /// Get the Prometheus registry
    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }
    
    /// Record a connection attempt
    pub fn record_connection_attempt(&self) {
        self.connections_total.inc();
    }
    
    /// Record a successful connection
    pub fn record_connection_success(&self) {
        self.connections_successful.inc();
        self.connection_state.set(1);
    }
    
    /// Record a failed connection
    pub fn record_connection_failure(&self) {
        self.connections_failed.inc();
        self.connection_state.set(0);
    }
    
    /// Record a reconnection attempt
    pub fn record_reconnection_attempt(&self) {
        self.reconnections_total.inc();
    }
    
    /// Record a successful reconnection
    pub fn record_reconnection_success(&self) {
        self.reconnections_successful.inc();
        self.connection_state.set(1);
    }
    
    /// Record a failed reconnection
    pub fn record_reconnection_failure(&self) {
        self.reconnections_failed.inc();
    }
    
    /// Record disconnection
    pub fn record_disconnection(&self) {
        self.connection_state.set(0);
    }
    
    /// Update active subscriptions count
    pub fn set_active_subscriptions(&self, count: usize) {
        self.active_subscriptions.set(count as i64);
    }
    
    /// Record a subscription attempt
    pub fn record_subscription_attempt(&self) {
        self.subscriptions_total.inc();
    }
    
    /// Record a subscription failure
    pub fn record_subscription_failure(&self) {
        self.subscription_failures.inc();
    }
    
    /// Record a received notification
    pub fn record_notification_received(&self) {
        self.notifications_received.inc();
    }
    
    /// Record a throttled notification
    pub fn record_notification_throttled(&self) {
        self.notifications_throttled.inc();
    }
    
    /// Record a dropped notification
    pub fn record_notification_dropped(&self) {
        self.notifications_dropped.inc();
    }
    
    /// Update problematic pools count
    pub fn set_problematic_pools(&self, count: usize) {
        self.problematic_pools.set(count as i64);
    }
    
    /// Record a cache hit
    pub fn record_cache_hit(&self) {
        self.cache_hits.inc();
    }
    
    /// Record a cache miss
    pub fn record_cache_miss(&self) {
        self.cache_misses.inc();
    }
    
    /// Record operation latency
    pub fn record_operation_latency(&self, duration_secs: f64) {
        self.operation_latency.observe(duration_secs);
    }
    
    /// Record reconnection delay
    pub fn record_reconnection_delay(&self, duration_secs: f64) {
        self.reconnection_delay.observe(duration_secs);
    }
    
    /// Get all metrics as a string in Prometheus format
    pub fn gather(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        
        let mut buffer = Vec::new();
        encoder.encode(&metric_families, &mut buffer).unwrap();
        
        String::from_utf8(buffer).unwrap()
    }
}

impl Default for WebSocketMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to create WebSocketMetrics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_metrics_creation() {
        let metrics = WebSocketMetrics::new();
        assert!(metrics.is_ok());
    }
    
    #[test]
    fn test_connection_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_connection_attempt();
        assert_eq!(metrics.connections_total.get(), 1);
        
        metrics.record_connection_success();
        assert_eq!(metrics.connections_successful.get(), 1);
        assert_eq!(metrics.connection_state.get(), 1);
        
        metrics.record_connection_failure();
        assert_eq!(metrics.connections_failed.get(), 1);
        assert_eq!(metrics.connection_state.get(), 0);
    }
    
    #[test]
    fn test_reconnection_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_reconnection_attempt();
        assert_eq!(metrics.reconnections_total.get(), 1);
        
        metrics.record_reconnection_success();
        assert_eq!(metrics.reconnections_successful.get(), 1);
        assert_eq!(metrics.connection_state.get(), 1);
        
        metrics.record_reconnection_failure();
        assert_eq!(metrics.reconnections_failed.get(), 1);
    }
    
    #[test]
    fn test_subscription_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_subscription_attempt();
        assert_eq!(metrics.subscriptions_total.get(), 1);
        
        metrics.record_subscription_failure();
        assert_eq!(metrics.subscription_failures.get(), 1);
        
        metrics.set_active_subscriptions(5);
        assert_eq!(metrics.active_subscriptions.get(), 5);
    }
    
    #[test]
    fn test_notification_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_notification_received();
        assert_eq!(metrics.notifications_received.get(), 1);
        
        metrics.record_notification_throttled();
        assert_eq!(metrics.notifications_throttled.get(), 1);
        
        metrics.record_notification_dropped();
        assert_eq!(metrics.notifications_dropped.get(), 1);
    }
    
    #[test]
    fn test_cache_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_cache_hit();
        assert_eq!(metrics.cache_hits.get(), 1);
        
        metrics.record_cache_miss();
        assert_eq!(metrics.cache_misses.get(), 1);
    }
    
    #[test]
    fn test_latency_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_operation_latency(0.5);
        metrics.record_reconnection_delay(2.0);
        
        // Just verify these don't panic
        let output = metrics.gather();
        assert!(output.contains("websocket_operation_latency_seconds"));
        assert!(output.contains("websocket_reconnection_delay_seconds"));
    }
    
    #[test]
    fn test_gather_metrics() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.record_connection_attempt();
        metrics.record_connection_success();
        
        let output = metrics.gather();
        assert!(output.contains("websocket_connections_total"));
        assert!(output.contains("websocket_connections_successful"));
    }
    
    #[test]
    fn test_problematic_pools_metric() {
        let metrics = WebSocketMetrics::new().unwrap();
        
        metrics.set_problematic_pools(3);
        assert_eq!(metrics.problematic_pools.get(), 3);
        
        metrics.set_problematic_pools(0);
        assert_eq!(metrics.problematic_pools.get(), 0);
    }
}
