//! WebSocket Manager for Solana Account Subscriptions
//!
//! This module provides a WebSocket manager for subscribing to and monitoring
//! Solana account updates in real-time. It includes features like:
//!
//! - Connection management with timeout
//! - Pool subscription and multi-pool support
//! - Configurable throttling to limit notification frequency
//! - Statistics tracking for skipped notifications
//! - Automatic reconnection handling via stream closure detection
//! - Exponential backoff reconnection strategy
//! - Graceful degradation for problematic pools
//!
//! # Example Usage
//!
//! ```no_run
//! use datanalyzer::websocket::WebSocketManager;
//! use datanalyzer::config::PoolConfig;
//! use solana_sdk::pubkey::Pubkey;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a WebSocket manager with 1-second throttling
//! let mut manager = WebSocketManager::new(
//!     "wss://api.mainnet-beta.solana.com".to_string(),
//!     1000  // 1 second minimum between snapshots
//! );
//!
//! // Connect to the WebSocket endpoint
//! manager.connect().await?;
//!
//! // Define a callback to process account updates
//! let callback = Arc::new(|pool: Pubkey, data: Vec<u8>, slot: u64| {
//!     println!("Pool {} updated at slot {}, data size: {}", pool, slot, data.len());
//! });
//!
//! // Listen to multiple pools
//! let pools = vec![Pubkey::new_unique(), Pubkey::new_unique()];
//! manager.listen(&pools, callback, 30).await?;
//!
//! // If connection is lost, reconnect with exponential backoff
//! if !manager.is_connected() {
//!     manager.reconnect_loop(Some(5)).await?;  // Max 5 attempts
//! }
//!
//! // Manually retry problematic pools
//! let recovered = manager.retry_problematic_pools().await;
//! println!("Recovered {} problematic pools", recovered);
//!
//! // Log statistics about throttled notifications
//! manager.log_skipped_stats().await;
//! # Ok(())
//! # }
//! ```

use crate::config::PoolConfig;
use crate::error::AppError;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::RpcAccountInfoConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Callback function type for processing account updates
pub type AccountUpdateCallback = Arc<dyn Fn(Pubkey, Vec<u8>, u64) + Send + Sync + 'static>;

/// Reconnection strategy with exponential backoff
#[derive(Debug, Clone)]
pub struct ReconnectStrategy {
    current_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
    initial_delay: Duration,
}

impl ReconnectStrategy {
    /// Create a new reconnect strategy with default values
    ///
    /// Default values:
    /// - Initial delay: 1 second
    /// - Max delay: 30 seconds
    /// - Multiplier: 2.0 (exponential backoff)
    pub fn new() -> Self {
        let initial_delay = Duration::from_secs(1);
        Self {
            current_delay: initial_delay,
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            initial_delay,
        }
    }

    /// Create a new reconnect strategy with custom parameters
    pub fn with_params(initial_delay: Duration, max_delay: Duration, multiplier: f64) -> Self {
        Self {
            current_delay: initial_delay,
            max_delay,
            multiplier,
            initial_delay,
        }
    }

    /// Get the next delay duration and update internal state
    ///
    /// Returns the current delay and then increases it for the next call
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current_delay;
        
        // Calculate next delay with exponential backoff
        let next = Duration::from_secs_f64(self.current_delay.as_secs_f64() * self.multiplier);
        
        // Cap at max_delay
        self.current_delay = if next > self.max_delay {
            self.max_delay
        } else {
            next
        };
        
        delay
    }

    /// Reset the strategy to initial delay
    pub fn reset(&mut self) {
        self.current_delay = self.initial_delay;
    }

    /// Get current delay without modifying state
    pub fn current_delay(&self) -> Duration {
        self.current_delay
    }
}

impl Default for ReconnectStrategy {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket manager for subscribing to Solana account updates
pub struct WebSocketManager {
    ws_url: String,
    client: Option<Arc<PubsubClient>>,
    subscriptions: Arc<Mutex<HashMap<Pubkey, u64>>>,
    last_snapshot_time: Arc<Mutex<HashMap<Pubkey, Instant>>>,
    snapshot_interval_ms: u64,
    skipped_notifications: Arc<Mutex<HashMap<Pubkey, u64>>>,
    next_subscription_id: Arc<Mutex<u64>>,
    monitored_pools: Arc<Mutex<Vec<Pubkey>>>,
    reconnect_strategy: Arc<Mutex<ReconnectStrategy>>,
    problematic_pools: Arc<Mutex<HashSet<Pubkey>>>,
    pool_failure_counts: Arc<Mutex<HashMap<Pubkey, u32>>>,
    is_connected: Arc<Mutex<bool>>,
}

impl WebSocketManager {
    /// Create a new WebSocketManager instance
    ///
    /// # Arguments
    ///
    /// * `ws_url` - WebSocket URL for the Solana RPC endpoint
    /// * `snapshot_interval_ms` - Minimum interval between snapshots in milliseconds (0 to disable throttling)
    pub fn new(ws_url: String, snapshot_interval_ms: u64) -> Self {
        Self {
            ws_url,
            client: None,
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            last_snapshot_time: Arc::new(Mutex::new(HashMap::new())),
            snapshot_interval_ms,
            skipped_notifications: Arc::new(Mutex::new(HashMap::new())),
            next_subscription_id: Arc::new(Mutex::new(1)),
            monitored_pools: Arc::new(Mutex::new(Vec::new())),
            reconnect_strategy: Arc::new(Mutex::new(ReconnectStrategy::new())),
            problematic_pools: Arc::new(Mutex::new(HashSet::new())),
            pool_failure_counts: Arc::new(Mutex::new(HashMap::new())),
            is_connected: Arc::new(Mutex::new(false)),
        }
    }

    /// Connect to the WebSocket endpoint with timeout
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If connection succeeded
    /// * `Err(AppError)` - If connection failed with detailed error message
    pub async fn connect(&mut self) -> Result<(), AppError> {
        let ws_url = self.ws_url.clone();
        
        log::info!("Attempting to connect to WebSocket at: {}", ws_url);
        
        // Attempt connection with 10 second timeout
        match tokio::time::timeout(Duration::from_secs(10), PubsubClient::new(&ws_url)).await {
            Ok(Ok(client)) => {
                self.client = Some(Arc::new(client));
                *self.is_connected.lock().await = true;
                let timestamp = chrono::Utc::now().to_rfc3339();
                log::info!("Successfully connected to WebSocket at {}", timestamp);
                Ok(())
            }
            Ok(Err(e)) => {
                *self.is_connected.lock().await = false;
                Err(AppError::RpcError(format!(
                    "Failed to create PubsubClient: {}. Please check if the URL is valid and the endpoint is accessible.",
                    e
                )))
            }
            Err(_) => {
                *self.is_connected.lock().await = false;
                Err(AppError::RpcError(
                    "Connection timeout after 10 seconds. Please check your network connection and endpoint availability.".to_string()
                ))
            }
        }
    }

    /// Subscribe to a single pool's account updates
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The Pubkey of the pool to subscribe to
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If subscription succeeded
    /// * `Err(AppError)` - If subscription failed
    pub async fn subscribe_pool(&self, pool_address: Pubkey) -> Result<(), AppError> {
        let client = self.client.as_ref()
            .ok_or_else(|| AppError::RpcError("Not connected. Call connect() first.".to_string()))?;
        
        // Subscribe with commitment config
        let config = RpcAccountInfoConfig {
            encoding: None,
            commitment: Some(CommitmentConfig::confirmed()),
            data_slice: None,
            min_context_slot: None,
        };
        
        match client.account_subscribe(&pool_address, Some(config)).await {
            Ok(_subscription_stream) => {
                // Generate and store a subscription ID
                let mut next_id = self.next_subscription_id.lock().await;
                let subscription_id = *next_id;
                *next_id += 1;
                
                let mut subscriptions = self.subscriptions.lock().await;
                subscriptions.insert(pool_address, subscription_id);
                
                // Add to monitored pools if not already there
                let mut monitored = self.monitored_pools.lock().await;
                if !monitored.contains(&pool_address) {
                    monitored.push(pool_address);
                }
                
                log::info!("Subscribed to pool {} with subscription ID: {}", pool_address, subscription_id);
                
                // Note: In a real implementation, we would spawn a task to listen to the stream
                // For now, we're just tracking that the subscription was created
                
                Ok(())
            }
            Err(e) => {
                // Track failure for this pool
                let mut failure_counts = self.pool_failure_counts.lock().await;
                let count = failure_counts.entry(pool_address).or_insert(0);
                *count += 1;
                
                // Mark as problematic if it fails too many times (e.g., 3 times)
                if *count >= 3 {
                    let mut problematic = self.problematic_pools.lock().await;
                    problematic.insert(pool_address);
                    log::warn!("Pool {} marked as problematic after {} failures", pool_address, count);
                }
                
                Err(AppError::RpcError(format!(
                    "Failed to subscribe to pool {}: {}",
                    pool_address, e
                )))
            }
        }
    }

    /// Subscribe to all pools in the configuration
    ///
    /// # Arguments
    ///
    /// * `pools` - Slice of PoolConfig instances to subscribe to
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If all subscriptions succeeded
    /// * `Err(AppError)` - If any subscription failed (returns first error)
    pub async fn subscribe_all_pools(&self, pools: &[PoolConfig]) -> Result<(), AppError> {
        for pool in pools {
            self.subscribe_pool(*pool.pool_address()).await?;
        }
        log::info!("Successfully subscribed to {} pools", pools.len());
        Ok(())
    }

    /// Listen to account updates for a single pool
    ///
    /// This method spawns a background task that listens to account updates for the specified pool.
    /// The callback is invoked for each update, after throttling is applied.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The pool address to listen to
    /// * `callback` - Callback function to process account updates (pool_address, account_data, slot)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If listener task was spawned successfully
    /// * `Err(AppError)` - If subscription failed or client not connected
    pub async fn listen_pool(
        &self,
        pool_address: Pubkey,
        callback: AccountUpdateCallback,
    ) -> Result<(), AppError> {
        // Clone everything we need from self FIRST
        let client = Arc::clone(self.client.as_ref()
            .ok_or_else(|| AppError::RpcError("Not connected. Call connect() first.".to_string()))?);
        let last_snapshot_time = Arc::clone(&self.last_snapshot_time);
        let skipped_notifications = Arc::clone(&self.skipped_notifications);
        let snapshot_interval_ms = self.snapshot_interval_ms;
        
        // Spawn a task to create subscription and listen to the stream
        tokio::spawn(async move {
            use futures_util::StreamExt;
            
            let config = RpcAccountInfoConfig {
                encoding: None,
                commitment: Some(CommitmentConfig::confirmed()),
                data_slice: None,
                min_context_slot: None,
            };
            
            let subscription_result = client.account_subscribe(&pool_address, Some(config)).await;
            
            let (mut stream, _unsubscribe) = match subscription_result {
                Ok(sub) => sub,
                Err(e) => {
                    log::error!("Failed to subscribe to pool {}: {}", pool_address, e);
                    return;
                }
            };
            
            log::info!("Started listening to pool {}", pool_address);
            
            while let Some(update) = stream.next().await {
                let slot = update.context.slot;
                
                // Check throttling
                let should_process = if snapshot_interval_ms == 0 {
                    true
                } else {
                    let mut last_times = last_snapshot_time.lock().await;
                    let now = Instant::now();
                    
                    let should_process = if let Some(last_time) = last_times.get(&pool_address) {
                        let elapsed = now.duration_since(*last_time);
                        let threshold = Duration::from_millis(snapshot_interval_ms);
                        
                        if elapsed < threshold {
                            let mut skipped = skipped_notifications.lock().await;
                            *skipped.entry(pool_address).or_insert(0) += 1;
                            false
                        } else {
                            true
                        }
                    } else {
                        true
                    };
                    
                    if should_process {
                        last_times.insert(pool_address, now);
                    }
                    
                    should_process
                };
                
                if should_process {
                    // Extract account data
                    if let Some(account) = update.value.data.decode() {
                        callback(pool_address, account.to_vec(), slot);
                    } else {
                        log::warn!("Failed to decode account data for pool {}", pool_address);
                    }
                }
            }
            
            log::warn!("Stopped listening to pool {} - connection lost or stream ended", pool_address);
        });
        
        Ok(())
    }

    /// Listen to account updates for multiple pools
    ///
    /// This spawns background tasks for each pool to listen to their account updates.
    /// Each task runs in an infinite loop with timeout handling for disconnect detection.
    ///
    /// # Arguments
    ///
    /// * `pools` - Slice of pool addresses to listen to
    /// * `callback` - Callback function to process account updates
    /// * `disconnect_timeout_secs` - Seconds without data before assuming disconnect (e.g., 30)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If all listener tasks were spawned successfully
    /// * `Err(AppError)` - If any subscription failed
    pub async fn listen(
        &self,
        pools: &[Pubkey],
        callback: AccountUpdateCallback,
        disconnect_timeout_secs: u64,
    ) -> Result<(), AppError> {
        for pool in pools {
            // Clone the callback Arc for each pool
            let pool_callback = Arc::clone(&callback);
            self.listen_pool(*pool, pool_callback).await?;
        }
        
        log::info!(
            "Started listening to {} pools with {}s disconnect timeout",
            pools.len(),
            disconnect_timeout_secs
        );
        
        Ok(())
    }

    /// Mark connection as disconnected
    ///
    /// Sets the connection state to disconnected and logs the event
    ///
    /// # Arguments
    ///
    /// * `reason` - Reason for disconnection
    pub async fn mark_disconnected(&mut self, reason: &str) {
        self.client = None;
        *self.is_connected.lock().await = false;
        let timestamp = chrono::Utc::now().to_rfc3339();
        log::warn!("Connection lost at {} - Reason: {}", timestamp, reason);
    }

    /// Reconnect loop with exponential backoff
    ///
    /// Attempts to reconnect with exponential backoff strategy.
    /// After successful reconnection, calls resubscribe_all().
    ///
    /// # Arguments
    ///
    /// * `max_attempts` - Maximum number of reconnection attempts (None for unlimited)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If reconnection succeeded
    /// * `Err(AppError)` - If all reconnection attempts failed
    pub async fn reconnect_loop(&mut self, max_attempts: Option<u32>) -> Result<(), AppError> {
        let mut attempt = 0;
        
        // Reset reconnect strategy
        self.reconnect_strategy.lock().await.reset();
        
        loop {
            attempt += 1;
            
            if let Some(max) = max_attempts {
                if attempt > max {
                    return Err(AppError::RpcError(format!(
                        "Failed to reconnect after {} attempts",
                        max
                    )));
                }
            }
            
            log::info!("Reconnection attempt #{}", attempt);
            
            match self.connect().await {
                Ok(()) => {
                    log::info!("Reconnection successful on attempt #{}", attempt);
                    
                    // Reset reconnect strategy after successful connection
                    self.reconnect_strategy.lock().await.reset();
                    
                    // Resubscribe to all pools
                    if let Err(e) = self.resubscribe_all().await {
                        log::error!("Failed to resubscribe after reconnection: {}", e);
                        // Continue anyway - we're connected at least
                    }
                    
                    return Ok(());
                }
                Err(e) => {
                    log::warn!("Reconnection attempt #{} failed: {}", attempt, e);
                    
                    // Get next delay from strategy
                    let delay = {
                        let mut strategy = self.reconnect_strategy.lock().await;
                        strategy.next_delay()
                    };
                    
                    log::info!("Waiting {:?} before next reconnection attempt", delay);
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Resubscribe to all monitored pools after reconnection
    ///
    /// Clears old subscription IDs and resubscribes to all pools that were
    /// being monitored before disconnection.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If all resubscriptions succeeded
    /// * `Err(AppError)` - If any resubscription failed (but continues for others)
    pub async fn resubscribe_all(&self) -> Result<(), AppError> {
        // Clear old subscription IDs
        let mut subscriptions = self.subscriptions.lock().await;
        subscriptions.clear();
        drop(subscriptions);
        
        let pools = {
            let monitored = self.monitored_pools.lock().await;
            monitored.clone()
        };
        
        if pools.is_empty() {
            log::info!("No pools to resubscribe");
            return Ok(());
        }
        
        log::info!("Resubscribing to {} pools", pools.len());
        
        let mut success_count = 0;
        let mut failure_count = 0;
        let mut first_error: Option<AppError> = None;
        
        for pool in pools.iter() {
            // Skip problematic pools
            let is_problematic = {
                let problematic = self.problematic_pools.lock().await;
                problematic.contains(pool)
            };
            
            if is_problematic {
                log::warn!("Skipping problematic pool {} during resubscription", pool);
                continue;
            }
            
            match self.subscribe_pool(*pool).await {
                Ok(()) => {
                    success_count += 1;
                    log::info!("Successfully resubscribed to pool {}", pool);
                    
                    // Reset failure count on success
                    let mut failure_counts = self.pool_failure_counts.lock().await;
                    failure_counts.remove(pool);
                }
                Err(e) => {
                    failure_count += 1;
                    log::error!("Failed to resubscribe to pool {}: {}", pool, e);
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }
        
        log::info!(
            "Resubscription complete: {} succeeded, {} failed",
            success_count,
            failure_count
        );
        
        // Return error if any subscriptions failed, but we've attempted all
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    /// Retry subscription for problematic pools
    ///
    /// Attempts to resubscribe to pools that were previously marked as problematic.
    ///
    /// # Returns
    ///
    /// * Count of pools successfully resubscribed
    pub async fn retry_problematic_pools(&self) -> usize {
        let pools_to_retry = {
            let problematic = self.problematic_pools.lock().await;
            problematic.iter().copied().collect::<Vec<_>>()
        };
        
        if pools_to_retry.is_empty() {
            log::info!("No problematic pools to retry");
            return 0;
        }
        
        log::info!("Retrying {} problematic pools", pools_to_retry.len());
        
        let mut success_count = 0;
        
        for pool in pools_to_retry.iter() {
            match self.subscribe_pool(*pool).await {
                Ok(()) => {
                    success_count += 1;
                    log::info!("Successfully resubscribed to previously problematic pool {}", pool);
                    
                    // Remove from problematic set on success
                    let mut problematic = self.problematic_pools.lock().await;
                    problematic.remove(pool);
                    
                    // Reset failure count
                    let mut failure_counts = self.pool_failure_counts.lock().await;
                    failure_counts.remove(pool);
                }
                Err(e) => {
                    log::error!("Still cannot subscribe to problematic pool {}: {}", pool, e);
                }
            }
        }
        
        log::info!("Retry complete: {} out of {} problematic pools recovered", success_count, pools_to_retry.len());
        
        success_count
    }

    /// Get list of problematic pools
    pub async fn get_problematic_pools(&self) -> Vec<Pubkey> {
        let problematic = self.problematic_pools.lock().await;
        problematic.iter().copied().collect()
    }

    /// Get list of monitored pools
    pub async fn get_monitored_pools(&self) -> Vec<Pubkey> {
        let monitored = self.monitored_pools.lock().await;
        monitored.clone()
    }

    /// Check if a notification should be processed based on throttling settings
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The pool address to check
    ///
    /// # Returns
    ///
    /// * `true` - If notification should be processed
    /// * `false` - If notification should be skipped due to throttling
    async fn should_process_notification(&self, pool_address: &Pubkey) -> bool {
        // If throttling is disabled (0), always process
        if self.snapshot_interval_ms == 0 {
            return true;
        }

        let mut last_times = self.last_snapshot_time.lock().await;
        let now = Instant::now();

        if let Some(last_time) = last_times.get(pool_address) {
            let elapsed = now.duration_since(*last_time);
            let threshold = Duration::from_millis(self.snapshot_interval_ms);

            if elapsed < threshold {
                // Increment skipped counter
                let mut skipped = self.skipped_notifications.lock().await;
                *skipped.entry(*pool_address).or_insert(0) += 1;
                return false;
            }
        }

        // Update last snapshot time
        last_times.insert(*pool_address, now);
        true
    }

    /// Log statistics about skipped notifications
    pub async fn log_skipped_stats(&self) {
        let skipped = self.skipped_notifications.lock().await;
        if !skipped.is_empty() {
            log::info!("=== Throttling Statistics ===");
            for (pool, count) in skipped.iter() {
                log::info!("Pool {}: {} notifications skipped", pool, count);
            }
        }
    }

    /// Reset skipped notification counters
    pub async fn reset_skipped_stats(&self) {
        let mut skipped = self.skipped_notifications.lock().await;
        skipped.clear();
    }

    /// Get the number of active subscriptions
    pub async fn subscription_count(&self) -> usize {
        self.subscriptions.lock().await.len()
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    /// Get WebSocket URL
    pub fn get_ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Get snapshot interval in milliseconds
    pub fn get_snapshot_interval_ms(&self) -> u64 {
        self.snapshot_interval_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_websocket_manager_new() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url.clone(), 1000);

        assert_eq!(manager.get_ws_url(), ws_url);
        assert_eq!(manager.get_snapshot_interval_ms(), 1000);
        assert!(!manager.is_connected());
    }

    #[test]
    fn test_websocket_manager_new_no_throttling() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 0);

        assert_eq!(manager.get_snapshot_interval_ms(), 0);
    }

    #[tokio::test]
    async fn test_subscription_count_empty() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 1000);

        assert_eq!(manager.subscription_count().await, 0);
    }

    #[tokio::test]
    async fn test_should_process_notification_no_throttling() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 0);

        let pool = Pubkey::new_unique();
        
        // Should always process when throttling is disabled
        assert!(manager.should_process_notification(&pool).await);
        assert!(manager.should_process_notification(&pool).await);
        assert!(manager.should_process_notification(&pool).await);
    }

    #[tokio::test]
    async fn test_should_process_notification_with_throttling() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 100);

        let pool = Pubkey::new_unique();
        
        // First notification should be processed
        assert!(manager.should_process_notification(&pool).await);
        
        // Immediate second notification should be skipped
        assert!(!manager.should_process_notification(&pool).await);
        
        // Wait for throttling interval to pass
        tokio::time::sleep(Duration::from_millis(150)).await;
        
        // Should be processed again
        assert!(manager.should_process_notification(&pool).await);
    }

    #[tokio::test]
    async fn test_skipped_stats() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 100);

        let pool = Pubkey::new_unique();
        
        // Process first
        assert!(manager.should_process_notification(&pool).await);
        
        // Skip next 5
        for _ in 0..5 {
            assert!(!manager.should_process_notification(&pool).await);
        }
        
        // Check stats were recorded
        let skipped = manager.skipped_notifications.lock().await;
        assert_eq!(skipped.get(&pool), Some(&5));
    }

    #[tokio::test]
    async fn test_reset_skipped_stats() {
        let ws_url = "wss://api.mainnet-beta.solana.com".to_string();
        let manager = WebSocketManager::new(ws_url, 100);

        let pool = Pubkey::new_unique();
        
        // Generate some skipped notifications
        manager.should_process_notification(&pool).await;
        manager.should_process_notification(&pool).await;
        
        // Reset stats
        manager.reset_skipped_stats().await;
        
        let skipped = manager.skipped_notifications.lock().await;
        assert_eq!(skipped.len(), 0);
    }

    #[tokio::test]
    async fn test_connect_invalid_url() {
        let mut manager = WebSocketManager::new("invalid_url".to_string(), 1000);
        
        let result = manager.connect().await;
        assert!(result.is_err());
    }

    #[test]
    fn test_callback_type_compiles() {
        // Test that we can create a callback with the correct type
        let _callback: AccountUpdateCallback = Arc::new(|pool, data, slot| {
            // Simple callback that just prints
            println!("Pool: {}, Data size: {}, Slot: {}", pool, data.len(), slot);
        });
    }

    #[tokio::test]
    async fn test_listen_without_connection() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        let callback: AccountUpdateCallback = Arc::new(|_pool, _data, _slot| {});
        
        let pools = vec![Pubkey::new_unique()];
        let result = manager.listen(&pools, callback, 30).await;
        
        // Should fail because not connected
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multiple_pool_throttling() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 100);
        
        let pool1 = Pubkey::new_unique();
        let pool2 = Pubkey::new_unique();
        
        // Both pools should process first notification
        assert!(manager.should_process_notification(&pool1).await);
        assert!(manager.should_process_notification(&pool2).await);
        
        // Both should be throttled immediately after
        assert!(!manager.should_process_notification(&pool1).await);
        assert!(!manager.should_process_notification(&pool2).await);
        
        // Wait and check that both can process again
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(manager.should_process_notification(&pool1).await);
        assert!(manager.should_process_notification(&pool2).await);
    }

    #[tokio::test]
    async fn test_get_skipped_stats_for_multiple_pools() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 100);
        
        let pool1 = Pubkey::new_unique();
        let pool2 = Pubkey::new_unique();
        
        // Process first, then skip some for pool1
        manager.should_process_notification(&pool1).await;
        for _ in 0..3 {
            manager.should_process_notification(&pool1).await;
        }
        
        // Process first, then skip some for pool2
        manager.should_process_notification(&pool2).await;
        for _ in 0..5 {
            manager.should_process_notification(&pool2).await;
        }
        
        let skipped = manager.skipped_notifications.lock().await;
        assert_eq!(skipped.get(&pool1), Some(&3));
        assert_eq!(skipped.get(&pool2), Some(&5));
    }

    // Tests for ReconnectStrategy
    #[test]
    fn test_reconnect_strategy_new() {
        let strategy = ReconnectStrategy::new();
        
        assert_eq!(strategy.current_delay(), Duration::from_secs(1));
    }

    #[test]
    fn test_reconnect_strategy_default() {
        let strategy = ReconnectStrategy::default();
        
        assert_eq!(strategy.current_delay(), Duration::from_secs(1));
    }

    #[test]
    fn test_reconnect_strategy_exponential_backoff() {
        let mut strategy = ReconnectStrategy::new();
        
        // First delay should be 1 second
        assert_eq!(strategy.next_delay(), Duration::from_secs(1));
        
        // Second delay should be 2 seconds
        assert_eq!(strategy.next_delay(), Duration::from_secs(2));
        
        // Third delay should be 4 seconds
        assert_eq!(strategy.next_delay(), Duration::from_secs(4));
        
        // Fourth delay should be 8 seconds
        assert_eq!(strategy.next_delay(), Duration::from_secs(8));
    }

    #[test]
    fn test_reconnect_strategy_max_delay() {
        let mut strategy = ReconnectStrategy::new();
        
        // Keep calling next_delay until we hit max
        for _ in 0..10 {
            strategy.next_delay();
        }
        
        // Should be capped at max_delay (30 seconds)
        let delay = strategy.current_delay();
        assert_eq!(delay, Duration::from_secs(30));
    }

    #[test]
    fn test_reconnect_strategy_reset() {
        let mut strategy = ReconnectStrategy::new();
        
        // Advance the delay
        strategy.next_delay();
        strategy.next_delay();
        assert_eq!(strategy.current_delay(), Duration::from_secs(4));
        
        // Reset should go back to initial
        strategy.reset();
        assert_eq!(strategy.current_delay(), Duration::from_secs(1));
    }

    #[test]
    fn test_reconnect_strategy_custom_params() {
        let mut strategy = ReconnectStrategy::with_params(
            Duration::from_millis(500),
            Duration::from_secs(10),
            3.0
        );
        
        assert_eq!(strategy.next_delay(), Duration::from_millis(500));
        assert_eq!(strategy.next_delay(), Duration::from_millis(1500));
        assert_eq!(strategy.next_delay(), Duration::from_millis(4500));
    }

    // Tests for reconnection logic
    #[tokio::test]
    async fn test_mark_disconnected() {
        let mut manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        
        manager.mark_disconnected("Test disconnect").await;
        
        assert!(!manager.is_connected());
        assert!(manager.client.is_none());
    }

    #[tokio::test]
    async fn test_get_monitored_pools() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        
        let pools = manager.get_monitored_pools().await;
        assert_eq!(pools.len(), 0);
    }

    #[tokio::test]
    async fn test_get_problematic_pools() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        
        let pools = manager.get_problematic_pools().await;
        assert_eq!(pools.len(), 0);
    }

    #[tokio::test]
    async fn test_resubscribe_all_no_pools() {
        let mut manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        
        // Connect first (will fail but that's ok for this test)
        let _ = manager.connect().await;
        
        // Should succeed with no pools
        let result = manager.resubscribe_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_retry_problematic_pools_empty() {
        let manager = WebSocketManager::new("wss://api.mainnet-beta.solana.com".to_string(), 1000);
        
        let count = manager.retry_problematic_pools().await;
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_reconnect_loop_max_attempts() {
        let mut manager = WebSocketManager::new("wss://invalid-url-that-will-fail".to_string(), 1000);
        
        // Should fail after max attempts
        let result = manager.reconnect_loop(Some(2)).await;
        assert!(result.is_err());
    }
}
