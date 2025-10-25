/// Token metadata provider module for fetching token decimals and other metadata via RPC.
///
/// This module provides:
/// - RPC-based token metadata fetching (decimals, supply, etc.)
/// - TTL-based caching for metadata
/// - Fallback mechanisms for missing metadata
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cached token metadata
#[derive(Debug, Clone)]
pub struct CachedMetadata {
    pub decimals: u8,
    pub supply: Option<u64>,
    timestamp: Instant,
}

impl CachedMetadata {
    /// Create new cached metadata
    pub fn new(decimals: u8, supply: Option<u64>) -> Self {
        Self {
            decimals,
            supply,
            timestamp: Instant::now(),
        }
    }
    
    /// Check if the cached metadata is expired
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.timestamp.elapsed() > ttl
    }
    
    /// Get the age of the cached metadata
    pub fn age(&self) -> Duration {
        self.timestamp.elapsed()
    }
}

/// Token metadata structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetadata {
    pub mint: String,
    pub decimals: u8,
    pub supply: Option<u64>,
}

/// Token metadata provider using Solana RPC
pub struct TokenMetadataProvider {
    rpc_client: Arc<RpcClient>,
    cache: Arc<RwLock<HashMap<String, CachedMetadata>>>,
    cache_ttl: Duration,
}

impl TokenMetadataProvider {
    /// Create a new token metadata provider
    pub fn new(rpc_url: String, cache_ttl: Duration) -> Self {
        let rpc_client = Arc::new(RpcClient::new(rpc_url));
        
        Self {
            rpc_client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
        }
    }
    
    /// Create a new provider with an existing RPC client
    pub fn with_client(rpc_client: Arc<RpcClient>, cache_ttl: Duration) -> Self {
        Self {
            rpc_client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
        }
    }
    
    /// Get token decimals for a mint address
    pub async fn get_decimals(&self, mint: &str) -> Result<u8, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(mint) {
                if !cached.is_expired(self.cache_ttl) {
                    log::debug!("Metadata cache hit for mint: {}", mint);
                    return Ok(cached.decimals);
                }
            }
        }
        
        // Fetch from RPC
        let metadata = self.fetch_metadata_from_rpc(mint).await?;
        
        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(mint.to_string(), metadata.clone());
        }
        
        Ok(metadata.decimals)
    }
    
    /// Get full token metadata
    pub async fn get_metadata(&self, mint: &str) -> Result<TokenMetadata, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(mint) {
                if !cached.is_expired(self.cache_ttl) {
                    log::debug!("Metadata cache hit for mint: {}", mint);
                    return Ok(TokenMetadata {
                        mint: mint.to_string(),
                        decimals: cached.decimals,
                        supply: cached.supply,
                    });
                }
            }
        }
        
        // Fetch from RPC
        let metadata = self.fetch_metadata_from_rpc(mint).await?;
        
        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(mint.to_string(), metadata.clone());
        }
        
        Ok(TokenMetadata {
            mint: mint.to_string(),
            decimals: metadata.decimals,
            supply: metadata.supply,
        })
    }
    
    /// Fetch metadata from RPC
    async fn fetch_metadata_from_rpc(&self, mint: &str) -> Result<CachedMetadata, AppError> {
        let pubkey = Pubkey::from_str(mint)
            .map_err(|e| AppError::ConfigError(format!("Invalid mint address: {}", e)))?;
        
        // Spawn blocking task for RPC call
        let rpc_client = Arc::clone(&self.rpc_client);
        let metadata = tokio::task::spawn_blocking(move || {
            // Get token account data
            let account_data = rpc_client.get_account_data(&pubkey)
                .map_err(|e| AppError::RpcError(format!("Failed to get account data: {}", e)))?;
            
            // SPL Token Mint layout: first byte is option (0 or 1), then decimals at offset 44
            if account_data.len() < 82 {
                return Err(AppError::DecodingError(format!(
                    "Invalid mint account data length: {}",
                    account_data.len()
                )));
            }
            
            // Decimals is at byte 44
            let decimals = account_data[44];
            
            // Supply is a u64 at bytes 36-43 (little-endian)
            let supply_bytes: [u8; 8] = account_data[36..44]
                .try_into()
                .map_err(|_| AppError::DecodingError("Failed to read supply bytes".to_string()))?;
            let supply = u64::from_le_bytes(supply_bytes);
            
            Ok::<CachedMetadata, AppError>(CachedMetadata::new(decimals, Some(supply)))
        })
        .await
        .map_err(|e| AppError::RpcError(format!("RPC task failed: {}", e)))??;
        
        Ok(metadata)
    }
    
    /// Clear the cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
    
    /// Get the number of cached entries
    pub async fn cache_size(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }
    
    /// Prefetch metadata for multiple mints
    pub async fn prefetch_metadata(&self, mints: &[String]) -> Result<(), AppError> {
        for mint in mints {
            if let Err(e) = self.get_metadata(mint).await {
                log::warn!("Failed to prefetch metadata for {}: {}", mint, e);
                // Continue with other mints even if one fails
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cached_metadata_new() {
        let metadata = CachedMetadata::new(9, Some(1000000));
        assert_eq!(metadata.decimals, 9);
        assert_eq!(metadata.supply, Some(1000000));
        assert!(metadata.age() < Duration::from_secs(1));
    }

    #[test]
    fn test_cached_metadata_not_expired() {
        let metadata = CachedMetadata::new(6, None);
        assert!(!metadata.is_expired(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn test_cached_metadata_expired() {
        let metadata = CachedMetadata::new(9, Some(1000));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(metadata.is_expired(Duration::from_millis(50)));
    }

    #[test]
    fn test_cached_metadata_age() {
        let metadata = CachedMetadata::new(9, Some(1000));
        std::thread::sleep(Duration::from_millis(50));
        let age = metadata.age();
        assert!(age >= Duration::from_millis(50));
        assert!(age < Duration::from_millis(200));
    }

    #[test]
    fn test_token_metadata_provider_new() {
        let provider = TokenMetadataProvider::new(
            "https://api.mainnet-beta.solana.com".to_string(),
            Duration::from_secs(300),
        );
        assert_eq!(provider.cache_ttl, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn test_token_metadata_provider_cache_size() {
        let provider = TokenMetadataProvider::new(
            "https://api.mainnet-beta.solana.com".to_string(),
            Duration::from_secs(300),
        );
        assert_eq!(provider.cache_size().await, 0);
        
        // Add entry to cache
        {
            let mut cache = provider.cache.write().await;
            cache.insert(
                "So11111111111111111111111111111111111111112".to_string(),
                CachedMetadata::new(9, Some(1000000)),
            );
        }
        
        assert_eq!(provider.cache_size().await, 1);
    }

    #[tokio::test]
    async fn test_token_metadata_provider_clear_cache() {
        let provider = TokenMetadataProvider::new(
            "https://api.mainnet-beta.solana.com".to_string(),
            Duration::from_secs(300),
        );
        
        // Add entry to cache
        {
            let mut cache = provider.cache.write().await;
            cache.insert(
                "So11111111111111111111111111111111111111112".to_string(),
                CachedMetadata::new(9, Some(1000000)),
            );
        }
        
        assert_eq!(provider.cache_size().await, 1);
        
        provider.clear_cache().await;
        assert_eq!(provider.cache_size().await, 0);
    }

    #[test]
    fn test_token_metadata_serialization() {
        let metadata = TokenMetadata {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            decimals: 9,
            supply: Some(1000000),
        };
        
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("So11111111111111111111111111111111111111112"));
        assert!(json.contains("\"decimals\":9"));
    }
}
