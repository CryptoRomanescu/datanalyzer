/// DEX decoder module for parsing liquidity pool account data.
/// 
/// This module provides traits and implementations for decoding various DEX
/// pool account structures on Solana. Each DEX has its own data layout,
/// and this module abstracts the decoding logic behind a common interface.

pub mod pumpfun;
pub mod raydium;

use crate::config::PoolConfig;
use crate::error::AppError;
use crate::models::DexType;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Trait for decoding DEX pool account data.
///
/// This trait defines the interface for extracting reserve amounts and validating
/// account data for different DEX implementations (Pump.fun, Raydium, etc.).
///
/// # Contract
///
/// Implementations of this trait must:
/// - Parse account data according to the specific DEX's data layout
/// - Return reserves as (base_reserve, quote_reserve) or (token_reserve, sol_reserve)
/// - Validate account data before decoding to prevent panics
/// - Use little-endian byte order for deserializing u64 values (standard for Solana)
/// - Be thread-safe (Send + Sync) for use in async contexts
///
/// # Example
///
/// ```ignore
/// use crate::dex::{DexDecoder, pumpfun::PumpFunDecoder};
///
/// let decoder = PumpFunDecoder;
/// let account_data: &[u8] = /* ... fetched from Solana RPC ... */;
///
/// // Validate the account data first
/// decoder.validate_account(account_data)?;
///
/// // Decode the reserves
/// let (token_reserve, sol_reserve) = decoder.decode_reserves(account_data)?;
/// println!("Token Reserve: {}, SOL Reserve: {}", token_reserve, sol_reserve);
/// ```
pub trait DexDecoder: Send + Sync {
    /// Decode reserve amounts from account data.
    ///
    /// Extracts the reserve amounts from the raw account data. For most DEXs,
    /// this returns (base_reserve, quote_reserve), typically representing
    /// (token_amount, sol_amount).
    ///
    /// # Arguments
    ///
    /// * `account_data` - Raw bytes from the Solana account
    ///
    /// # Returns
    ///
    /// * `Ok((u64, u64))` - Tuple of (base_reserve, quote_reserve)
    /// * `Err(AppError)` - If decoding fails due to invalid data, incorrect size, etc.
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Account data is too small to contain the required fields
    /// - Data is corrupted or cannot be properly deserialized
    /// - Reserved values are invalid (e.g., both reserves are zero)
    fn decode_reserves(&self, account_data: &[u8]) -> Result<(u64, u64), AppError>;

    /// Validate account data before decoding.
    ///
    /// Performs validation checks to ensure the account data is valid for this
    /// specific DEX. This should be called before `decode_reserves` to prevent
    /// errors and ensure data integrity.
    ///
    /// # Arguments
    ///
    /// * `account_data` - Raw bytes from the Solana account
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the account data is valid
    /// * `Err(AppError)` - If validation fails with detailed error message
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - Account size is incorrect
    /// - Discriminator/magic bytes don't match expected values
    /// - Critical fields contain invalid values
    fn validate_account(&self, account_data: &[u8]) -> Result<(), AppError>;
}

/// Create a decoder instance for the specified DEX type.
///
/// This factory function returns a boxed trait object implementing DexDecoder
/// based on the specified DEX type. The decoder is thread-safe (Send + Sync).
///
/// # Arguments
///
/// * `dex_type` - The type of DEX to create a decoder for
///
/// # Returns
///
/// * `Box<dyn DexDecoder>` - A boxed decoder instance for the specified DEX
///
/// # Example
///
/// ```ignore
/// use crate::dex::create_decoder;
/// use crate::models::DexType;
///
/// let decoder = create_decoder(DexType::PumpFun);
/// let (token_reserve, sol_reserve) = decoder.decode_reserves(&account_data)?;
/// ```
pub fn create_decoder(dex_type: DexType) -> Box<dyn DexDecoder> {
    match dex_type {
        DexType::PumpFun => Box::new(pumpfun::PumpFunDecoder),
        DexType::Raydium => Box::new(raydium::RaydiumDecoder),
    }
}

/// Statistics for a single pool decoder.
///
/// Tracks success and error counts for monitoring and debugging purposes.
#[derive(Debug, Clone, Default)]
pub struct DecoderStats {
    /// Number of successful decodings
    pub success_count: u64,
    
    /// Number of failed decodings
    pub error_count: u64,
    
    /// Timestamp of first successful decode (Unix epoch seconds)
    pub first_success_time: Option<u64>,
    
    /// Timestamp of last successful decode (Unix epoch seconds)
    pub last_success_time: Option<u64>,
    
    /// Timestamp of last error (Unix epoch seconds)
    pub last_error_time: Option<u64>,
}

impl DecoderStats {
    /// Create a new empty stats object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a successful decoding.
    pub fn record_success(&mut self) {
        self.success_count += 1;
        let now = current_timestamp();
        
        if self.first_success_time.is_none() {
            self.first_success_time = Some(now);
        }
        self.last_success_time = Some(now);
    }

    /// Record a failed decoding.
    pub fn record_error(&mut self) {
        self.error_count += 1;
        self.last_error_time = Some(current_timestamp());
    }

    /// Get the success rate as a percentage (0.0 to 100.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.error_count;
        if total == 0 {
            return 0.0;
        }
        (self.success_count as f64 / total as f64) * 100.0
    }

    /// Get the total number of decode attempts.
    pub fn total_attempts(&self) -> u64 {
        self.success_count + self.error_count
    }
}

/// Get current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Registry for managing decoders for multiple pools.
///
/// DecoderRegistry maintains a thread-safe map of pool addresses to their
/// corresponding decoder instances. It supports lazy initialization where
/// decoders are created only when first accessed.
///
/// # Thread Safety
///
/// The registry uses RwLock for concurrent access:
/// - Multiple readers can access different decoders simultaneously
/// - Write lock is only needed when creating new decoders
/// - Optimized for read-heavy workloads (common in monitoring scenarios)
///
/// # Example
///
/// ```ignore
/// use crate::dex::DecoderRegistry;
/// use crate::config::PoolConfig;
///
/// let registry = DecoderRegistry::new();
///
/// // Register pools
/// registry.register_pool(pool_config1)?;
/// registry.register_pool(pool_config2)?;
///
/// // Use decoder
/// let (base, quote) = registry.decode_pool_data(&pool_address, &account_data)?;
/// ```
pub struct DecoderRegistry {
    /// Map of pool addresses to their decoders (thread-safe)
    decoders: Arc<RwLock<HashMap<Pubkey, Box<dyn DexDecoder>>>>,
    
    /// Map of pool addresses to their DEX types (for lazy initialization)
    pool_types: Arc<RwLock<HashMap<Pubkey, DexType>>>,
    
    /// Map of pool addresses to their statistics
    stats: Arc<RwLock<HashMap<Pubkey, DecoderStats>>>,
    
    /// Timestamp when statistics were last logged
    last_stats_log: Arc<RwLock<u64>>,
}

impl DecoderRegistry {
    /// Create a new empty decoder registry.
    pub fn new() -> Self {
        Self {
            decoders: Arc::new(RwLock::new(HashMap::new())),
            pool_types: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(HashMap::new())),
            last_stats_log: Arc::new(RwLock::new(0)),
        }
    }

    /// Register a pool and create its decoder.
    ///
    /// This method immediately creates and stores a decoder for the pool.
    ///
    /// # Arguments
    ///
    /// * `pool_config` - Configuration containing pool address and DEX type
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If registration succeeded
    /// * `Err(AppError)` - If registration failed
    pub fn register_pool(&self, pool_config: PoolConfig) -> Result<(), AppError> {
        let pool_address = *pool_config.pool_address();
        let dex_type = pool_config.dex_type();
        
        // Create decoder for this pool
        let decoder = create_decoder(dex_type);
        
        // Store decoder and pool type
        let mut decoders = self.decoders.write()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire write lock: {}", e)))?;
        
        let mut pool_types = self.pool_types.write()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire write lock: {}", e)))?;
        
        decoders.insert(pool_address, decoder);
        pool_types.insert(pool_address, dex_type);
        
        Ok(())
    }

    /// Get a decoder for a specific pool.
    ///
    /// Returns a reference to the decoder if the pool is registered.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If decoder exists
    /// * `Err(AppError)` - If pool is not registered
    ///
    /// # Note
    ///
    /// This is primarily used for validation. For decoding, use `decode_pool_data`
    /// which is more ergonomic.
    pub fn get_decoder(&self, pool_address: &Pubkey) -> Result<(), AppError> {
        let decoders = self.decoders.read()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire read lock: {}", e)))?;
        
        if decoders.contains_key(pool_address) {
            Ok(())
        } else {
            Err(AppError::ConfigError(format!(
                "No decoder registered for pool: {}",
                pool_address
            )))
        }
    }

    /// Decode pool data using the appropriate decoder.
    ///
    /// This is a facade method that selects the correct decoder and decodes
    /// the account data in one operation. Records success/error statistics.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    /// * `account_data` - Raw account data from Solana
    ///
    /// # Returns
    ///
    /// * `Ok((u64, u64))` - Tuple of (base_reserve, quote_reserve)
    /// * `Err(AppError)` - If pool not registered or decoding failed
    pub fn decode_pool_data(
        &self,
        pool_address: &Pubkey,
        account_data: &[u8],
    ) -> Result<(u64, u64), AppError> {
        let decoders = self.decoders.read()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire read lock: {}", e)))?;
        
        let decoder = decoders.get(pool_address)
            .ok_or_else(|| AppError::ConfigError(format!(
                "No decoder registered for pool: {}",
                pool_address
            )))?;
        
        // Try to decode
        let result = decoder.decode_reserves(account_data);
        
        // Record statistics
        drop(decoders); // Release read lock before acquiring write lock
        let mut stats = self.stats.write()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire write lock: {}", e)))?;
        
        let pool_stats = stats.entry(*pool_address).or_insert_with(DecoderStats::new);
        
        match &result {
            Ok(_) => pool_stats.record_success(),
            Err(_) => pool_stats.record_error(),
        }
        
        result
    }

    /// Get the number of registered pools.
    pub fn pool_count(&self) -> usize {
        self.decoders.read()
            .map(|decoders| decoders.len())
            .unwrap_or(0)
    }

    /// Get or create a decoder for a specific pool (lazy initialization).
    ///
    /// This method implements lazy initialization - the decoder is created only
    /// when first requested, not at registration time. This is more efficient
    /// when dealing with many pools that may not all be actively used.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If decoder exists or was created
    /// * `Err(AppError)` - If pool type not registered or decoder creation failed
    ///
    /// # Thread Safety
    ///
    /// Uses double-check locking pattern for optimal performance:
    /// 1. First check with read lock (fast path, most common case)
    /// 2. If not found, acquire write lock and check again
    /// 3. Create decoder only if still not present
    pub fn get_or_create_decoder(&self, pool_address: &Pubkey) -> Result<(), AppError> {
        // Fast path: check if decoder already exists (read lock)
        {
            let decoders = self.decoders.read()
                .map_err(|e| AppError::ConfigError(format!("Failed to acquire read lock: {}", e)))?;
            
            if decoders.contains_key(pool_address) {
                return Ok(());
            }
        }

        // Slow path: create decoder (write lock)
        let mut decoders = self.decoders.write()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire write lock: {}", e)))?;
        
        // Double-check: another thread might have created it while we waited
        if decoders.contains_key(pool_address) {
            return Ok(());
        }

        // Get pool type
        let pool_types = self.pool_types.read()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire read lock: {}", e)))?;
        
        let dex_type = pool_types.get(pool_address)
            .ok_or_else(|| AppError::ConfigError(format!(
                "Pool type not registered for: {}. Call register_pool_type first.",
                pool_address
            )))?;

        // Create and insert decoder
        let decoder = create_decoder(*dex_type);
        decoders.insert(*pool_address, decoder);

        Ok(())
    }

    /// Register a pool type without creating the decoder.
    ///
    /// This method only stores the pool type, allowing for lazy decoder creation
    /// via `get_or_create_decoder` or `decode_pool_data_lazy`.
    ///
    /// # Arguments
    ///
    /// * `pool_config` - Configuration containing pool address and DEX type
    pub fn register_pool_type(&self, pool_config: PoolConfig) -> Result<(), AppError> {
        let pool_address = *pool_config.pool_address();
        let dex_type = pool_config.dex_type();
        
        let mut pool_types = self.pool_types.write()
            .map_err(|e| AppError::ConfigError(format!("Failed to acquire write lock: {}", e)))?;
        
        pool_types.insert(pool_address, dex_type);
        
        Ok(())
    }

    /// Decode pool data with lazy decoder initialization.
    ///
    /// This method will create the decoder on first use if it doesn't exist yet.
    /// The pool type must be registered first via `register_pool_type`.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    /// * `account_data` - Raw account data from Solana
    ///
    /// # Returns
    ///
    /// * `Ok((u64, u64))` - Tuple of (base_reserve, quote_reserve)
    /// * `Err(AppError)` - If pool type not registered or decoding failed
    pub fn decode_pool_data_lazy(
        &self,
        pool_address: &Pubkey,
        account_data: &[u8],
    ) -> Result<(u64, u64), AppError> {
        // Ensure decoder exists (creates if needed)
        self.get_or_create_decoder(pool_address)?;
        
        // Now decode using the existing method
        self.decode_pool_data(pool_address, account_data)
    }

    /// Get statistics for a specific pool.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    ///
    /// # Returns
    ///
    /// * `Some(DecoderStats)` - Statistics if pool has been used
    /// * `None` - If pool has no statistics yet
    pub fn get_stats(&self, pool_address: &Pubkey) -> Option<DecoderStats> {
        self.stats.read()
            .ok()
            .and_then(|stats| stats.get(pool_address).cloned())
    }

    /// Get statistics for all pools.
    ///
    /// # Returns
    ///
    /// HashMap mapping pool addresses to their statistics
    pub fn get_all_stats(&self) -> HashMap<Pubkey, DecoderStats> {
        self.stats.read()
            .map(|stats| stats.clone())
            .unwrap_or_default()
    }

    /// Log statistics for all pools.
    ///
    /// Prints statistics to stdout. In production, this would integrate
    /// with a logging framework.
    ///
    /// # Arguments
    ///
    /// * `force` - If true, logs regardless of time since last log
    ///
    /// # Returns
    ///
    /// True if statistics were logged, false if skipped
    pub fn log_stats(&self, force: bool) -> bool {
        const LOG_INTERVAL_SECONDS: u64 = 300; // 5 minutes

        let now = current_timestamp();
        
        // Check if we should log
        if !force {
            let last_log = self.last_stats_log.read()
                .map(|t| *t)
                .unwrap_or(0);
            
            if now - last_log < LOG_INTERVAL_SECONDS {
                return false;
            }
        }

        // Update last log time
        if let Ok(mut last_log) = self.last_stats_log.write() {
            *last_log = now;
        }

        // Print statistics
        println!("=== Decoder Statistics ===");
        println!("Timestamp: {}", now);
        
        let all_stats = self.get_all_stats();
        if all_stats.is_empty() {
            println!("No statistics available yet.");
            return true;
        }

        for (pool_addr, stats) in all_stats.iter() {
            println!("\nPool: {}", pool_addr);
            println!("  Total attempts: {}", stats.total_attempts());
            println!("  Success count: {}", stats.success_count);
            println!("  Error count: {}", stats.error_count);
            println!("  Success rate: {:.2}%", stats.success_rate());
            
            if let Some(first) = stats.first_success_time {
                println!("  First success: {}", first);
            }
            if let Some(last) = stats.last_success_time {
                println!("  Last success: {}", last);
            }
            if let Some(last_err) = stats.last_error_time {
                println!("  Last error: {}", last_err);
            }
        }
        
        println!("========================\n");
        true
    }

    /// Reset statistics for all pools.
    ///
    /// Useful for testing or periodic stats resets.
    pub fn reset_stats(&self) {
        if let Ok(mut stats) = self.stats.write() {
            stats.clear();
        }
    }

    /// Reset statistics for a specific pool.
    ///
    /// # Arguments
    ///
    /// * `pool_address` - The address of the pool
    pub fn reset_pool_stats(&self, pool_address: &Pubkey) {
        if let Ok(mut stats) = self.stats.write() {
            stats.remove(pool_address);
        }
    }
}

impl Default for DecoderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DexType;

    // Test that the trait is object-safe (can be used as Box<dyn DexDecoder>)
    #[test]
    fn test_trait_is_object_safe() {
        fn _assert_object_safe(_decoder: &dyn DexDecoder) {}
    }

    #[test]
    fn test_create_decoder_pumpfun() {
        let decoder = create_decoder(DexType::PumpFun);
        
        // Create valid test data for PumpFun (256 bytes)
        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());
        
        let result = decoder.decode_reserves(&data);
        assert!(result.is_ok());
        let (token, sol) = result.unwrap();
        assert_eq!(token, 1000);
        assert_eq!(sol, 2000);
    }

    #[test]
    fn test_create_decoder_raydium() {
        let decoder = create_decoder(DexType::Raydium);
        
        // Raydium decoder validation should work
        let data = vec![0u8; 100]; // Invalid size
        let result = decoder.validate_account(&data);
        assert!(result.is_err()); // Should fail validation
    }

    #[test]
    fn test_create_decoder_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Box<dyn DexDecoder>>();
    }

    // Test DecoderRegistry
    #[test]
    fn test_decoder_registry_new() {
        let registry = DecoderRegistry::new();
        assert_eq!(registry.pool_count(), 0);
    }

    #[test]
    fn test_decoder_registry_register_pool() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();

        let result = registry.register_pool(pool_config);
        assert!(result.is_ok());
        assert_eq!(registry.pool_count(), 1);
    }

    #[test]
    fn test_decoder_registry_get_decoder() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        // Should fail for non-registered pool
        let result = registry.get_decoder(&pool_addr);
        assert!(result.is_err());

        // Register pool
        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        // Should succeed now
        let result = registry.get_decoder(&pool_addr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_decoder_registry_decode_pool_data() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        // Register a PumpFun pool
        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        // Create valid test data for PumpFun (256 bytes)
        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&5000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&10000u64.to_le_bytes());

        // Decode using registry
        let result = registry.decode_pool_data(&pool_addr, &data);
        assert!(result.is_ok());
        let (token, sol) = result.unwrap();
        assert_eq!(token, 5000);
        assert_eq!(sol, 10000);
    }

    #[test]
    fn test_decoder_registry_decode_unregistered_pool() {
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let data = vec![0u8; 256];

        // Should fail for unregistered pool
        let result = registry.decode_pool_data(&pool_addr, &data);
        assert!(result.is_err());
    }

    #[test]
    fn test_decoder_registry_multiple_pools() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();

        // Register multiple pools
        for _ in 0..5 {
            let pool_addr = Pubkey::new_unique();
            let token_mint = Pubkey::new_unique();
            let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
            registry.register_pool(pool_config).unwrap();
        }

        assert_eq!(registry.pool_count(), 5);
    }

    #[test]
    fn test_decoder_registry_default() {
        let registry = DecoderRegistry::default();
        assert_eq!(registry.pool_count(), 0);
    }

    // Test lazy initialization
    #[test]
    fn test_decoder_registry_lazy_initialization() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        // Register only the pool type (no decoder created yet)
        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool_type(pool_config).unwrap();

        // Pool count should be 0 since no decoder created yet
        assert_eq!(registry.pool_count(), 0);

        // Trigger lazy creation
        let result = registry.get_or_create_decoder(&pool_addr);
        assert!(result.is_ok());

        // Now decoder should exist
        assert_eq!(registry.pool_count(), 1);
    }

    #[test]
    fn test_decoder_registry_get_or_create_idempotent() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool_type(pool_config).unwrap();

        // Call multiple times - should be idempotent
        registry.get_or_create_decoder(&pool_addr).unwrap();
        registry.get_or_create_decoder(&pool_addr).unwrap();
        registry.get_or_create_decoder(&pool_addr).unwrap();

        assert_eq!(registry.pool_count(), 1);
    }

    #[test]
    fn test_decoder_registry_lazy_decode() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        // Register only pool type
        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool_type(pool_config).unwrap();

        // Create valid test data
        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&3000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&6000u64.to_le_bytes());

        // Decode with lazy initialization
        let result = registry.decode_pool_data_lazy(&pool_addr, &data);
        assert!(result.is_ok());
        let (token, sol) = result.unwrap();
        assert_eq!(token, 3000);
        assert_eq!(sol, 6000);

        // Decoder should now exist
        assert_eq!(registry.pool_count(), 1);
    }

    #[test]
    fn test_decoder_registry_lazy_without_registration() {
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();

        // Try to get or create without registering pool type
        let result = registry.get_or_create_decoder(&pool_addr);
        assert!(result.is_err());
    }

    // Test statistics
    #[test]
    fn test_decoder_stats_new() {
        let stats = DecoderStats::new();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.error_count, 0);
        assert_eq!(stats.total_attempts(), 0);
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_decoder_stats_record_success() {
        let mut stats = DecoderStats::new();
        
        stats.record_success();
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.error_count, 0);
        assert!(stats.first_success_time.is_some());
        assert!(stats.last_success_time.is_some());
        assert_eq!(stats.success_rate(), 100.0);
    }

    #[test]
    fn test_decoder_stats_record_error() {
        let mut stats = DecoderStats::new();
        
        stats.record_error();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.error_count, 1);
        assert!(stats.last_error_time.is_some());
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_decoder_stats_success_rate() {
        let mut stats = DecoderStats::new();
        
        stats.record_success();
        stats.record_success();
        stats.record_success();
        stats.record_error();
        
        assert_eq!(stats.total_attempts(), 4);
        assert_eq!(stats.success_rate(), 75.0);
    }

    #[test]
    fn test_decoder_registry_tracks_stats() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        // Create valid test data
        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());

        // Successful decode
        registry.decode_pool_data(&pool_addr, &data).unwrap();

        // Check stats
        let stats = registry.get_stats(&pool_addr).unwrap();
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.error_count, 0);
    }

    #[test]
    fn test_decoder_registry_tracks_errors() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        // Invalid data (too small)
        let data = vec![0u8; 50];

        // Should fail
        let _ = registry.decode_pool_data(&pool_addr, &data);

        // Check stats
        let stats = registry.get_stats(&pool_addr).unwrap();
        assert_eq!(stats.success_count, 0);
        assert_eq!(stats.error_count, 1);
    }

    #[test]
    fn test_decoder_registry_get_all_stats() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();

        // Register multiple pools and use them
        for _ in 0..3 {
            let pool_addr = Pubkey::new_unique();
            let token_mint = Pubkey::new_unique();
            let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
            registry.register_pool(pool_config).unwrap();

            let mut data = vec![0u8; 256];
            data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
            data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());
            registry.decode_pool_data(&pool_addr, &data).unwrap();
        }

        let all_stats = registry.get_all_stats();
        assert_eq!(all_stats.len(), 3);
    }

    #[test]
    fn test_decoder_registry_reset_stats() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());
        registry.decode_pool_data(&pool_addr, &data).unwrap();

        // Stats should exist
        assert!(registry.get_stats(&pool_addr).is_some());

        // Reset all stats
        registry.reset_stats();

        // Stats should be gone
        assert!(registry.get_stats(&pool_addr).is_none());
    }

    #[test]
    fn test_decoder_registry_reset_pool_stats() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr1 = Pubkey::new_unique();
        let pool_addr2 = Pubkey::new_unique();

        for pool_addr in [&pool_addr1, &pool_addr2] {
            let token_mint = Pubkey::new_unique();
            let pool_config = PoolConfig::new(*pool_addr, DexType::PumpFun, token_mint).unwrap();
            registry.register_pool(pool_config).unwrap();

            let mut data = vec![0u8; 256];
            data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
            data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());
            registry.decode_pool_data(pool_addr, &data).unwrap();
        }

        // Both should have stats
        assert!(registry.get_stats(&pool_addr1).is_some());
        assert!(registry.get_stats(&pool_addr2).is_some());

        // Reset only pool1
        registry.reset_pool_stats(&pool_addr1);

        // Pool1 stats should be gone, pool2 should remain
        assert!(registry.get_stats(&pool_addr1).is_none());
        assert!(registry.get_stats(&pool_addr2).is_some());
    }

    #[test]
    fn test_decoder_registry_log_stats() {
        use crate::config::PoolConfig;
        use solana_sdk::pubkey::Pubkey;

        let registry = DecoderRegistry::new();
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();

        let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, token_mint).unwrap();
        registry.register_pool(pool_config).unwrap();

        let mut data = vec![0u8; 256];
        data[0x18..0x20].copy_from_slice(&1000u64.to_le_bytes());
        data[0x20..0x28].copy_from_slice(&2000u64.to_le_bytes());
        registry.decode_pool_data(&pool_addr, &data).unwrap();

        // Force log should work
        let logged = registry.log_stats(true);
        assert!(logged);
    }
}
