# DataAnalyzer

A high-performance, production-ready Solana pool data analyzer supporting Raydium and Pump.fun DEXes. Real-time WebSocket monitoring, price fetching from multiple sources, CSV export, and comprehensive observability.

## Features

### Core Functionality
- **Multi-DEX Support**: Raydium AMM and Pump.fun pool monitoring
- **WebSocket Monitoring**: Real-time pool state updates via Solana account subscriptions
- **Reserve Resolution**: Automatic handling of direct reserves (Pump.fun) and vault-based reserves (Raydium)
- **Price Fetching**: Fallback chain Jupiter → CoinGecko with circuit breaker protection
- **CSV Export**: Buffered CSV writing with automatic rotation and batching
- **Liquidity Calculation**: On-chain liquidity computation with USD valuation

### Advanced Features
- **Orchestrator Pattern**: Async RPC orchestration for vault balance fetching
- **Token Mapping**: Configurable mint → CoinGecko ID mapping with per-token TTL
- **Token Metadata**: RPC-based decimal and supply fetching with caching
- **Circuit Breaker**: Rate limit protection with automatic recovery
- **Throttling**: Configurable per-pool update rate limiting (token bucket algorithm)
- **Observability**: Prometheus metrics, health checks, and structured logging

## Quick Start

### Prerequisites

- Rust 1.70+ (for `std::io::IsTerminal`)
- Solana RPC endpoint
- Optional: CoinGecko API access

### Installation

```bash
git clone https://github.com/CryptoRomanescu/datanalyzer
cd datanalyzer
cargo build --release
```

### Configuration

Create a `config.toml` file:

```toml
# Solana RPC Configuration
rpc_url = "https://api.mainnet-beta.solana.com"
ws_url = "wss://api.mainnet-beta.solana.com"

# Monitoring Configuration
snapshot_interval_ms = 60000  # 1 minute

# CSV Output
csv_file_path = "./data/pools.csv"

# Price Fetcher
[price_fetcher]
cache_ttl_secs = 300  # 5 minutes

# Token Mapping (mint -> CoinGecko ID)
[[token_mapping]]
mint = "So11111111111111111111111111111111111111112"
coingecko_id = "solana"
cache_ttl_secs = 600

[[token_mapping]]
mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
coingecko_id = "usd-coin"
cache_ttl_secs = 600

# Pools to Monitor
[[pools]]
address = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"  # SOL/USDC Raydium
dex_type = "raydium"

[[pools]]
address = "7YttLkHDoNj9wyDur5pM1ejNaAvT9X4eqaYcHQqtj2G5"  # Example Pump.fun
dex_type = "pumpfun"

# Health Check
[healthcheck]
host = "127.0.0.1"
port = 8080

# Metrics
[metrics]
host = "127.0.0.1"
port = 9090

# Throttling (optional)
[throttle]
updates_per_second = 10.0
bucket_size = 10
```

### Running

```bash
# Run with default config (./config.toml)
cargo run --release

# Run with custom config
cargo run --release -- --config /path/to/config.toml

# Run examples
cargo run --example orchestrator_demo
cargo run --example decoder_registry_demo
cargo run --example liquidity_integration
cargo run --example observability_demo
```

## Architecture

### Component Overview

```
┌──────────────────────────────────────────────────────────────┐
│                     DataAnalyzer                              │
│                                                               │
│  ┌────────────────┐  ┌──────────────┐  ┌─────────────────┐  │
│  │   WebSocket    │  │  Orchestrator│  │  Price Provider │  │
│  │    Manager     │──│    (RPC)     │  │   Fallback      │  │
│  └────────────────┘  └──────────────┘  └─────────────────┘  │
│          │                  │                    │            │
│          ▼                  ▼                    ▼            │
│  ┌────────────────┐  ┌──────────────┐  ┌─────────────────┐  │
│  │   DEX Decoder  │  │   Token      │  │   CSV Writer    │  │
│  │   (Raydium/    │  │  Metadata    │  │  (Buffered)     │  │
│  │    Pump.fun)   │  │   Provider   │  │                 │  │
│  └────────────────┘  └──────────────┘  └─────────────────┘  │
│                                                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │          Observability Layer                           │  │
│  │  • Prometheus Metrics  • Health Checks  • Logging      │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
                             │
                             ▼
            ┌────────────────────────────────┐
            │      Solana Blockchain         │
            │  • RPC (state queries)         │
            │  • WebSocket (subscriptions)   │
            └────────────────────────────────┘
```

### Data Flow

1. **Subscription**: WebSocket Manager subscribes to pool accounts
2. **Notification**: Solana sends account updates via WebSocket
3. **Decoding**: DEX Decoder parses account data (Raydium AmmInfo or Pump.fun state)
4. **Resolution**: Orchestrator fetches vault balances via RPC (if needed)
5. **Enrichment**: Token Metadata Provider fetches decimals, Price Provider gets USD price
6. **Persistence**: CSV Writer appends snapshot to file
7. **Observability**: Metrics updated, health checks respond

### Key Patterns

- **Enum-based Polymorphism**: `ReserveInfo::Direct` vs `ReserveInfo::RequiresVaults`
- **Fallback Chain**: Jupiter (primary) → CoinGecko (secondary) → Stale Cache (last resort)
- **Circuit Breaker**: Automatic rate limit protection with half-open recovery
- **Token Bucket**: Per-pool throttling to prevent notification spam
- **Zero-Copy Deserialization**: `bytemuck` for safe, fast account parsing

## API Reference

### WebSocket Manager

```rust
use datanalyzer::{WebSocketManager, PoolConfig, DexType};

let manager = WebSocketManager::new(
    "wss://api.mainnet-beta.solana.com".to_string(),
    60000  // snapshot interval in ms
);

// Subscribe to a pool
manager.subscribe_pool(pool_address).await?;

// Listen for updates
manager.listen(
    |pool_address, account_data| {
        // Handle notification
    }
).await?;
```

### Reserve Orchestrator

```rust
use datanalyzer::{ReserveOrchestrator, RaydiumDecoder, DexDecoder};

let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
let decoder = RaydiumDecoder;

// Decode reserve info
let reserve_info = decoder.decode_reserve_info(&account_data)?;

// Resolve reserves (fetches vaults if needed)
let (base, quote) = orchestrator.resolve_reserves(&reserve_info)?;
```

### Price Provider

```rust
use datanalyzer::{JupiterPriceProvider, CoinGeckoPriceProvider, FallbackPriceProvider};

let jupiter = Arc::new(JupiterPriceProvider::new(Duration::from_secs(300)));
let coingecko = Arc::new(CoinGeckoPriceProvider::new(fetcher, mapping));

let providers: Vec<Arc<dyn PriceProvider>> = vec![jupiter, coingecko];
let fallback = FallbackPriceProvider::new(providers);

// Fetch price with automatic fallback
let price = fallback.fetch_price("So11111111111111111111111111111111111111112").await?;
```

### CSV Writer

```rust
use datanalyzer::{CsvWriter, PoolSnapshot};

let headers = &["pool_address", "token_mint", "dex_type", ...];
let mut writer = CsvWriter::new("./data/pools.csv", headers)?;

let snapshot = PoolSnapshot::new(...)?;
writer.write_record(&snapshot.to_csv_row())?;
writer.flush()?;
```

## CSV Output Format

The CSV file contains the following columns:

| Column | Type | Description |
|--------|------|-------------|
| `pool_address` | String | Solana address of the pool account |
| `token_mint` | String | Mint address of the token |
| `dex_type` | String | "Raydium" or "PumpFun" |
| `reserve_base` | u64 | Base token reserve (raw amount) |
| `reserve_quote` | u64 | Quote token reserve (raw amount) |
| `timestamp` | i64 | Unix timestamp of snapshot |
| `price` | f64 | Price in quote token terms |
| `liquidity_usd` | f64 | Total liquidity in USD (optional) |

Example row:
```csv
58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2,So11111111111111111111111111111111111111112,Raydium,1000000000,2000000000,1730000000,0.5,1500000.50
```

## Observability

### Health Check Endpoint

```bash
curl http://localhost:8080/health
```

Response:
```json
{
  "status": "healthy",
  "timestamp": 1730000000,
  "checks": {
    "websocket": "connected",
    "rpc": "available"
  }
}
```

### Prometheus Metrics

Metrics available at `http://localhost:9090/metrics`:

- `datanalyzer_websocket_subscriptions` - Current number of subscriptions
- `datanalyzer_websocket_notifications_total` - Total notifications received
- `datanalyzer_websocket_notifications_processed` - Notifications processed
- `datanalyzer_websocket_notifications_skipped` - Notifications skipped (throttling)
- `datanalyzer_websocket_reconnections_total` - Reconnection attempts
- `datanalyzer_price_fetcher_requests_total` - Total price fetch requests
- `datanalyzer_price_fetcher_cache_hits` - Cache hits
- `datanalyzer_price_fetcher_errors` - Fetch errors

### Logging

Structured logging with configurable levels:

```bash
# Set log level via environment variable
RUST_LOG=debug cargo run

# Log levels: error, warn, info, debug, trace
RUST_LOG=datanalyzer=debug,solana=warn cargo run
```

## Testing

```bash
# Run all tests
cargo test

# Run specific test suites
cargo test --lib                              # Library tests (218)
cargo test --test stage4_integration_tests   # Integration tests (21)
cargo test --test performance_tests          # Performance tests (8)

# Run with output
cargo test -- --nocapture

# Run performance tests (single-threaded for accurate measurements)
cargo test --test performance_tests -- --test-threads=1
```

### Test Coverage

- **247 Total Tests**
  - 218 Unit tests
  - 21 Integration tests  
  - 8 Performance tests

## Performance Characteristics

Based on performance test results:

- **Throughput**: 10,000+ snapshots/sec processing
- **CSV Writes**: 300-500 writes/sec sustained
- **Concurrent Access**: 20 parallel writers, no deadlocks
- **Memory**: Stable over 5,000+ snapshot iterations
- **Latency**: <1ms cache hits, 100-500ms RPC/API calls

### Optimization Tips

1. **Batching**: Configure `batch_size` in CSV writer (default: 100)
2. **Caching**: Set appropriate TTLs for price data (recommended: 300s)
3. **Throttling**: Enable per-pool throttling to reduce notification spam
4. **RPC**: Use rate-limited RPC endpoints to avoid 429 errors

## Security

### Dependency Audit

```bash
# Run cargo-deny checks
cargo deny check

# Advisories: PASS (documented exceptions)
# Licenses: PASS (MIT, Apache-2.0, BSD compatible)
# Bans: PASS (no banned crates)
# Sources: PASS (crates.io only)
```

See `deny.toml` for configuration and documented exceptions.

### Known Advisories

All security advisories from transitive dependencies (solana-sdk) are documented in `deny.toml` with risk assessment:

- `RUSTSEC-2025-0009/0010`: ring 0.16 issues (low impact, waiting for upstream)
- `RUSTSEC-2022-0093`: ed25519-dalek (no signing API exposure)
- Others: Unmaintained crates in dependency tree (no direct usage)

### Best Practices

- ✅ No unsafe code in application layer
- ✅ Input validation on all external data
- ✅ Rate limiting with circuit breaker
- ✅ Thread-safe concurrent access (Arc<RwLock>)
- ✅ No secrets in configuration (public APIs only)
- ✅ Regular dependency updates

## Troubleshooting

### Common Issues

**WebSocket disconnections**
- Check network connectivity
- Verify WebSocket URL is correct (`wss://`)
- Monitor reconnection attempts in logs
- Increase `max_reconnect_attempts` if needed

**RPC rate limits (429 errors)**
- Circuit breaker will activate automatically
- Reduce query frequency
- Use premium RPC endpoint
- Monitor circuit breaker state in logs

**High CPU usage**
- Enable throttling to reduce notification processing
- Increase `snapshot_interval_ms`
- Reduce number of monitored pools

**Memory growth**
- CSV writer flushes automatically (default: every 100 records)
- Check for long-running caches without TTL
- Monitor with performance tests

**Missing price data**
- Verify token is in mapping (for CoinGecko)
- Check if fallback chain is working
- Review circuit breaker status
- Jupiter may not have all tokens

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

### Development Guidelines

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix warnings
- Add tests for new features
- Update documentation
- Run `cargo deny check` to verify dependencies

## License

This project is licensed under the MIT License - see the `Cargo.toml` file for details.

## Acknowledgments

- Solana Foundation for the Solana SDK
- Raydium and Pump.fun teams for DEX implementations
- Rust community for excellent async ecosystem

## Links

- [Raydium Documentation](https://docs.raydium.io/)
- [Solana Documentation](https://docs.solana.com/)
- [Pump.fun](https://pump.fun/)

## Support

For issues, questions, or feature requests, please open an issue on GitHub.

---

**Built with ❤️ for the Solana ecosystem**
