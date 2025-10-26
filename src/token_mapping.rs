/// Token mapping module for mapping Solana mint addresses to CoinGecko token IDs.
///
/// This module provides functionality to:
/// - Map Solana mint addresses to CoinGecko token identifiers
/// - Support static configuration from TOML
/// - Support dynamic provider implementations for future extensibility
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for a single token mapping
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenMappingEntry {
    /// Solana mint address (as string)
    pub mint: String,
    /// CoinGecko token ID
    pub coingecko_id: String,
    /// Optional: Custom cache TTL in seconds for this specific token
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_ttl_secs: Option<u64>,
}

/// Trait for token mapping providers
#[async_trait::async_trait]
pub trait TokenMappingProvider: Send + Sync {
    /// Get the CoinGecko token ID for a given mint address
    async fn get_token_id(&self, mint: &str) -> Result<Option<String>, AppError>;

    /// Get the custom cache TTL for a token, if configured
    async fn get_cache_ttl(&self, mint: &str) -> Option<u64>;
}

/// Static token mapping provider that uses a pre-configured HashMap
pub struct StaticTokenMapping {
    mappings: HashMap<String, TokenMappingEntry>,
}

impl StaticTokenMapping {
    /// Create a new static token mapping from a list of entries
    pub fn new(entries: Vec<TokenMappingEntry>) -> Result<Self, AppError> {
        let mut mappings = HashMap::new();

        for entry in entries {
            // Validate mint address format (basic check - should be base58)
            if entry.mint.is_empty() {
                return Err(AppError::ConfigError(
                    "Token mint cannot be empty".to_string(),
                ));
            }

            // Validate CoinGecko ID format
            if entry.coingecko_id.is_empty() {
                return Err(AppError::ConfigError(format!(
                    "CoinGecko ID cannot be empty for mint: {}",
                    entry.mint
                )));
            }

            mappings.insert(entry.mint.clone(), entry);
        }

        Ok(Self { mappings })
    }

    /// Get the number of configured mappings
    pub fn len(&self) -> usize {
        self.mappings.len()
    }

    /// Check if the mapping is empty
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
}

#[async_trait::async_trait]
impl TokenMappingProvider for StaticTokenMapping {
    async fn get_token_id(&self, mint: &str) -> Result<Option<String>, AppError> {
        Ok(self
            .mappings
            .get(mint)
            .map(|entry| entry.coingecko_id.clone()))
    }

    async fn get_cache_ttl(&self, mint: &str) -> Option<u64> {
        self.mappings
            .get(mint)
            .and_then(|entry| entry.cache_ttl_secs)
    }
}

/// Token mapping service that supports multiple providers
pub struct TokenMappingService {
    providers: Vec<Arc<dyn TokenMappingProvider>>,
    /// Cache for resolved token IDs to avoid repeated lookups
    cache: Arc<RwLock<HashMap<String, Option<String>>>>,
}

impl TokenMappingService {
    /// Create a new token mapping service with the given providers
    pub fn new(providers: Vec<Arc<dyn TokenMappingProvider>>) -> Self {
        Self {
            providers,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a service with a single static provider
    pub fn with_static_mapping(entries: Vec<TokenMappingEntry>) -> Result<Self, AppError> {
        let provider = Arc::new(StaticTokenMapping::new(entries)?);
        Ok(Self::new(vec![provider]))
    }

    /// Get the CoinGecko token ID for a given mint address
    /// Checks cache first, then queries providers in order
    pub async fn get_token_id(&self, mint: &str) -> Result<Option<String>, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached_id) = cache.get(mint) {
                return Ok(cached_id.clone());
            }
        }

        // Query providers in order
        for provider in &self.providers {
            match provider.get_token_id(mint).await {
                Ok(Some(token_id)) => {
                    // Cache the result
                    let mut cache = self.cache.write().await;
                    cache.insert(mint.to_string(), Some(token_id.clone()));
                    return Ok(Some(token_id));
                }
                Ok(None) => continue, // Try next provider
                Err(e) => {
                    log::warn!("Provider failed for mint {}: {}", mint, e);
                    continue; // Try next provider
                }
            }
        }

        // No provider had a mapping, cache the negative result
        let mut cache = self.cache.write().await;
        cache.insert(mint.to_string(), None);
        Ok(None)
    }

    /// Get the custom cache TTL for a token from any provider
    pub async fn get_cache_ttl(&self, mint: &str) -> Option<u64> {
        for provider in &self.providers {
            if let Some(ttl) = provider.get_cache_ttl(mint).await {
                return Some(ttl);
            }
        }
        None
    }

    /// Clear the internal cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Get the number of cached entries
    pub async fn cache_size(&self) -> usize {
        let cache = self.cache.read().await;
        cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_mapping_entry_creation() {
        let entry = TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        };

        assert_eq!(entry.mint, "So11111111111111111111111111111111111111112");
        assert_eq!(entry.coingecko_id, "solana");
        assert_eq!(entry.cache_ttl_secs, None);
    }

    #[test]
    fn test_token_mapping_entry_with_ttl() {
        let entry = TokenMappingEntry {
            mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
            coingecko_id: "usd-coin".to_string(),
            cache_ttl_secs: Some(600),
        };

        assert_eq!(entry.cache_ttl_secs, Some(600));
    }

    #[test]
    fn test_static_token_mapping_new() {
        let entries = vec![
            TokenMappingEntry {
                mint: "So11111111111111111111111111111111111111112".to_string(),
                coingecko_id: "solana".to_string(),
                cache_ttl_secs: None,
            },
            TokenMappingEntry {
                mint: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(),
                coingecko_id: "usd-coin".to_string(),
                cache_ttl_secs: Some(600),
            },
        ];

        let mapping = StaticTokenMapping::new(entries).unwrap();
        assert_eq!(mapping.len(), 2);
        assert!(!mapping.is_empty());
    }

    #[test]
    fn test_static_token_mapping_empty_mint() {
        let entries = vec![TokenMappingEntry {
            mint: "".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let result = StaticTokenMapping::new(entries);
        assert!(result.is_err());
    }

    #[test]
    fn test_static_token_mapping_empty_coingecko_id() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "".to_string(),
            cache_ttl_secs: None,
        }];

        let result = StaticTokenMapping::new(entries);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_static_token_mapping_get_token_id() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let mapping = StaticTokenMapping::new(entries).unwrap();
        let result = mapping
            .get_token_id("So11111111111111111111111111111111111111112")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("solana".to_string()));
    }

    #[tokio::test]
    async fn test_static_token_mapping_get_token_id_not_found() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let mapping = StaticTokenMapping::new(entries).unwrap();
        let result = mapping.get_token_id("UnknownMint").await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[tokio::test]
    async fn test_static_token_mapping_get_cache_ttl() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: Some(300),
        }];

        let mapping = StaticTokenMapping::new(entries).unwrap();
        let ttl = mapping
            .get_cache_ttl("So11111111111111111111111111111111111111112")
            .await;

        assert_eq!(ttl, Some(300));
    }

    #[tokio::test]
    async fn test_static_token_mapping_get_cache_ttl_none() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let mapping = StaticTokenMapping::new(entries).unwrap();
        let ttl = mapping
            .get_cache_ttl("So11111111111111111111111111111111111111112")
            .await;

        assert_eq!(ttl, None);
    }

    #[tokio::test]
    async fn test_token_mapping_service_with_static() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let service = TokenMappingService::with_static_mapping(entries).unwrap();
        let result = service
            .get_token_id("So11111111111111111111111111111111111111112")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some("solana".to_string()));
    }

    #[tokio::test]
    async fn test_token_mapping_service_caching() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let service = TokenMappingService::with_static_mapping(entries).unwrap();

        // First call - populates cache
        let result1 = service
            .get_token_id("So11111111111111111111111111111111111111112")
            .await;
        assert_eq!(result1.unwrap(), Some("solana".to_string()));
        assert_eq!(service.cache_size().await, 1);

        // Second call - should use cache
        let result2 = service
            .get_token_id("So11111111111111111111111111111111111111112")
            .await;
        assert_eq!(result2.unwrap(), Some("solana".to_string()));
        assert_eq!(service.cache_size().await, 1);
    }

    #[tokio::test]
    async fn test_token_mapping_service_cache_negative_result() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let service = TokenMappingService::with_static_mapping(entries).unwrap();

        // Look up non-existent mint
        let result1 = service.get_token_id("UnknownMint").await;
        assert_eq!(result1.unwrap(), None);
        assert_eq!(service.cache_size().await, 1);

        // Second lookup should use cached negative result
        let result2 = service.get_token_id("UnknownMint").await;
        assert_eq!(result2.unwrap(), None);
        assert_eq!(service.cache_size().await, 1);
    }

    #[tokio::test]
    async fn test_token_mapping_service_clear_cache() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: None,
        }];

        let service = TokenMappingService::with_static_mapping(entries).unwrap();

        // Populate cache
        let _ = service
            .get_token_id("So11111111111111111111111111111111111111112")
            .await;
        assert_eq!(service.cache_size().await, 1);

        // Clear cache
        service.clear_cache().await;
        assert_eq!(service.cache_size().await, 0);
    }

    #[tokio::test]
    async fn test_token_mapping_service_get_cache_ttl() {
        let entries = vec![TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: Some(600),
        }];

        let service = TokenMappingService::with_static_mapping(entries).unwrap();
        let ttl = service
            .get_cache_ttl("So11111111111111111111111111111111111111112")
            .await;

        assert_eq!(ttl, Some(600));
    }
}
