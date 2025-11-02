/// Raydium Pool Address Resolver
///
/// This module provides functionality to resolve pool addresses by fetching
/// metadata from the Raydium API and mapping various identifiers (marketId,
/// baseMint, quoteMint) to the canonical AMM pool address (ammId).
///
/// # Features
///
/// - Fetches pool data from https://api.raydium.io/v2/sdk/liquidity/mainnet.json
/// - Maps marketId, baseMint/quoteMint pairs to ammId
/// - Automatic retry logic with configurable timeout
/// - Caching of API responses to reduce network calls
///
/// # Example
///
/// ```no_run
/// use datanalyzer::raydium_resolver::RaydiumResolver;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let resolver = RaydiumResolver::new();
/// resolver.fetch_pool_data().await?;
///
/// // Resolve by marketId
/// if let Some(amm_id) = resolver.resolve_by_market_id("market_address").await? {
///     println!("AMM ID: {}", amm_id);
/// }
///
/// // Resolve by base/quote mint pair
/// if let Some(amm_id) = resolver.resolve_by_mints("base_mint", "quote_mint").await? {
///     println!("AMM ID: {}", amm_id);
/// }
/// # Ok(())
/// # }
/// ```
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Default Raydium API endpoint for pool data
const RAYDIUM_API_URL: &str = "https://api.raydium.io/v2/sdk/liquidity/mainnet.json";

/// Default timeout for API requests
const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Maximum retry attempts for API requests
const MAX_RETRIES: u32 = 3;

/// Pool data from Raydium API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaydiumPool {
    /// AMM pool ID (the canonical address)
    pub id: String,
    /// Base token mint
    pub base_mint: String,
    /// Quote token mint
    pub quote_mint: String,
    /// LP token mint
    pub lp_mint: String,
    /// Market ID (OpenBook/Serum market)
    #[serde(default)]
    pub market_id: String,
    /// Pool version
    #[serde(default)]
    pub version: u8,
    /// Program ID (should be Raydium AMM v4 for our purposes)
    #[serde(default)]
    pub program_id: String,
}

/// Response structure from Raydium API
#[derive(Debug, Deserialize)]
pub struct RaydiumApiResponse {
    /// Official pools
    #[serde(default)]
    pub official: Vec<RaydiumPool>,
    /// Unverified pools (we'll skip these for safety)
    #[serde(default)]
    pub unverified: Vec<RaydiumPool>,
}

/// Raydium pool address resolver
///
/// Provides methods to resolve pool addresses using various identifiers
pub struct RaydiumResolver {
    /// Cached pool data
    pools: Arc<RwLock<Vec<RaydiumPool>>>,
    /// Index: marketId -> ammId
    market_index: Arc<RwLock<HashMap<String, String>>>,
    /// Index: (baseMint, quoteMint) -> ammId
    mint_pair_index: Arc<RwLock<HashMap<(String, String), String>>>,
    /// Index: lpMint -> ammId
    lp_mint_index: Arc<RwLock<HashMap<String, String>>>,
    /// HTTP client for API requests
    client: reqwest::Client,
    /// API URL
    api_url: String,
}

impl RaydiumResolver {
    /// Create a new RaydiumResolver with default configuration
    pub fn new() -> Self {
        Self::with_config(RAYDIUM_API_URL.to_string(), DEFAULT_TIMEOUT_SECS)
    }

    /// Create a new RaydiumResolver with custom configuration
    ///
    /// # Arguments
    ///
    /// * `api_url` - URL of the Raydium API endpoint
    /// * `timeout_secs` - Request timeout in seconds
    pub fn with_config(api_url: String, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            pools: Arc::new(RwLock::new(Vec::new())),
            market_index: Arc::new(RwLock::new(HashMap::new())),
            mint_pair_index: Arc::new(RwLock::new(HashMap::new())),
            lp_mint_index: Arc::new(RwLock::new(HashMap::new())),
            client,
            api_url,
        }
    }

    /// Fetch pool data from Raydium API with retry logic
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If data was successfully fetched and indexed
    /// * `Err(AppError)` - If all retry attempts failed
    pub async fn fetch_pool_data(&self) -> Result<(), AppError> {
        let mut last_error = None;

        for attempt in 1..=MAX_RETRIES {
            match self.fetch_pool_data_once().await {
                Ok(()) => {
                    log::info!("Successfully fetched Raydium pool data on attempt {}", attempt);
                    return Ok(());
                }
                Err(e) => {
                    log::warn!(
                        "Failed to fetch Raydium pool data (attempt {}/ {}): {}",
                        attempt,
                        MAX_RETRIES,
                        e
                    );
                    last_error = Some(e);

                    if attempt < MAX_RETRIES {
                        // Exponential backoff
                        let delay = Duration::from_millis(500 * 2u64.pow(attempt - 1));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AppError::RpcError("Failed to fetch Raydium pool data after retries".to_string())
        }))
    }

    /// Fetch pool data once (single attempt)
    async fn fetch_pool_data_once(&self) -> Result<(), AppError> {
        log::debug!("Fetching Raydium pool data from {}", self.api_url);

        let response = self
            .client
            .get(&self.api_url)
            .send()
            .await
            .map_err(|e| AppError::RpcError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::RpcError(format!(
                "HTTP {} from Raydium API",
                response.status()
            )));
        }

        let api_data: RaydiumApiResponse = response
            .json()
            .await
            .map_err(|e| AppError::DecodingError(format!("Failed to parse JSON: {}", e)))?;

        // Only use official pools for safety
        let official_pools = api_data.official;
        log::info!("Fetched {} official Raydium pools", official_pools.len());

        // Build indices
        let mut market_idx = HashMap::new();
        let mut mint_pair_idx = HashMap::new();
        let mut lp_mint_idx = HashMap::new();

        for pool in &official_pools {
            let amm_id = pool.id.clone();

            // Index by marketId if present
            if !pool.market_id.is_empty() {
                market_idx.insert(pool.market_id.clone(), amm_id.clone());
            }

            // Index by (baseMint, quoteMint) pair
            mint_pair_idx.insert(
                (pool.base_mint.clone(), pool.quote_mint.clone()),
                amm_id.clone(),
            );

            // Index by lpMint
            if !pool.lp_mint.is_empty() {
                lp_mint_idx.insert(pool.lp_mint.clone(), amm_id);
            }
        }

        // Update cached data
        *self.pools.write().await = official_pools;
        *self.market_index.write().await = market_idx;
        *self.mint_pair_index.write().await = mint_pair_idx;
        *self.lp_mint_index.write().await = lp_mint_idx;

        Ok(())
    }

    /// Resolve a pool address by any known identifier
    ///
    /// Tries to resolve in this order:
    /// 1. Exact ammId match
    /// 2. marketId match
    /// 3. lpMint match
    /// 4. (baseMint, quoteMint) pair match
    ///
    /// # Arguments
    ///
    /// * `address` - The address to resolve (could be ammId, marketId, or lpMint)
    ///
    /// # Returns
    ///
    /// * `Ok(Some(String))` - The resolved ammId
    /// * `Ok(None)` - No match found
    /// * `Err(AppError)` - If resolution fails
    pub async fn resolve(&self, address: &str) -> Result<Option<String>, AppError> {
        // Check if it's already an ammId (exact match in pools)
        let pools = self.pools.read().await;
        if pools.iter().any(|p| p.id == address) {
            log::debug!("Address {} is already an ammId", address);
            return Ok(Some(address.to_string()));
        }
        drop(pools);

        // Try marketId
        let market_idx = self.market_index.read().await;
        if let Some(amm_id) = market_idx.get(address) {
            log::info!("Resolved marketId {} -> ammId {}", address, amm_id);
            return Ok(Some(amm_id.clone()));
        }
        drop(market_idx);

        // Try lpMint
        let lp_idx = self.lp_mint_index.read().await;
        if let Some(amm_id) = lp_idx.get(address) {
            log::info!("Resolved lpMint {} -> ammId {}", address, amm_id);
            return Ok(Some(amm_id.clone()));
        }
        drop(lp_idx);

        Ok(None)
    }

    /// Resolve by base and quote mint pair
    ///
    /// # Arguments
    ///
    /// * `base_mint` - Base token mint address
    /// * `quote_mint` - Quote token mint address
    ///
    /// # Returns
    ///
    /// * `Ok(Some(String))` - The resolved ammId
    /// * `Ok(None)` - No match found
    /// * `Err(AppError)` - If resolution fails
    pub async fn resolve_by_mints(
        &self,
        base_mint: &str,
        quote_mint: &str,
    ) -> Result<Option<String>, AppError> {
        let mint_idx = self.mint_pair_index.read().await;
        let key = (base_mint.to_string(), quote_mint.to_string());

        if let Some(amm_id) = mint_idx.get(&key) {
            log::info!(
                "Resolved mint pair ({}, {}) -> ammId {}",
                base_mint,
                quote_mint,
                amm_id
            );
            return Ok(Some(amm_id.clone()));
        }

        Ok(None)
    }

    /// Resolve by marketId
    ///
    /// # Arguments
    ///
    /// * `market_id` - OpenBook/Serum market ID
    ///
    /// # Returns
    ///
    /// * `Ok(Some(String))` - The resolved ammId
    /// * `Ok(None)` - No match found
    /// * `Err(AppError)` - If resolution fails
    pub async fn resolve_by_market_id(&self, market_id: &str) -> Result<Option<String>, AppError> {
        let market_idx = self.market_index.read().await;

        if let Some(amm_id) = market_idx.get(market_id) {
            log::info!("Resolved marketId {} -> ammId {}", market_id, amm_id);
            return Ok(Some(amm_id.clone()));
        }

        Ok(None)
    }

    /// Get the number of pools in the cache
    pub async fn pool_count(&self) -> usize {
        self.pools.read().await.len()
    }
}

impl Default for RaydiumResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default as it requires network access
    async fn test_fetch_pool_data() {
        let resolver = RaydiumResolver::new();
        let result = resolver.fetch_pool_data().await;

        assert!(result.is_ok(), "Failed to fetch pool data: {:?}", result);

        let pool_count = resolver.pool_count().await;
        assert!(pool_count > 0, "Expected non-zero pool count");
    }

    #[tokio::test]
    #[ignore] // Ignore by default as it requires network access
    async fn test_resolve_sol_usdc_pool() {
        let resolver = RaydiumResolver::new();
        resolver.fetch_pool_data().await.unwrap();

        // Test with known SOL/USDC pool
        let known_pool = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
        let result = resolver.resolve(known_pool).await.unwrap();

        assert_eq!(result, Some(known_pool.to_string()));
    }

    #[tokio::test]
    #[ignore] // Ignore by default as it requires network access
    async fn test_resolve_by_mints() {
        let resolver = RaydiumResolver::new();
        resolver.fetch_pool_data().await.unwrap();

        // SOL/USDC mints
        let sol_mint = "So11111111111111111111111111111111111111112";
        let usdc_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

        let result = resolver.resolve_by_mints(sol_mint, usdc_mint).await.unwrap();

        assert!(result.is_some(), "Should resolve SOL/USDC pool");
    }
}
