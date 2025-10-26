/// Datanalyzer - Solana DEX Pool Monitor Library
///
/// This library provides tools for decoding and monitoring Solana DEX pools.
pub mod config;
pub mod csv_writer;
pub mod dex;
pub mod error;
pub mod healthcheck;
pub mod liquidity;
pub mod metrics;
pub mod models;
pub mod orchestrator;
pub mod price_fetcher;
pub mod price_provider;
pub mod token_mapping;
pub mod token_metadata;
pub mod websocket;

// Re-export commonly used types
pub use config::{
    AppConfig, PersistenceConfig, PoolConfig, PriceFetcherConfig, RateLimitConfig, RetryConfig,
    RuntimeConfig,
};
pub use csv_writer::{CsvWriter, CsvWriterConfig};
pub use dex::{create_decoder, DecoderRegistry, DecoderStats, DexDecoder};
pub use error::AppError;
pub use healthcheck::{AppState, HealthResponse, ReadinessResponse};
pub use metrics::WebSocketMetrics;
pub use models::{DexType, PoolSnapshot};
pub use orchestrator::Orchestrator;
pub use price_fetcher::{CachedPrice, PriceFetcher, PriceFetcherMetrics};
pub use price_provider::{
    CircuitBreaker, CircuitBreakerState, CoinGeckoPriceProvider, FallbackPriceProvider,
    JupiterPriceProvider, PriceProvider,
};
pub use token_mapping::{StaticTokenMapping, TokenMappingEntry, TokenMappingProvider, TokenMappingService};
pub use token_metadata::{CachedMetadata, TokenMetadata, TokenMetadataProvider};
pub use websocket::{ReconnectStrategy, WebSocketManager};
