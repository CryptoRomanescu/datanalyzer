# DataAnalyzer

A high-performance, production-ready Solana pool data analyzer supporting Raydium, Pump.fun, and PumpSwap DEXes. Real-time WebSocket monitoring, automatic pool discovery, price fetching from multiple sources, CSV export, and comprehensive observability.

## Features

### Core Functionality
- **Multi-DEX Support**: Raydium AMM, Pump.fun, and PumpSwap pool monitoring
- **Automatic Pool Discovery**: Automatically discover and subscribe to PumpSwap pools without manual configuration
- **WebSocket Monitoring**: Real-time pool state updates via Solana account subscriptions
- **Reserve Resolution**: Automatic handling of direct reserves (Pump.fun, PumpSwap) and vault-based reserves (Raydium)
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
rpc_ws_url = "wss://api.mainnet-beta.solana.com"

# Output directory for CSV files
output_dir = "./snapshots"

# Monitoring Configuration
snapshot_interval_ms = 5000  # 5 seconds

# CSV Writer Configuration
[csv]
# Enable append mode (default: true)
append = true

# Maximum file size in bytes before rotation (default: 500MB)
max_file_size = 500000000

# Maximum file age in seconds before rotation (0 = no rotation, default: 0)
max_file_age = 0

# Number of records to buffer before flushing (default: 500)
batch_size = 500

# Time in milliseconds before auto-flush (default: 3000 = 3 seconds)
batch_time_ms = 3000

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

# Pool Discovery (Optional - PumpSwap Auto-Discovery)
[discovery]
# Enable automatic PumpSwap pool discovery (default: false)
enable_pumpswap = true

# PumpSwap AMM program ID
pumpswap_program_id = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"

# Quote token allowlist - only pools with these quote mints will be subscribed
quote_allowlist = [
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", # USDC
  "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", # USDT
  "So11111111111111111111111111111111111111112",  # SOL
]

# Minimum quote liquidity to subscribe to a pool (in base units)
min_quote_liquidity = 1000.0

# Maximum number of pools to track
max_pools = 2000

# Interval between rescans in seconds (default: 300 = 5 minutes)
rescan_interval_secs = 300

# Pools to Monitor (Manual Configuration)
[[pools]]
pool_address = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"  # SOL/USDC Raydium
dex_type = "raydium"
token_mint = "So11111111111111111111111111111111111111112"

[[pools]]
pool_address = "7YttLkHDoNj9wyDur5pM1ejNaAvT9X4eqaYcHQqtj2G5"  # Example Pump.fun
dex_type = "pumpfun"
token_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
```

#### CSV Configuration Options

The `[csv]` section controls CSV file writing behavior with support for rotation, batching, and append mode:

- **`append`** (bool): When `true`, appends to existing CSV files instead of overwriting them. Default: `true`
- **`max_file_size`** (u64): Maximum file size in bytes before triggering rotation. When the file exceeds this size, it's renamed with a timestamp suffix and a new file is created. Set to `0` to disable size-based rotation. Default: `500000000` (500MB)
- **`max_file_age`** (u64): Maximum file age in seconds before triggering rotation. When the file is older than this duration, it's rotated. Set to `0` to disable age-based rotation. Default: `0` (disabled)
- **`batch_size`** (usize): Number of records to buffer before flushing to disk. Larger values improve performance but increase risk of data loss on crashes. Default: `500`
- **`batch_time_ms`** (u64): Maximum time in milliseconds between flushes. Even if `batch_size` isn't reached, data is flushed after this interval. Set to `0` to disable time-based flushing. Default: `3000` (3 seconds)

**File Rotation**: When rotation is triggered (by size or age), the current file is renamed to `{basename}_{timestamp}.csv` (e.g., `raydium_58oQChx4.csv` → `raydium_58oQChx4_1730000000.csv`) and a new file with the original name is created with headers.

#### Raydium Pool Address Resolver Configuration

The `[raydium_resolver]` section enables automatic validation and resolution of Raydium pool addresses:

- **`enabled`** (bool): Enable the Raydium pool address resolver. Default: `true`
- **`api_url`** (string): URL of the Raydium API endpoint. Default: `"https://api.raydium.io/v2/sdk/liquidity/mainnet.json"`
- **`timeout_secs`** (u64): Request timeout in seconds. Default: `10`

**What it does**:
1. **Validates Pool Addresses**: On startup, fetches the official list of Raydium AMM pools and validates that your configured pool addresses exist
2. **Address Resolution**: Can resolve marketId or LP mint addresses to the canonical AMM pool address (ammId)
3. **Program Verification**: Logs warnings if a configured address doesn't belong to the Raydium AMM v4 program
4. **Non-blocking**: If the API fetch fails, the service continues with your configured addresses and logs a warning

**Example usage**:
```toml
[raydium_resolver]
enabled = true
api_url = "https://api.raydium.io/v2/sdk/liquidity/mainnet.json"
timeout_secs = 10

[[pools]]
pool_address = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"  # SOL/USDC AMM
dex_type = "raydium"
token_mint = "So11111111111111111111111111111111111111112"
```

On startup, you'll see:
```
✓ Raydium resolver loaded 500 official pools
✓ Verified Raydium pool address: 58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2
First update for pool 58oQ...: owner=675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8, data_length=752 bytes
✓ Verified Raydium AMM v4 program for pool 58oQ...
```

**Why this matters**: Raydium has multiple program versions (AMM v4, CLMM v5). The resolver ensures you're using AMM v4 pools (752 bytes, program `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`), not CLMM pools which have different structure.

#### Pool Discovery Configuration

The `[discovery]` section enables automatic pool discovery for PumpSwap:

- **`enable_pumpswap`** (bool): Enable automatic PumpSwap pool discovery. Default: `false`
- **`pumpswap_program_id`** (string): The program ID of the PumpSwap AMM. Default: `"pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"`
- **`quote_allowlist`** (array): List of quote token mint addresses to filter pools by. Only pools with these quote tokens will be discovered and subscribed. Default: `[USDC, USDT, SOL]`
- **`min_quote_liquidity`** (f64): Minimum quote token liquidity (in base units) for a pool to be subscribed. Default: `1000.0`
- **`max_pools`** (usize): Maximum number of pools to discover and track. Default: `2000`
- **`rescan_interval_secs`** (u64): Interval between pool rescans in seconds (for future live discovery). Default: `300` (5 minutes)

**Discovery Process**:
1. **Backfill**: On startup, the service queries all PumpSwap program accounts and filters them by:
   - Account size (324 bytes for PumpSwap pools)
   - Quote mint in the allowlist
   - Minimum quote liquidity threshold
2. **Subscription**: Filtered pools are automatically registered with the orchestrator and subscribed via WebSocket
3. **No Manual Mapping**: You don't need to know token mints or pool addresses - discovery is fully automatic

**Example**: With `enable_pumpswap = true` and default settings, the service will:
- Find all PumpSwap pools with SOL, USDC, or USDT as quote token
- Filter out pools with less than 1000 base units of quote liquidity
- Subscribe to up to 2000 pools automatically
- Monitor them in real-time via WebSocket

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

The orchestrator uses `CsvWriter` with full configuration support for rotation, batching, and append mode:

```rust
use datanalyzer::csv_writer::{CsvWriter, CsvWriterConfig};

// Create configuration
let config = CsvWriterConfig::builder()
    .append(true)
    .max_file_size(500_000_000)  // 500MB
    .max_file_age(0)              // No age-based rotation
    .batch_size(500)              // Flush every 500 records
    .batch_time_ms(3000)          // Or every 3 seconds
    .build();

// Headers match PoolSnapshot::to_csv_row()
let headers = &[
    "pool_address", "token_mint", "dex_type", 
    "reserve_base", "reserve_quote", "timestamp", 
    "price", "liquidity_usd"
];

// Create writer with configuration
let mut writer = CsvWriter::with_config("./snapshots/pool.csv", headers, config)?;

// Write records - automatic batching and rotation
let snapshot = PoolSnapshot::new(...)?;
writer.write_record(&snapshot.to_csv_row())?;

// Flush is called automatically based on batch_size and batch_time_ms
// Or manually:
writer.flush()?;
```

**Features:**
- **Automatic Headers**: Headers are written once when creating or rotating files
- **Append Mode**: Continue writing to existing files without overwriting
- **File Rotation**: Automatically rotates files based on size or age
- **Batched Writes**: Configurable batching for optimal performance
- **Auto-flush**: Flushes on Drop to prevent data loss

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

- **260 Total Tests**
  - 218 Unit tests
  - 21 Integration tests (Stage 4)
  - 8 Performance tests
  - 11 Observability tests
  - 2 Documentation tests

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
