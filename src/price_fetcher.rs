
/// Price fetcher module for retrieving token prices and metadata.
///
/// This module provo fetch token prices from external sources
/// and cache token metadata like decimals to avoid repeated RPC calls.

use crate::error::AppError;
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Default number of decimals to use when fetching fails
const DEFAULT_TOKEN_DECIMALS: u8 = 9;

/// Wrapped Solana (SOL) mint address on Solana
const WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";

/// Structure to hold token price information
#[derive(Debug, Clone)]
pub struct TokenPrice {
    /// Token mint address
    pub mint: Pubkey,
    /// Price in USD
    pub price_usd: f64,
}

/// Price fetcher with caching capabilities.
///
/// This structure maintains a cache of token decimals to avoid repeated
/// RPC calls when fetching token metadata.
pub struct PriceFetcher {
    /// RPC client for fetching on-chain data
    rpc_client: Arc<RpcClient>,
    /// Cache for token decimals (mint address -> decimals)
    decimals_cache: Arc<RwLock<HashMap<Pubkey, u8>>>,
}

impl PriceFetcher {
    /// Create a new PriceFetcher instance.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - URL of the Solana RPC endpoint
    ///
    /// # Returns
    ///
    /// * `PriceFetcher` - New instance ready to fetch prices
    pub fn new(rpc_url: &str) -> Self {
        let rpc_client = Arc::new(RpcClient::new(rpc_url.to_string()));
        let mut decimals_cache = HashMap::new();
        
        // Pre-populate cache with known token decimals
        // SOL has 9 decimals
        if let Ok(sol_mint) = WRAPPED_SOL_MINT.parse::<Pubkey>() {
            decimals_cache.insert(sol_mint, 9);
        }
        
        Self {
            rpc_client,
            decimals_cache: Arc::new(RwLock::new(decimals_cache)),
        }
    }

    /// Fetch prices for multiple tokens.
    ///
    /// This method fetches current USD prices for the specified token mints.
    /// In a real implementation, this would call external price APIs like CoinGecko,
    /// Jupiter, or other price oracles. For now, it returns placeholder values.
    ///
    /// # Arguments
    ///
    /// * `mints` - Slice of token mint addresses to fetch prices for
    ///
    /// # Returns
    ///
    /// * `Ok(HashMap<Pubkey, f64>)` - Map of mint addresses to prices in USD
    /// * `Err(AppError)` - If fetching prices fails
    ///
    /// # Note
    ///
    /// This is a placeholder implementation. In production, you would:
    /// 1. Call a price API (CoinGecko, Jupiter, etc.)
    /// 2. Handle rate limiting
    /// 3. Implement proper error handling
    /// 4. Cache prices to avoid excessive API calls
    pub async fn fetch_prices(&self, mints: &[Pubkey]) -> Result<HashMap<Pubkey, f64>, AppError> {
        // Placeholder implementation
        // In production, this would fetch from price APIs
        let mut prices = HashMap::new();
        
        for mint in mints {
            // Check if this is the SOL mint
            if mint.to_string() == WRAPPED_SOL_MINT {
                // Placeholder SOL price
                prices.insert(*mint, 100.0);
            } else {
                // Placeholder token price
                // In production, fetch from API
                prices.insert(*mint, 1.0);
            }
        }
        
        Ok(prices)
    }

    /// Get token decimals from mint account.
    ///
    /// This method fetches the decimals value from the token mint account.
    /// Results are cached to avoid repeated RPC calls.
    ///
    /// # Arguments
    ///
    /// * `mint` - Token mint address
    ///
    /// # Returns
    ///
    /// * `Ok(u8)` - Number of decimals for the token
    /// * `Err(AppError)` - If fetching fails (returns default value instead)
    ///
    /// # Behavior
    ///
    /// 1. Check cache first
    /// 2. If not in cache, fetch from RPC
    /// 3. Store in cache for future use
    /// 4. If fetch fails, use default value (9) and log warning
    pub async fn get_token_decimals(&self, mint: &Pubkey) -> Result<u8, AppError> {
        // Check cache first
        {
            let cache = self.decimals_cache.read()
                .map_err(|e| AppError::PriceError(format!("Failed to acquire read lock: {}", e)))?;
            
            if let Some(&decimals) = cache.get(mint) {
                return Ok(decimals);
            }
        }

        // Not in cache, fetch from RPC
        let decimals = match self.fetch_decimals_from_rpc(mint).await {
            Ok(decimals) => decimals,
            Err(e) => {
                log::warn!(
                    "Failed to fetch decimals for mint {}: {}. Using default {}",
                    mint,
                    e,
                    DEFAULT_TOKEN_DECIMALS
                );
                DEFAULT_TOKEN_DECIMALS
            }
        };

        // Store in cache
        {
            let mut cache = self.decimals_cache.write()
                .map_err(|e| AppError::PriceError(format!("Failed to acquire write lock: {}", e)))?;
            
            cache.insert(*mint, decimals);
        }

        Ok(decimals)
    }

    /// Fetch decimals from the token mint account via RPC.
    ///
    /// This is an internal method that performs the actual RPC call.
    ///
    /// # Arguments
    ///
    /// * `mint` - Token mint address
    ///
    /// # Returns
    ///
    /// * `Ok(u8)` - Number of decimals
    /// * `Err(AppError)` - If RPC call or parsing fails
    async fn fetch_decimals_from_rpc(&self, mint: &Pubkey) -> Result<u8, AppError> {
        // Fetch the mint account data
        let account_data = self.rpc_client
            .get_account_data(mint)
            .map_err(|e| AppError::RpcError(format!("Failed to fetch mint account: {}", e)))?;

        // Parse the mint account to extract decimals
        // SPL Token mint account structure:
        // - Bytes 0-3: Not used in our case
        // - Bytes 4-35: mint_authority (32 bytes pubkey)
        // - Bytes 36-43: supply (u64)
        // - Byte 44: decimals (u8)
        if account_data.len() < 45 {
            return Err(AppError::DecodingError(format!(
                "Mint account data too small: {} bytes",
                account_data.len()
            )));
        }

        let decimals = account_data[44];
        Ok(decimals)
    }

    /// Get the current size of the decimals cache.
    ///
    /// Useful for monitoring and debugging.
    pub fn cache_size(&self) -> usize {
        self.decimals_cache.read()
            .map(|cache| cache.len())
            .unwrap_or(0)
    }

    /// Clear the decimals cache.
    ///
    /// Useful for testing or when you want to force refresh of all decimals.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.decimals_cache.write() {
            cache.clear();
        }
    }

    /// Get a clone of the entire decimals cache.
    ///
    /// Useful for debugging and monitoring purposes.
    pub fn get_cache_snapshot(&self) -> HashMap<Pubkey, u8> {
        self.decimals_cache.read()
            .map(|cache| cache.clone())
            .unwrap_or_default()

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;
use serde::{Deserialize, Serialize};
use crate::error::AppError;

/// Structure to cache price data with timestamp
#[derive(Debug, Clone)]
pub struct CachedPrice {
    /// Price in USD
    price: f64,
    /// Timestamp when the price was fetched
    timestamp: Instant,
}

impl CachedPrice {
    /// Create a new CachedPrice
    pub fn new(price: f64) -> Self {
        Self {
            price,
            timestamp: Instant::now(),
        }
    }

    /// Check if the cached price is expired
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.timestamp.elapsed() > ttl
    }

    /// Get the age of the cached price
    pub fn age(&self) -> Duration {
        self.timestamp.elapsed()
    }

    /// Get the cached price value
    pub fn price(&self) -> f64 {
        self.price
    }
}

/// Response structure from CoinGecko API for simple price endpoint
#[derive(Debug, Deserialize, Serialize)]
struct CoinGeckoSimplePrice {
    #[serde(rename = "usd")]
    usd: f64,
}

/// Metrics for tracking API performance
#[derive(Debug, Clone, Default)]
pub struct PriceFetcherMetrics {
    /// Total number of requests made
    pub total_requests: u64,
    /// Number of successful requests
    pub successful_requests: u64,
    /// Number of failed requests
    pub failed_requests: u64,
    /// Total response time in milliseconds
    pub total_response_time_ms: u64,
}

impl PriceFetcherMetrics {
    /// Calculate success rate as a percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            (self.successful_requests as f64 / self.total_requests as f64) * 100.0
        }
    }

    /// Calculate average response time in milliseconds
    pub fn avg_response_time_ms(&self) -> f64 {
        if self.successful_requests == 0 {
            0.0
        } else {
            self.total_response_time_ms as f64 / self.successful_requests as f64
        }
    }
}

/// Main structure for fetching cryptocurrency prices
pub struct PriceFetcher {
    /// Reusable HTTP client
    client: reqwest::Client,
    /// Thread-safe cache for prices
    cache: Arc<RwLock<HashMap<String, CachedPrice>>>,
    /// API URL for price fetching
    api_url: String,
    /// Cache time-to-live duration
    cache_ttl: Duration,
    /// Metrics for tracking performance
    metrics: Arc<RwLock<PriceFetcherMetrics>>,
}

impl PriceFetcher {
    /// Create a new PriceFetcher with CoinGecko API
    pub fn new(cache_ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            api_url: "https://api.coingecko.com/api/v3".to_string(),
            cache_ttl,
            metrics: Arc::new(RwLock::new(PriceFetcherMetrics::default())),
        }
    }

    /// Create a new PriceFetcher with custom API URL and cache TTL
    pub fn with_config(api_url: String, cache_ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            api_url,
            cache_ttl,
            metrics: Arc::new(RwLock::new(PriceFetcherMetrics::default())),
        }
    }

    /// Fetch price for a single token with retry logic
    pub async fn fetch_price(&self, token_id: &str) -> Result<f64, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(token_id) {
                if !cached.is_expired(self.cache_ttl) {
                    log::debug!("Cache hit for token: {}", token_id);
                    return Ok(cached.price());
                }
            }
        }

        log::debug!("Cache miss for token: {}, fetching from API", token_id);
        
        // Fetch from API with retry logic
        let price = self.fetch_with_retry(token_id, 3).await?;
        
        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.insert(token_id.to_string(), CachedPrice::new(price));
        }

        Ok(price)
    }

    /// Fetch price with retry logic for transient errors
    async fn fetch_with_retry(&self, token_id: &str, max_retries: u32) -> Result<f64, AppError> {
        let mut last_error = None;
        
        for attempt in 0..max_retries {
            if attempt > 0 {
                let delay = Duration::from_millis(500 * (1 << attempt)); // Exponential backoff
                log::debug!("Retry attempt {} after {:?} delay", attempt + 1, delay);
                sleep(delay).await;
            }

            let start_time = Instant::now();
            let result = self.fetch_price_from_api(token_id).await;
            let elapsed = start_time.elapsed();

            // Update metrics
            {
                let mut metrics = self.metrics.write().await;
                metrics.total_requests += 1;
                
                if result.is_ok() {
                    metrics.successful_requests += 1;
                    metrics.total_response_time_ms += elapsed.as_millis() as u64;
                } else {
                    metrics.failed_requests += 1;
                }
            }

            match result {
                Ok(price) => return Ok(price),
                Err(e) => {
                    log::warn!("Attempt {} failed for token {}: {}", attempt + 1, token_id, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| 
            AppError::PriceError(format!("Failed to fetch price for {} after {} retries", token_id, max_retries))
        ))
    }

    /// Fetch price directly from API
    async fn fetch_price_from_api(&self, token_id: &str) -> Result<f64, AppError> {
        let url = format!(
            "{}/simple/price?ids={}&vs_currencies=usd",
            self.api_url, token_id
        );

        let response = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::PriceError(format!("HTTP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(AppError::PriceError(
                format!("API returned error status: {}", response.status())
            ));
        }

        let body = response
            .text()
            .await
            .map_err(|e| AppError::PriceError(format!("Failed to read response body: {}", e)))?;

        let parsed: HashMap<String, CoinGeckoSimplePrice> = serde_json::from_str(&body)
            .map_err(|e| AppError::PriceError(format!("Failed to parse JSON response: {}", e)))?;

        parsed
            .get(token_id)
            .map(|p| p.usd)
            .ok_or_else(|| AppError::PriceError(format!("Token {} not found in response", token_id)))
    }

    /// Fetch prices for multiple tokens using batch request
    pub async fn fetch_prices(&self, token_ids: &[String]) -> Result<HashMap<String, f64>, AppError> {
        if token_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut result = HashMap::new();
        let mut to_fetch = Vec::new();

        // Check cache for each token
        {
            let cache = self.cache.read().await;
            for token_id in token_ids {
                if let Some(cached) = cache.get(token_id) {
                    if !cached.is_expired(self.cache_ttl) {
                        result.insert(token_id.clone(), cached.price());
                        continue;
                    }
                }
                to_fetch.push(token_id.clone());
            }
        }

        if to_fetch.is_empty() {
            return Ok(result);
        }

        log::debug!("Fetching {} tokens from API", to_fetch.len());

        // Batch fetch from API (CoinGecko supports comma-separated IDs)
        let token_ids_str = to_fetch.join(",");
        let url = format!(
            "{}/simple/price?ids={}&vs_currencies=usd",
            self.api_url, token_ids_str
        );

        let start_time = Instant::now();
        let fetch_result = self.client
            .get(&url)
            .send()
            .await;
        let elapsed = start_time.elapsed();

        // Update metrics
        {
            let mut metrics = self.metrics.write().await;
            metrics.total_requests += 1;
        }

        match fetch_result {
            Ok(response) => {
                if !response.status().is_success() {
                    log::warn!("Batch API request failed with status: {}", response.status());
                    // Fall back to individual requests with retry
                    return self.fetch_prices_individually(&to_fetch).await;
                }

                let body = response
                    .text()
                    .await
                    .map_err(|e| AppError::PriceError(format!("Failed to read response body: {}", e)))?;

                let parsed: HashMap<String, CoinGeckoSimplePrice> = serde_json::from_str(&body)
                    .map_err(|e| AppError::PriceError(format!("Failed to parse JSON response: {}", e)))?;

                // Update metrics and cache
                {
                    let mut metrics = self.metrics.write().await;
                    metrics.successful_requests += 1;
                    metrics.total_response_time_ms += elapsed.as_millis() as u64;
                }

                {
                    let mut cache = self.cache.write().await;
                    for (token_id, price_data) in parsed.iter() {
                        let price = price_data.usd;
                        result.insert(token_id.clone(), price);
                        cache.insert(token_id.clone(), CachedPrice::new(price));
                    }
                }

                // Log warning for missing tokens
                for token_id in &to_fetch {
                    if !result.contains_key(token_id) {
                        log::warn!("Token {} not found in batch response", token_id);
                    }
                }

                Ok(result)
            }
            Err(e) => {
                log::warn!("Batch request failed: {}, falling back to individual requests", e);
                let mut metrics = self.metrics.write().await;
                metrics.failed_requests += 1;
                drop(metrics);
                
                // Fall back to individual requests
                self.fetch_prices_individually(&to_fetch).await
            }
        }
    }

    /// Fetch prices individually with retry logic (fallback for batch failure)
    async fn fetch_prices_individually(&self, token_ids: &[String]) -> Result<HashMap<String, f64>, AppError> {
        let mut result = HashMap::new();
        
        for token_id in token_ids {
            match self.fetch_with_retry(token_id, 3).await {
                Ok(price) => {
                    result.insert(token_id.clone(), price);
                }
                Err(e) => {
                    log::warn!("Failed to fetch price for {}: {}", token_id, e);
                    // Try to use stale cache as fallback
                    let cache = self.cache.read().await;
                    if let Some(cached) = cache.get(token_id) {
                        log::warn!("Using stale cached price for {} (age: {:?})", token_id, cached.age());
                        result.insert(token_id.clone(), cached.price());
                    }
                }
            }
            
            // Rate limiting: sleep between individual requests
            sleep(Duration::from_millis(200)).await;
        }

        Ok(result)
    }

    /// Start background refresh loop for specified tokens
    pub async fn start_refresh_loop(
        self: Arc<Self>,
        tokens: Vec<String>,
        shutdown_signal: tokio::sync::watch::Receiver<bool>,
    ) {
        tokio::spawn(async move {
            let mut shutdown_rx = shutdown_signal;
            let refresh_interval = self.cache_ttl / 2;
            
            log::info!("Starting price refresh loop with interval: {:?}", refresh_interval);
            
            loop {
                tokio::select! {
                    _ = sleep(refresh_interval) => {
                        log::debug!("Refreshing prices for {} tokens", tokens.len());
                        
                        match self.fetch_prices(&tokens).await {
                            Ok(prices) => {
                                log::debug!("Successfully refreshed {} prices", prices.len());
                            }
                            Err(e) => {
                                log::error!("Failed to refresh prices: {}", e);
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            log::info!("Shutting down price refresh loop");
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Get price with fallback to stale cache
    pub async fn get_price_with_fallback(&self, token_id: &str) -> Result<f64, AppError> {
        match self.fetch_price(token_id).await {
            Ok(price) => Ok(price),
            Err(e) => {
                log::warn!("Failed to fetch fresh price for {}: {}", token_id, e);
                
                // Try stale cache
                let cache = self.cache.read().await;
                if let Some(cached) = cache.get(token_id) {
                    log::warn!("Using stale cached price for {} (age: {:?})", token_id, cached.age());
                    Ok(cached.price())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Get current metrics
    pub async fn get_metrics(&self) -> PriceFetcherMetrics {
        self.metrics.read().await.clone()
    }

    /// Reset metrics
    pub async fn reset_metrics(&self) {
        let mut metrics = self.metrics.write().await;
        *metrics = PriceFetcherMetrics::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    // Note: These tests are basic structural tests.
    // Full integration tests would require a mock RPC client or test network.

    #[test]
    fn test_price_fetcher_new() {
        let fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
        
        // Should successfully create a fetcher
        let size = fetcher.cache_size();
        assert!(size == 0 || size > 0);  // Just checking it doesn't panic
    }

    #[test]
    fn test_cache_size() {
        let fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
        let size = fetcher.cache_size();
        // Cache size should be a valid usize
        let _ = size;  // Just ensure it compiles
    }

    #[test]
    fn test_clear_cache() {
        let fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
        
        // Clear the cache
        fetcher.clear_cache();
        
        // Should be empty or have default entries
        let size = fetcher.cache_size();
        assert_eq!(size, 0);
    }

    #[test]
    fn test_get_cache_snapshot() {
        let fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
        let snapshot = fetcher.get_cache_snapshot();
        
        // Should return a HashMap
        let _ = snapshot.len();  // Just ensure it works
    }

    #[tokio::test]
    async fn test_fetch_prices_basic() {
        let fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
        
        let sol_mint = WRAPPED_SOL_MINT.parse::<Pubkey>().unwrap();
        let mints = vec![sol_mint];
        
        let result = fetcher.fetch_prices(&mints).await;
        assert!(result.is_ok());
        
        let prices = result.unwrap();
        assert_eq!(prices.len(), 1);
        assert!(prices.contains_key(&sol_mint));
    }

    #[test]
    fn test_wrapped_sol_mint_valid() {
        let result = WRAPPED_SOL_MINT.parse::<Pubkey>();
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_price_structure() {
        let mint = Pubkey::new_unique();
        let price = TokenPrice {
            mint,
            price_usd: 100.0,
        };
        
        assert_eq!(price.mint, mint);
        assert_eq!(price.price_usd, 100.0);
=======
    #[test]
    fn test_cached_price_new() {
        let price = CachedPrice::new(100.5);
        assert_eq!(price.price(), 100.5);
        assert!(price.age() < Duration::from_secs(1));
    }

    #[test]
    fn test_cached_price_not_expired() {
        let price = CachedPrice::new(50.0);
        assert!(!price.is_expired(Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn test_cached_price_expired() {
        let price = CachedPrice::new(75.0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(price.is_expired(Duration::from_millis(50)));
    }

    #[test]
    fn test_cached_price_age() {
        let price = CachedPrice::new(25.0);
        std::thread::sleep(Duration::from_millis(50));
        let age = price.age();
        assert!(age >= Duration::from_millis(50));
        assert!(age < Duration::from_millis(200));
    }

    #[test]
    fn test_price_fetcher_new() {
        let fetcher = PriceFetcher::new(Duration::from_secs(300));
        assert_eq!(fetcher.cache_ttl, Duration::from_secs(300));
    }

    #[test]
    fn test_price_fetcher_with_config() {
        let custom_url = "https://api.example.com".to_string();
        let ttl = Duration::from_secs(600);
        let fetcher = PriceFetcher::with_config(custom_url.clone(), ttl);
        assert_eq!(fetcher.api_url, custom_url);
        assert_eq!(fetcher.cache_ttl, ttl);
    }

    #[test]
    fn test_metrics_default() {
        let metrics = PriceFetcherMetrics::default();
        assert_eq!(metrics.total_requests, 0);
        assert_eq!(metrics.successful_requests, 0);
        assert_eq!(metrics.failed_requests, 0);
        assert_eq!(metrics.total_response_time_ms, 0);
    }

    #[test]
    fn test_metrics_success_rate() {
        let mut metrics = PriceFetcherMetrics::default();
        metrics.total_requests = 10;
        metrics.successful_requests = 8;
        metrics.failed_requests = 2;
        
        assert_eq!(metrics.success_rate(), 80.0);
    }

    #[test]
    fn test_metrics_success_rate_zero_requests() {
        let metrics = PriceFetcherMetrics::default();
        assert_eq!(metrics.success_rate(), 0.0);
    }

    #[test]
    fn test_metrics_avg_response_time() {
        let mut metrics = PriceFetcherMetrics::default();
        metrics.successful_requests = 5;
        metrics.total_response_time_ms = 1000;
        
        assert_eq!(metrics.avg_response_time_ms(), 200.0);
    }

    #[test]
    fn test_metrics_avg_response_time_zero_requests() {
        let metrics = PriceFetcherMetrics::default();
        assert_eq!(metrics.avg_response_time_ms(), 0.0);
    }

    #[tokio::test]
    async fn test_fetch_prices_empty_input() {
        let fetcher = PriceFetcher::new(Duration::from_secs(300));
        let result = fetcher.fetch_prices(&[]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_get_metrics() {
        let fetcher = PriceFetcher::new(Duration::from_secs(300));
        let metrics = fetcher.get_metrics().await;
        assert_eq!(metrics.total_requests, 0);
    }

    #[tokio::test]
    async fn test_reset_metrics() {
        let fetcher = PriceFetcher::new(Duration::from_secs(300));
        
        // Simulate some metrics
        {
            let mut metrics = fetcher.metrics.write().await;
            metrics.total_requests = 10;
            metrics.successful_requests = 8;
        }
        
        fetcher.reset_metrics().await;
        let metrics = fetcher.get_metrics().await;
        assert_eq!(metrics.total_requests, 0);
        assert_eq!(metrics.successful_requests, 0);
    }
}
