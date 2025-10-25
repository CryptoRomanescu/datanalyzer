# Changelog

All notable changes to the DataAnalyzer project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-10-25

### Added - Stage 5: Performance Testing, Hardening, and Release

#### Performance & Testing
- **Performance Test Suite**: 8 comprehensive performance tests validating:
  - High-frequency data processing (10,000+ snapshots/sec)
  - CSV persistence under load (300-500 writes/sec)
  - Concurrent access with 20 parallel tasks
  - Memory stability over 5,000+ iterations
  - Data structure scalability
  - Combined load scenarios
  
#### Dependency Management
- **cargo-deny Integration**: Automated dependency auditing
  - Security advisory checking
  - License compliance verification
  - Dependency source validation
  - Documented exceptions for known issues

#### Security
- **Security Hardening**:
  - Updated prometheus 0.13 → 0.14 (fixes protobuf vulnerability RUSTSEC-2024-0437)
  - Documented all transitive dependency advisories with risk assessment
  - Zero critical vulnerabilities in direct dependencies
  
#### Documentation
- **Comprehensive Documentation**:
  - Complete README with quick start, architecture, and API reference
  - CHANGELOG for version tracking
  - RUNBOOK with operational procedures
  - ARCHITECTURE documentation with diagrams
  - cargo-deny configuration with security notes

#### Licensing
- **MIT License**: Project now properly licensed under MIT

### Added - Stage 4: Token Mapping & Price Fallback Chain

#### Token Mapping System
- **StaticTokenMapping**: TOML-based mint → CoinGecko ID mapping
- **TokenMappingService**: Service layer with caching and per-token TTL
- **TokenMappingProvider**: Extensible trait for custom mapping providers

#### Price Provider Fallback Chain
- **JupiterPriceProvider**: Primary price source with circuit breaker
- **CoinGeckoPriceProvider**: Secondary source using existing PriceFetcher
- **FallbackPriceProvider**: Orchestrates Jupiter → CoinGecko → Stale Cache

#### Circuit Breaker
- **Automatic Rate Limit Protection**:
  - 3-failure threshold
  - 60-second timeout
  - Half-open recovery testing
  - Prevents cascading failures

#### Token Metadata
- **TokenMetadataProvider**: RPC-based metadata fetching
  - Decimal precision
  - Token supply
  - TTL-based caching
  - Bulk prefetch support

#### Testing
- 21 integration tests covering:
  - Token mapping workflows
  - Circuit breaker lifecycle
  - Price fallback scenarios
  - Metadata provider functionality
  - Edge cases and error handling

### Added - Stage 3: Persistence & Observability

#### CSV Export
- **CsvWriter**: Buffered CSV writer with rotation
  - Automatic file rotation by size/age
  - Batching for performance
  - Header management
  - Append mode support

#### Liquidity Calculation
- **On-chain Liquidity**: SOL/USDC pool liquidity calculation
  - Raydium AMM integration
  - Quote reserve decimals handling
  - USD valuation
  - Optional liquidity field in PoolSnapshot

#### Observability
- **Prometheus Metrics**: 15+ metrics for monitoring
  - WebSocket subscriptions and notifications
  - Price fetcher performance
  - Reconnection tracking
  - Pool-specific stats

- **Health Checks**: HTTP endpoint for system health
  - WebSocket connection status
  - RPC availability
  - Timestamp reporting
  - JSON response format

- **Structured Logging**: tracing-subscriber integration
  - Environment-based filtering
  - Hierarchical logging
  - Performance debugging

### Added - Stage 2: WebSocket & Price Integration

#### WebSocket Manager
- **Real-time Monitoring**: Solana account subscription system
  - Automatic reconnection with exponential backoff
  - Subscription tracking and resubscription
  - Problematic pool retry mechanism
  - Connection state management

#### Throttling
- **Token Bucket Algorithm**: Per-pool rate limiting
  - Configurable updates/second
  - Burst capacity
  - Skipped notification tracking
  - Prevents notification spam

#### Price Fetching
- **PriceFetcher**: CoinGecko API integration
  - TTL-based caching
  - Retry logic with exponential backoff
  - Batch price fetching
  - Performance metrics

#### Configuration
- **TOML-based Configuration**: Flexible system configuration
  - RPC and WebSocket URLs
  - Pool monitoring lists
  - Price fetcher settings
  - Health check and metrics ports
  - Throttling parameters

### Added - Stage 1: Orchestrator & DEX Decoders

#### Reserve Orchestrator
- **ReserveOrchestrator**: Async RPC orchestration
  - Automatic vault balance fetching for Raydium
  - Direct reserve support for Pump.fun
  - Error handling and retry logic
  - Clean separation of concerns

#### DEX Decoders
- **RaydiumDecoder**: Zero-copy AmmInfo deserialization
  - bytemuck-based safe parsing
  - Vault pubkey extraction
  - Reserve info abstraction
  - Field offset validation

- **PumpfunDecoder**: Pump.fun state parsing
  - Direct reserve extraction
  - No vault fetching needed
  - Compatible interface

#### Data Models
- **ReserveInfo Enum**: Polymorphic reserve representation
  - `Direct { base, quote }` - for Pump.fun
  - `RequiresVaults(VaultInfo)` - for Raydium
  - Uniform API for both types

- **VaultInfo**: Vault address encapsulation
  - Coin vault pubkey
  - PC vault pubkey
  - Mint validation

- **PoolSnapshot**: Comprehensive pool state
  - Pool and token addresses
  - DEX type identification
  - Reserve amounts
  - Price and liquidity
  - Timestamp

#### Safety & Testing
- **Zero Unsafe Code**: Safe Rust throughout
- **Comprehensive Tests**: 
  - 218 unit tests
  - Structure validation (size, alignment, Pod-safety)
  - Integration tests for all flows
  - Mock RPC simulations

### Dependencies

#### Core
- `solana-sdk = "1.18"` - Solana blockchain integration
- `solana-client = "1.18"` - RPC client
- `spl-token = "4.0"` - SPL Token parsing
- `tokio = "1.13"` - Async runtime

#### Data Handling
- `serde = "1.0"` - Serialization
- `csv = "1.3"` - CSV writing
- `bytemuck = "1.14"` - Zero-copy parsing
- `chrono = "0.4"` - Timestamp handling

#### Networking
- `reqwest = "0.11"` - HTTP client for price APIs
- `futures-util = "0.3"` - WebSocket utilities

#### Observability
- `prometheus = "0.14"` - Metrics (updated from 0.13)
- `axum = "0.6"` - HTTP server for health/metrics
- `tracing = "0.1"` - Structured logging
- `tracing-subscriber = "0.3"` - Log subscriber

#### Configuration
- `toml = "0.8"` - Configuration parsing

#### Development
- `thiserror = "1.0"` - Error handling
- `log = "0.4"` - Logging facade
- `env_logger = "0.9"` - Environment logging
- `rand = "0.8"` - Testing utilities
- `async-trait = "0.1"` - Async trait support

### Changed
- **prometheus**: Updated from 0.13 to 0.14 to fix protobuf vulnerability
- **PoolSnapshot**: Added `liquidity_usd` optional field
- **CSV Format**: Added liquidity_usd column to output
- **License**: Added MIT license to Cargo.toml

### Fixed
- **RUSTSEC-2024-0437**: protobuf stack overflow vulnerability (via prometheus update)
- **Zero Reserve Validation**: Removed validation to allow empty pool states
- **Build Warnings**: Resolved all compiler warnings

### Security
- **No Critical Vulnerabilities**: All direct dependencies clean
- **Documented Exceptions**: Transitive dependency advisories documented with risk assessment
- **Input Validation**: All external data validated
- **Thread Safety**: Proper Arc<RwLock> usage throughout
- **No Secrets**: Public APIs only, no credential storage

### Performance
- **10,000+ snapshots/sec**: Data structure processing
- **300-500 writes/sec**: Sustained CSV write throughput
- **<1ms cache hits**: Price and metadata caching
- **100-500ms API calls**: External API latency
- **Concurrent Access**: 20+ parallel tasks without deadlock
- **Memory Stable**: No leaks over 5,000+ iterations

## [Unreleased]

### Planned Features
- Dynamic token mapping via HTTP provider
- Configurable circuit breaker parameters
- Smart caching with adaptive TTL
- Multi-currency price support
- Provider plugin system
- Weighted fallback preferences

### Future Enhancements
- Historical data export
- GraphQL API
- Database storage option
- Real-time WebSocket API for clients
- Advanced analytics and aggregations

---

## Release Notes Format

Each release includes:
- **Added**: New features and capabilities
- **Changed**: Modifications to existing functionality
- **Deprecated**: Features planned for removal
- **Removed**: Deleted features
- **Fixed**: Bug fixes and corrections
- **Security**: Security-related changes

## Version Numbering

Following Semantic Versioning:
- **MAJOR**: Incompatible API changes
- **MINOR**: Backward-compatible functionality additions
- **PATCH**: Backward-compatible bug fixes

---

**For detailed implementation notes, see individual stage documentation files.**
