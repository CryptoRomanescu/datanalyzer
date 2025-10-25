/// Datanalyzer - Solana DEX Pool Monitor Library
///
/// This library provides tools for decoding and monitoring Solana DEX pools.

pub mod config;
pub mod dex;
pub mod error;
pub mod liquidity;
pub mod models;
pub mod price_fetcher;
pub mod websocket;

// Re-export commonly used types
pub use dex::{create_decoder, DexDecoder, DecoderRegistry, DecoderStats};
pub use error::AppError;
pub use models::{DexType, PoolSnapshot};
pub use price_fetcher::{PriceFetcher, CachedPrice, PriceFetcherMetrics};
pub use websocket::WebSocketManager;
