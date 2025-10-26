/// Oracle module for fetching token prices in USD.
///
/// This module provides a pluggable architecture for price oracles.
/// Currently implements:
/// - JupiterQuoteOracle: Fetches prices via Jupiter's quote API
/// - Support for stable coin direct valuation (USDC, USDT = $1.0)
///
/// Future implementations can include:
/// - Pyth oracle for on-chain price feeds
/// - Other DEX aggregators
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cached price with timestamp
#[derive(Debug, Clone)]
pub struct CachedPrice {
    pub price: f64,
    timestamp: Instant,
}

impl CachedPrice {
    pub fn new(price: f64) -> Self {
        Self {
            price,
            timestamp: Instant::now(),
        }
    }

    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.timestamp.elapsed() > ttl
    }

    pub fn age(&self) -> Duration {
        self.timestamp.elapsed()
    }
}

/// Oracle trait for price providers
#[async_trait::async_trait]
pub trait Oracle: Send + Sync {
    /// Fetch USD price for a token mint
    async fn fetch_price_usd(&self, mint: &str) -> Result<f64, AppError>;

    /// Get the name of this oracle
    fn name(&self) -> &str;
}

/// Configuration for Oracle
#[derive(Debug, Clone)]
pub struct OracleConfig {
    /// List of stable coin mints that are valued at $1.0
    pub stable_mints: HashSet<String>,
    /// Jupiter API URL
    pub jupiter_url: String,
    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
}

impl Default for OracleConfig {
    fn default() -> Self {
        // Default stable coins
        let mut stable_mints = HashSet::new();
        stable_mints.insert("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()); // USDC
        stable_mints.insert("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string()); // USDT

        Self {
            stable_mints,
            jupiter_url: "https://price.jup.ag/v4".to_string(),
            cache_ttl_secs: 60,
        }
    }
}

/// Jupiter quote oracle implementation
pub struct JupiterQuoteOracle {
    client: reqwest::Client,
    api_url: String,
    cache: Arc<RwLock<HashMap<String, CachedPrice>>>,
    cache_ttl: Duration,
    stable_mints: HashSet<String>,
}

impl JupiterQuoteOracle {
    /// Create a new Jupiter oracle with configuration
    pub fn new(config: OracleConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_url: config.jupiter_url,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(config.cache_ttl_secs),
            stable_mints: config.stable_mints,
        }
    }

    /// Check if a mint is a stable coin
    fn is_stable_mint(&self, mint: &str) -> bool {
        self.stable_mints.contains(mint)
    }

    /// Fetch price from Jupiter API
    async fn fetch_from_api(&self, mint: &str) -> Result<f64, AppError> {
        let url = format!("{}/price?ids={}&vsToken=USDC", self.api_url, mint);

        log::debug!("Fetching price from Jupiter: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::PriceError(format!("Jupiter API request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::PriceError(format!(
                "Jupiter API returned error status: {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| AppError::PriceError(format!("Failed to read response: {}", e)))?;

        let parsed: JupiterPriceResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::PriceError(format!("Failed to parse Jupiter response: {}", e))
        })?;

        // Extract price from response
        parsed.data.get(mint).map(|data| data.price).ok_or_else(|| {
            AppError::PriceError(format!("Mint {} not found in Jupiter response", mint))
        })
    }
}

#[async_trait::async_trait]
impl Oracle for JupiterQuoteOracle {
    async fn fetch_price_usd(&self, mint: &str) -> Result<f64, AppError> {
        // Check if it's a stable mint
        if self.is_stable_mint(mint) {
            log::debug!("Mint {} is a stable coin, returning 1.0", mint);
            return Ok(1.0);
        }

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(mint) {
                if !cached.is_expired(self.cache_ttl) {
                    log::debug!("Cache hit for mint: {}", mint);
                    return Ok(cached.price);
                }
            }
        }

        // Fetch from API
        log::debug!("Cache miss for mint: {}, fetching from Jupiter", mint);
        let price = self.fetch_from_api(mint).await.unwrap_or_else(|e| {
            log::warn!("Failed to fetch price for {}: {}, returning 0.0", mint, e);
            0.0
        });

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(mint.to_string(), CachedPrice::new(price));
        }

        Ok(price)
    }

    fn name(&self) -> &str {
        "JupiterQuoteOracle"
    }
}

/// Jupiter price response structure
#[derive(Debug, Deserialize, Serialize)]
struct JupiterPriceResponse {
    data: HashMap<String, JupiterPriceData>,
}

#[derive(Debug, Deserialize, Serialize)]
struct JupiterPriceData {
    id: String,
    price: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_config_default() {
        let config = OracleConfig::default();
        assert!(config
            .stable_mints
            .contains("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"));
        assert!(config
            .stable_mints
            .contains("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"));
        assert_eq!(config.cache_ttl_secs, 60);
    }

    #[test]
    fn test_cached_price_new() {
        let price = CachedPrice::new(100.5);
        assert_eq!(price.price, 100.5);
        assert!(price.age() < Duration::from_secs(1));
    }

    #[test]
    fn test_cached_price_not_expired() {
        let price = CachedPrice::new(50.0);
        assert!(!price.is_expired(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn test_jupiter_oracle_stable_mint() {
        let config = OracleConfig::default();
        let oracle = JupiterQuoteOracle::new(config);

        let price = oracle
            .fetch_price_usd("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
            .await
            .unwrap();
        assert_eq!(price, 1.0);
    }

    #[test]
    fn test_jupiter_oracle_is_stable_mint() {
        let config = OracleConfig::default();
        let oracle = JupiterQuoteOracle::new(config);

        assert!(oracle.is_stable_mint("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"));
        assert!(oracle.is_stable_mint("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"));
        assert!(!oracle.is_stable_mint("So11111111111111111111111111111111111111112"));
    }

    #[test]
    fn test_jupiter_oracle_name() {
        let config = OracleConfig::default();
        let oracle = JupiterQuoteOracle::new(config);
        assert_eq!(oracle.name(), "JupiterQuoteOracle");
    }
}
