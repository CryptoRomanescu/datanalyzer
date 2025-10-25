/// Datanalyzer - Solana DEX Pool Monitor Library
///
/// This library provides tools for decoding and monitoring Solana DEX pools.
pub mod config;
pub mod dex;
pub mod error;
pub mod healthcheck;
pub mod liquidity;
pub mod metrics;
pub mod models;
pub mod orchestrator;
pub mod price_fetcher;
pub mod websocket;

// Re-export commonly used types
pub use dex::{create_decoder, DecoderRegistry, DecoderStats, DexDecoder};
pub use error::AppError;
pub use healthcheck::{AppState, HealthResponse, ReadinessResponse};
pub use metrics::WebSocketMetrics;
pub use models::{DexType, PoolSnapshot};
pub use orchestrator::{ReserveInfo, ReserveOrchestrator};
pub use price_fetcher::{CachedPrice, PriceFetcher, PriceFetcherMetrics};
pub use websocket::{ReconnectStrategy, WebSocketManager};
