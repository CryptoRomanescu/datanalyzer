/// Price provider module implementing the fallback chain for price fetching.
///
/// This module provides:
/// - Trait for price providers (Jupiter, CoinGecko, etc.)
/// - Circuit breaker pattern for rate limit handling
/// - Fallback chain: Jupiter -> CoinGecko -> stale cache
/// - Per-token TTL configuration
use crate::error::AppError;
use crate::price_fetcher::{CachedPrice, PriceFetcher};
use crate::token_mapping::TokenMappingService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Response structure from Jupiter price API
#[derive(Debug, Deserialize, Serialize)]
pub struct JupiterPriceResponse {
    pub data: HashMap<String, JupiterPriceData>,
}

/// Price data structure from Jupiter
#[derive(Debug, Deserialize, Serialize)]
pub struct JupiterPriceData {
    pub id: String,
    #[serde(rename = "mintSymbol")]
    pub mint_symbol: Option<String>,
    pub vs_token: String,
    #[serde(rename = "vsTokenSymbol")]
    pub vs_token_symbol: String,
    pub price: f64,
}

/// Circuit breaker state for handling rate limits
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitBreakerState {
    Closed,   // Normal operation
    Open,     // Too many failures, stop trying
    HalfOpen, // Testing if service recovered
}

/// Circuit breaker for managing API failures and rate limits
#[derive(Debug)]
pub struct CircuitBreaker {
    state: CircuitBreakerState,
    failure_count: u32,
    last_failure_time: Option<Instant>,
    threshold: u32,
    timeout: Duration,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(threshold: u32, timeout: Duration) -> Self {
        Self {
            state: CircuitBreakerState::Closed,
            failure_count: 0,
            last_failure_time: None,
            threshold,
            timeout,
        }
    }

    /// Check if the circuit breaker allows requests
    pub fn can_request(&mut self) -> bool {
        match self.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                // Check if timeout has elapsed
                if let Some(last_failure) = self.last_failure_time {
                    if last_failure.elapsed() >= self.timeout {
                        log::info!("Circuit breaker transitioning to half-open state");
                        self.state = CircuitBreakerState::HalfOpen;
                        self.failure_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    /// Record a successful request
    pub fn record_success(&mut self) {
        if self.state == CircuitBreakerState::HalfOpen {
            log::info!("Circuit breaker closing after successful request");
            self.state = CircuitBreakerState::Closed;
        }
        self.failure_count = 0;
        self.last_failure_time = None;
    }

    /// Record a failed request
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(Instant::now());

        if self.failure_count >= self.threshold && self.state != CircuitBreakerState::Open {
            log::warn!(
                "Circuit breaker opening after {} failures",
                self.failure_count
            );
            self.state = CircuitBreakerState::Open;
        }
    }

    /// Get the current state
    pub fn state(&self) -> CircuitBreakerState {
        self.state.clone()
    }

    /// Reset the circuit breaker
    pub fn reset(&mut self) {
        self.state = CircuitBreakerState::Closed;
        self.failure_count = 0;
        self.last_failure_time = None;
    }
}

/// Trait for price providers
#[async_trait::async_trait]
pub trait PriceProvider: Send + Sync {
    /// Fetch price for a single token mint address
    async fn fetch_price(&self, mint: &str) -> Result<f64, AppError>;

    /// Get the name of the provider
    fn name(&self) -> &str;

    /// Check if the provider is currently available
    async fn is_available(&self) -> bool {
        true // Default implementation
    }
}

/// Jupiter price provider
pub struct JupiterPriceProvider {
    client: reqwest::Client,
    api_url: String,
    cache: Arc<RwLock<HashMap<String, CachedPrice>>>,
    cache_ttl: Duration,
    circuit_breaker: Arc<RwLock<CircuitBreaker>>,
}

impl JupiterPriceProvider {
    /// Create a new Jupiter price provider
    pub fn new(cache_ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        // Circuit breaker: open after 3 failures within 60 seconds
        let circuit_breaker =
            Arc::new(RwLock::new(CircuitBreaker::new(3, Duration::from_secs(60))));

        Self {
            client,
            api_url: "https://api.jup.ag/price/v2".to_string(),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
            circuit_breaker,
        }
    }

    /// Create a Jupiter price provider with custom API URL
    pub fn with_config(api_url: String, cache_ttl: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        let circuit_breaker =
            Arc::new(RwLock::new(CircuitBreaker::new(3, Duration::from_secs(60))));

        Self {
            client,
            api_url,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl,
            circuit_breaker,
        }
    }

    /// Fetch price directly from Jupiter API
    async fn fetch_from_api(&self, mint: &str) -> Result<f64, AppError> {
        let url = format!("{}?ids={}", self.api_url, mint);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::PriceError(format!("Jupiter API request failed: {}", e)))?;

        // Check for rate limiting (429)
        if response.status().as_u16() == 429 {
            log::warn!("Jupiter API rate limit hit (429)");
            return Err(AppError::PriceError(
                "Rate limit exceeded (429)".to_string(),
            ));
        }

        if !response.status().is_success() {
            return Err(AppError::PriceError(format!(
                "Jupiter API returned error status: {}",
                response.status()
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| AppError::PriceError(format!("Failed to read response body: {}", e)))?;

        let parsed: JupiterPriceResponse = serde_json::from_str(&body).map_err(|e| {
            AppError::PriceError(format!("Failed to parse Jupiter response: {}", e))
        })?;

        parsed.data.get(mint).map(|data| data.price).ok_or_else(|| {
            AppError::PriceError(format!("Token {} not found in Jupiter response", mint))
        })
    }
}

#[async_trait::async_trait]
impl PriceProvider for JupiterPriceProvider {
    async fn fetch_price(&self, mint: &str) -> Result<f64, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(mint) {
                if !cached.is_expired(self.cache_ttl) {
                    log::debug!("Jupiter cache hit for mint: {}", mint);
                    return Ok(cached.price());
                }
            }
        }

        // Check circuit breaker
        {
            let mut cb = self.circuit_breaker.write().await;
            if !cb.can_request() {
                log::warn!("Jupiter circuit breaker is open, skipping request");
                return Err(AppError::PriceError("Circuit breaker is open".to_string()));
            }
        }

        // Fetch from API
        match self.fetch_from_api(mint).await {
            Ok(price) => {
                // Update cache
                {
                    let mut cache = self.cache.write().await;
                    cache.insert(mint.to_string(), CachedPrice::new(price));
                }

                // Record success in circuit breaker
                {
                    let mut cb = self.circuit_breaker.write().await;
                    cb.record_success();
                }

                Ok(price)
            }
            Err(e) => {
                // Record failure in circuit breaker
                {
                    let mut cb = self.circuit_breaker.write().await;
                    cb.record_failure();
                }

                Err(e)
            }
        }
    }

    fn name(&self) -> &str {
        "Jupiter"
    }

    async fn is_available(&self) -> bool {
        let cb = self.circuit_breaker.read().await;
        cb.state() != CircuitBreakerState::Open
    }
}

/// CoinGecko price provider (wrapper around PriceFetcher)
pub struct CoinGeckoPriceProvider {
    fetcher: Arc<PriceFetcher>,
    token_mapping: Arc<TokenMappingService>,
    circuit_breaker: Arc<RwLock<CircuitBreaker>>,
}

impl CoinGeckoPriceProvider {
    /// Create a new CoinGecko price provider
    pub fn new(fetcher: Arc<PriceFetcher>, token_mapping: Arc<TokenMappingService>) -> Self {
        let circuit_breaker =
            Arc::new(RwLock::new(CircuitBreaker::new(3, Duration::from_secs(60))));

        Self {
            fetcher,
            token_mapping,
            circuit_breaker,
        }
    }
}

#[async_trait::async_trait]
impl PriceProvider for CoinGeckoPriceProvider {
    async fn fetch_price(&self, mint: &str) -> Result<f64, AppError> {
        // Check circuit breaker
        {
            let mut cb = self.circuit_breaker.write().await;
            if !cb.can_request() {
                log::warn!("CoinGecko circuit breaker is open, skipping request");
                return Err(AppError::PriceError("Circuit breaker is open".to_string()));
            }
        }

        // Map mint to CoinGecko token ID
        let token_id = self
            .token_mapping
            .get_token_id(mint)
            .await?
            .ok_or_else(|| AppError::PriceError(format!("No token mapping for mint: {}", mint)))?;

        // Fetch price using CoinGecko API
        match self.fetcher.fetch_price(&token_id).await {
            Ok(price) => {
                // Record success in circuit breaker
                {
                    let mut cb = self.circuit_breaker.write().await;
                    cb.record_success();
                }
                Ok(price)
            }
            Err(e) => {
                // Record failure in circuit breaker
                {
                    let mut cb = self.circuit_breaker.write().await;
                    cb.record_failure();
                }
                Err(e)
            }
        }
    }

    fn name(&self) -> &str {
        "CoinGecko"
    }

    async fn is_available(&self) -> bool {
        let cb = self.circuit_breaker.read().await;
        cb.state() != CircuitBreakerState::Open
    }
}

/// Fallback price provider chain: Jupiter -> CoinGecko -> stale cache
pub struct FallbackPriceProvider {
    providers: Vec<Arc<dyn PriceProvider>>,
    stale_cache: Arc<RwLock<HashMap<String, CachedPrice>>>,
}

impl FallbackPriceProvider {
    /// Create a new fallback price provider with the given chain
    pub fn new(providers: Vec<Arc<dyn PriceProvider>>) -> Self {
        Self {
            providers,
            stale_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Fetch price using fallback chain
    pub async fn fetch_price(&self, mint: &str) -> Result<f64, AppError> {
        // Try each provider in order
        for provider in &self.providers {
            log::debug!("Trying provider: {}", provider.name());

            match provider.fetch_price(mint).await {
                Ok(price) => {
                    log::debug!(
                        "Successfully fetched price from {}: {}",
                        provider.name(),
                        price
                    );

                    // Update stale cache
                    {
                        let mut cache = self.stale_cache.write().await;
                        cache.insert(mint.to_string(), CachedPrice::new(price));
                    }

                    return Ok(price);
                }
                Err(e) => {
                    log::warn!(
                        "Provider {} failed for mint {}: {}",
                        provider.name(),
                        mint,
                        e
                    );
                    continue; // Try next provider
                }
            }
        }

        // All providers failed, try stale cache
        log::warn!("All providers failed for mint {}, trying stale cache", mint);
        let cache = self.stale_cache.read().await;
        if let Some(cached) = cache.get(mint) {
            log::warn!(
                "Using stale cached price for {} (age: {:?})",
                mint,
                cached.age()
            );
            Ok(cached.price())
        } else {
            Err(AppError::PriceError(format!(
                "Failed to fetch price for {} from all providers and no stale cache available",
                mint
            )))
        }
    }

    /// Get the number of active providers
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_new() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(cb.failure_count, 0);
    }

    #[test]
    fn test_circuit_breaker_can_request_closed() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert!(cb.can_request());
    }

    #[test]
    fn test_circuit_breaker_record_success() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        assert_eq!(cb.failure_count, 1);

        cb.record_success();
        assert_eq!(cb.failure_count, 0);
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
    }

    #[test]
    fn test_circuit_breaker_cannot_request_when_open() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));

        // Open the circuit breaker
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();

        assert!(!cb.can_request());
    }

    #[tokio::test]
    async fn test_circuit_breaker_transitions_to_half_open() {
        let mut cb = CircuitBreaker::new(3, Duration::from_millis(100));

        // Open the circuit breaker
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should transition to half-open
        assert!(cb.can_request());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);
    }

    #[test]
    fn test_circuit_breaker_half_open_closes_on_success() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.state = CircuitBreakerState::HalfOpen;

        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_circuit_breaker_reset() {
        let mut cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(cb.failure_count, 0);
    }

    #[test]
    fn test_jupiter_price_provider_new() {
        let provider = JupiterPriceProvider::new(Duration::from_secs(300));
        assert_eq!(provider.name(), "Jupiter");
    }

    #[test]
    fn test_jupiter_price_provider_with_config() {
        let provider = JupiterPriceProvider::with_config(
            "https://api.example.com".to_string(),
            Duration::from_secs(600),
        );
        assert_eq!(provider.api_url, "https://api.example.com");
    }

    #[tokio::test]
    async fn test_jupiter_price_provider_is_available() {
        let provider = JupiterPriceProvider::new(Duration::from_secs(300));
        assert!(provider.is_available().await);
    }

    #[test]
    fn test_fallback_price_provider_new() {
        let providers: Vec<Arc<dyn PriceProvider>> = vec![];
        let fallback = FallbackPriceProvider::new(providers);
        assert_eq!(fallback.provider_count(), 0);
    }

    #[test]
    fn test_fallback_price_provider_count() {
        let provider1 =
            Arc::new(JupiterPriceProvider::new(Duration::from_secs(300))) as Arc<dyn PriceProvider>;
        let providers = vec![provider1];
        let fallback = FallbackPriceProvider::new(providers);
        assert_eq!(fallback.provider_count(), 1);
    }
}
