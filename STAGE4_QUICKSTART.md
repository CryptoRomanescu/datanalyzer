# Stage 4 Implementation - Quick Start Guide

## Overview

Stage 4 adds robust price fetching capabilities with fallback mechanisms and token mapping. This guide shows you how to use the new features.

## Quick Start

### 1. Configure Token Mappings

Add token mappings to your `config.toml`:

```toml
[[token_mapping]]
mint = "So11111111111111111111111111111111111111112"
coingecko_id = "solana"
cache_ttl_secs = 300

[[token_mapping]]
mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
coingecko_id = "usd-coin"
cache_ttl_secs = 600
```

### 2. Basic Usage Example

```rust
use datanalyzer::{
    TokenMappingEntry, TokenMappingService,
    PriceFetcher, JupiterPriceProvider, CoinGeckoPriceProvider,
    FallbackPriceProvider, PriceProvider,
    TokenMetadataProvider,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup token mapping
    let mappings = vec![
        TokenMappingEntry {
            mint: "So11111111111111111111111111111111111111112".to_string(),
            coingecko_id: "solana".to_string(),
            cache_ttl_secs: Some(300),
        },
    ];
    let mapping = Arc::new(TokenMappingService::with_static_mapping(mappings)?);
    
    // 2. Create price providers
    let jupiter = Arc::new(JupiterPriceProvider::new(Duration::from_secs(300)));
    let coingecko_fetcher = Arc::new(PriceFetcher::new(Duration::from_secs(300)));
    let coingecko = Arc::new(CoinGeckoPriceProvider::new(
        coingecko_fetcher,
        Arc::clone(&mapping),
    ));
    
    // 3. Create fallback chain
    let providers: Vec<Arc<dyn PriceProvider>> = vec![jupiter, coingecko];
    let fallback = FallbackPriceProvider::new(providers);
    
    // 4. Fetch price (tries Jupiter, then CoinGecko, then stale cache)
    let price = fallback.fetch_price("So11111111111111111111111111111111111111112").await?;
    println!("SOL Price: ${:.2}", price);
    
    // 5. Get token metadata
    let metadata = TokenMetadataProvider::new(
        "https://api.mainnet-beta.solana.com".to_string(),
        Duration::from_secs(3600),
    );
    let decimals = metadata.get_decimals("So11111111111111111111111111111111111111112").await?;
    println!("SOL Decimals: {}", decimals);
    
    Ok(())
}
```

## Key Features

### Automatic Fallback

The system automatically falls through providers:

1. **Jupiter API** - Fastest, direct mint queries
2. **CoinGecko API** - If Jupiter fails, uses token mapping
3. **Stale Cache** - If all APIs fail, uses last known price

### Circuit Breaker

Protects against API rate limits:

- Opens after 3 failures
- Stays open for 60 seconds
- Tests recovery automatically
- Prevents cascading failures

### Caching

All layers implement caching:

- **Token Mapping**: Caches mint → token_id lookups
- **Jupiter**: Caches prices with configurable TTL
- **CoinGecko**: Reuses PriceFetcher cache
- **Metadata**: Caches decimals and supply

### Per-Token TTL

Configure different cache durations per token:

```toml
[[token_mapping]]
mint = "StableCoinMint..."
coingecko_id = "usdc"
cache_ttl_secs = 3600  # 1 hour for stablecoins

[[token_mapping]]
mint = "VolatileCoinMint..."
coingecko_id = "volatile-token"
cache_ttl_secs = 60  # 1 minute for volatile tokens
```

## Common Patterns

### Pattern 1: Price Fetching Only

```rust
// Just use the fallback provider
let fallback = FallbackPriceProvider::new(vec![
    Arc::new(JupiterPriceProvider::new(Duration::from_secs(300))),
]);

let price = fallback.fetch_price("mint_address").await?;
```

### Pattern 2: Metadata Only

```rust
// Just use the metadata provider
let metadata = TokenMetadataProvider::new(
    "https://api.mainnet-beta.solana.com".to_string(),
    Duration::from_secs(3600),
);

let decimals = metadata.get_decimals("mint_address").await?;
```

### Pattern 3: Full Integration

```rust
// Use both price and metadata
let fallback = setup_fallback_provider()?;
let metadata = TokenMetadataProvider::new(rpc_url, Duration::from_secs(3600));

// Get price
let price = fallback.fetch_price("mint").await?;

// Get metadata
let decimals = metadata.get_decimals("mint").await?;

// Calculate value
let value = amount_raw as f64 / 10_f64.powi(decimals as i32) * price;
println!("Value: ${:.2}", value);
```

## Configuration Options

### Token Mapping

```toml
[[token_mapping]]
mint = "Mint address (required)"
coingecko_id = "CoinGecko ID (required)"
cache_ttl_secs = 300  # Optional, per-token cache TTL
```

### Price Fetcher (for CoinGecko)

```toml
[price_fetcher]
cache_ttl_secs = 300  # Cache TTL for CoinGecko
```

### Code Configuration

```rust
// Jupiter cache TTL
JupiterPriceProvider::new(Duration::from_secs(300))

// Metadata cache TTL
TokenMetadataProvider::new(rpc_url, Duration::from_secs(3600))

// Circuit breaker is hardcoded:
// - Threshold: 3 failures
// - Timeout: 60 seconds
```

## Monitoring

### Check Provider Availability

```rust
// Check if Jupiter is available (circuit breaker not open)
if jupiter.is_available().await {
    println!("Jupiter is available");
}

// Check provider name
println!("Provider: {}", jupiter.name());
```

### Cache Statistics

```rust
// Token mapping cache size
println!("Mapping cache: {} entries", mapping.cache_size().await);

// Metadata cache size
println!("Metadata cache: {} entries", metadata.cache_size().await);
```

### Clear Caches

```rust
// Clear token mapping cache
mapping.clear_cache().await;

// Clear metadata cache
metadata.clear_cache().await;
```

## Error Handling

All operations return `Result<T, AppError>`:

```rust
match fallback.fetch_price("mint").await {
    Ok(price) => println!("Price: ${}", price),
    Err(e) => {
        eprintln!("Failed to fetch price: {}", e);
        // Handle error appropriately
    }
}
```

## Performance Tips

1. **Reuse Providers**: Create providers once, use many times
2. **Configure TTLs**: Balance freshness vs. API calls
3. **Prefetch Metadata**: Use `prefetch_metadata()` at startup
4. **Monitor Circuit Breakers**: Check `is_available()` for health

## Testing

Run Stage 4 tests:

```bash
# All Stage 4 integration tests
cargo test stage4_integration --test stage4_integration_tests

# Specific test category
cargo test token_mapping --lib
cargo test price_provider --lib
cargo test token_metadata --lib
```

## Troubleshooting

### "Circuit breaker is open"

**Problem**: Too many API failures  
**Solution**: Wait 60 seconds or check API status

### "No token mapping for mint"

**Problem**: Mint not in configuration  
**Solution**: Add mapping to `config.toml`

### "Failed to fetch price from all providers"

**Problem**: All APIs failed and no stale cache  
**Solution**: Check network, API status, and logs

### "Invalid mint account data length"

**Problem**: Mint address is not an SPL token  
**Solution**: Verify mint address is correct

## Next Steps

- Read `STAGE4_IMPLEMENTATION.md` for detailed architecture
- Check `SECURITY_SUMMARY_STAGE4.md` for security analysis
- Review integration tests for more examples
- Configure your token mappings in `config.toml`

## Support

For issues or questions:
1. Check the comprehensive documentation
2. Review the 21 integration test examples
3. Examine the implementation code
4. File an issue on GitHub

---

**Version**: Stage 4 Complete  
**Tests**: 250 passing (including 21 Stage 4 integration tests)  
**Status**: Production Ready ✅
