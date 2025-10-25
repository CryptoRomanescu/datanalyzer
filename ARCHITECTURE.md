# DataAnalyzer Architecture

Comprehensive architectural documentation for the DataAnalyzer system.

## Table of Contents

1. [System Overview](#system-overview)
2. [Component Architecture](#component-architecture)
3. [Data Flow](#data-flow)
4. [Design Patterns](#design-patterns)
5. [Data Models](#data-models)
6. [API Contracts](#api-contracts)
7. [Error Handling](#error-handling)
8. [Performance Considerations](#performance-considerations)
9. [Security Architecture](#security-architecture)
10. [Future Architecture](#future-architecture)

## System Overview

DataAnalyzer is a production-ready system for monitoring Solana decentralized exchange (DEX) pools in real-time, fetching price data from multiple sources, and persisting snapshots to CSV files.

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     External Systems                             │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────────┐ │
│  │  Solana RPC    │  │  Solana WS     │  │  Price APIs       │ │
│  │  (State Query) │  │  (Real-time)   │  │  (Jupiter/CG)     │ │
│  └───────┬────────┘  └───────┬────────┘  └────────┬───────────┘ │
└──────────┼────────────────────┼──────────────────────┼───────────┘
           │                    │                      │
           │                    │                      │
┌──────────┼────────────────────┼──────────────────────┼───────────┐
│          ▼                    ▼                      ▼           │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────────┐ │
│  │  Orchestrator  │  │   WebSocket    │  │  Price Provider    │ │
│  │   (RPC Calls)  │  │    Manager     │  │   Fallback Chain   │ │
│  └───────┬────────┘  └───────┬────────┘  └────────┬───────────┘ │
│          │                   │                     │             │
│  ┌───────┴────────┐  ┌───────┴────────┐  ┌────────┴───────────┐ │
│  │   DEX Decoder  │  │  Throttle      │  │  Circuit Breaker   │ │
│  │  (Raydium/     │  │  (Token        │  │  (Rate Limit       │ │
│  │   Pump.fun)    │  │   Bucket)      │  │   Protection)      │ │
│  └───────┬────────┘  └───────┬────────┘  └────────┬───────────┘ │
│          │                   │                     │             │
│  ┌───────┴───────────────────┴─────────────────────┴───────────┐ │
│  │                  Data Processing Layer                       │ │
│  │  • Token Metadata Provider  • Pool Snapshot Builder         │ │
│  │  • Liquidity Calculator     • CSV Row Formatter             │ │
│  └───────────────────────────┬─────────────────────────────────┘ │
│                              │                                   │
│  ┌───────────────────────────┴─────────────────────────────────┐ │
│  │                   Persistence Layer                          │ │
│  │  ┌────────────────┐  ┌─────────────────────────────────────┐│ │
│  │  │  CSV Writer    │  │      Observability                  ││ │
│  │  │  (Buffered,    │  │  • Metrics (Prometheus)             ││ │
│  │  │   Rotated)     │  │  • Health Checks (HTTP)             ││ │
│  │  │                │  │  • Logging (tracing)                ││ │
│  │  └────────────────┘  └─────────────────────────────────────┘│ │
│  └───────────────────────────────────────────────────────────────┘ │
│                     DataAnalyzer Application                      │
└───────────────────────────────────────────────────────────────────┘
```

### Deployment View

```
┌──────────────────────────────────────────────────┐
│              Operating System                    │
│  ┌────────────────────────────────────────────┐  │
│  │         DataAnalyzer Process               │  │
│  │  ┌──────────────┐  ┌──────────────┐       │  │
│  │  │ Main Thread  │  │ Tokio Runtime│       │  │
│  │  │              │  │ (Async Tasks)│       │  │
│  │  └──────────────┘  └──────────────┘       │  │
│  │                                            │  │
│  │  HTTP Servers:                             │  │
│  │  • :8080 /health (Health Check)            │  │
│  │  • :9090 /metrics (Prometheus)             │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  File System:                                    │
│  • /opt/datanalyzer/config.toml (Config)        │
│  • /opt/datanalyzer/data/*.csv (Data)           │
│  • /var/log/datanalyzer/*.log (Logs)            │
└──────────────────────────────────────────────────┘
```

## Component Architecture

### 1. WebSocket Manager

**Purpose**: Maintain WebSocket connection to Solana and manage pool subscriptions

**Responsibilities**:
- Establish and maintain WebSocket connection
- Subscribe to pool account updates
- Handle connection failures with exponential backoff
- Resubscribe to all pools after reconnection
- Track problematic pools and retry
- Throttle notifications per pool (token bucket)

**Key Types**:
```rust
pub struct WebSocketManager {
    ws_url: String,
    snapshot_interval_ms: u64,
    // Internal state...
}
```

**Dependencies**:
- `tokio` for async runtime
- `futures-util` for WebSocket
- `solana-client` for Pubkey handling

### 2. Reserve Orchestrator

**Purpose**: Orchestrate RPC calls to resolve pool reserves

**Responsibilities**:
- Fetch vault balances for Raydium pools via RPC
- Return direct reserves for Pump.fun pools
- Parse SPL Token account data
- Validate mint addresses
- Error handling and retries

**Key Types**:
```rust
pub struct ReserveOrchestrator {
    rpc_client: Arc<RpcClient>,
}

pub enum ReserveInfo {
    Direct { base: u64, quote: u64 },
    RequiresVaults(VaultInfo),
}
```

**Design Pattern**: Strategy Pattern (polymorphic reserve resolution)

### 3. DEX Decoders

**Purpose**: Parse on-chain account data for different DEX types

#### RaydiumDecoder

```rust
pub struct RaydiumDecoder;

impl DexDecoder for RaydiumDecoder {
    fn decode_reserve_info(&self, data: &[u8]) -> Result<ReserveInfo>;
    fn get_vault_info(&self, data: &[u8]) -> Result<VaultInfo>;
}
```

**Approach**:
- Zero-copy deserialization with `bytemuck`
- Pod-safe structures (repr(C), repr(packed))
- Field offset validation
- No unsafe code

#### PumpfunDecoder

```rust
pub struct PumpfunDecoder;

impl DexDecoder for PumpfunDecoder {
    fn decode_reserve_info(&self, data: &[u8]) -> Result<ReserveInfo>;
}
```

**Approach**:
- Direct byte reading at known offsets
- Little-endian u64 parsing
- Simpler than Raydium (no vaults)

### 4. Price Provider System

**Architecture**: Fallback Chain Pattern

```
┌──────────────┐
│   Consumer   │
└──────┬───────┘
       │
       ▼
┌──────────────────────┐
│ FallbackPriceProvider│
└──────┬───────────────┘
       │
       ├─► 1. JupiterPriceProvider ──► Circuit Breaker ──► Jupiter API
       │
       ├─► 2. CoinGeckoPriceProvider ─► PriceFetcher ──► CoinGecko API
       │                                    │
       │                                    ▼
       │                              TokenMappingService
       │
       └─► 3. Stale Cache (last resort)
```

**Components**:

- **JupiterPriceProvider**: Primary source, direct mint queries, circuit breaker
- **CoinGeckoPriceProvider**: Secondary source, requires token mapping
- **FallbackPriceProvider**: Orchestrates fallback logic
- **CircuitBreaker**: Protects against rate limits (3 failures → 60s timeout)

### 5. Token Mapping Service

**Purpose**: Map Solana mint addresses to CoinGecko token IDs

**Architecture**:
```
┌────────────────────────┐
│  TokenMappingService   │
│  ┌──────────────────┐  │
│  │  Cache (RwLock)  │  │
│  └────────┬─────────┘  │
│           ▼            │
│  ┌──────────────────┐  │
│  │    Providers     │  │
│  │  ┌────────────┐  │  │
│  │  │  Static    │  │  │  (TOML config)
│  │  │  Mapping   │  │  │
│  │  └────────────┘  │  │
│  │  ┌────────────┐  │  │
│  │  │  Dynamic   │  │  │  (Future: HTTP API)
│  │  │  Provider  │  │  │
│  │  └────────────┘  │  │
│  └──────────────────┘  │
└────────────────────────┘
```

**Extensibility**: Provider trait allows custom implementations

### 6. Token Metadata Provider

**Purpose**: Fetch token decimals and supply from on-chain data

**Approach**:
- RPC `getAccountInfo` calls
- SPL Token Mint parsing
- TTL-based caching
- Prefetch support for batch operations

**Cache Structure**:
```rust
struct CachedMetadata {
    metadata: TokenMetadata,
    cached_at: Instant,
}
```

### 7. CSV Writer

**Purpose**: Persist pool snapshots to disk

**Features**:
- Buffered I/O for performance
- Automatic header writing
- File rotation by size/age
- Batching with configurable flush
- Thread-safe (Arc<RwLock>)

**Configuration**:
```rust
pub struct CsvWriterConfig {
    append: bool,
    max_file_size: u64,      // Rotation trigger
    max_file_age: u64,       // Age-based rotation
    batch_size: usize,       // Flush after N records
    batch_time_ms: u64,      // Time-based flush
}
```

### 8. Observability

#### Metrics (Prometheus)

**Endpoint**: `http://localhost:9090/metrics`

**Metrics**:
- Gauges: subscription count, cache size
- Counters: notifications, errors, requests
- Histograms: latency distributions (future)

#### Health Checks

**Endpoint**: `http://localhost:8080/health`

**Checks**:
- WebSocket connection status
- RPC availability
- Recent activity timestamp

#### Logging

**Framework**: `tracing` + `tracing-subscriber`

**Levels**:
- ERROR: Critical failures
- WARN: Degraded performance
- INFO: Normal events (connections, subscriptions)
- DEBUG: Detailed operation (cache hits, RPC calls)
- TRACE: Very verbose (development only)

## Data Flow

### End-to-End Flow: Pool Update

```
1. Solana → WebSocket Notification
   │
   ├─ [Account Address]
   └─ [Account Data (binary)]
      │
      ▼
2. WebSocketManager
   │
   ├─ Throttle Check (token bucket)
   └─ If allowed:
      │
      ▼
3. DEX Decoder
   │
   ├─ Raydium: decode_reserve_info() → RequiresVaults(VaultInfo)
   └─ Pump.fun: decode_reserve_info() → Direct { base, quote }
      │
      ▼
4. Reserve Orchestrator (if RequiresVaults)
   │
   ├─ RPC: get_account(coin_vault)
   ├─ RPC: get_account(pc_vault)
   ├─ Parse SPL Token accounts
   └─ Return (base_reserve, quote_reserve)
      │
      ▼
5. Token Metadata Provider
   │
   ├─ Cache lookup
   └─ If miss: RPC get_account(mint) → parse decimals
      │
      ▼
6. Price Provider Fallback Chain
   │
   ├─ Try Jupiter API
   ├─ If fail: Try CoinGecko
   └─ If fail: Use stale cache
      │
      ▼
7. Liquidity Calculator
   │
   └─ liquidity_usd = (quote_reserve / 10^decimals) * 2
      │
      ▼
8. PoolSnapshot Builder
   │
   └─ Aggregate all data into snapshot
      │
      ▼
9. CSV Writer
   │
   ├─ Convert to CSV row
   ├─ Buffer write
   └─ Periodic flush
      │
      ▼
10. Metrics Update
    │
    └─ Increment counters, update gauges
```

### Circuit Breaker State Machine

```
     ┌─────────┐
     │ CLOSED  │ ◄─────────┐
     └────┬────┘           │
          │                │
     [3 failures]     [success in
          │            half-open]
          ▼                │
     ┌─────────┐           │
     │  OPEN   │           │
     └────┬────┘           │
          │                │
     [60s timeout]         │
          │                │
          ▼                │
     ┌──────────┐          │
     │ HALF-    │          │
     │  OPEN    │ ─────────┘
     └──────────┘
      [failure: back to OPEN]
```

## Design Patterns

### 1. Strategy Pattern: Reserve Resolution

**Problem**: Different DEX types have different ways of storing reserves

**Solution**: Enum-based polymorphism

```rust
pub enum ReserveInfo {
    Direct { base: u64, quote: u64 },
    RequiresVaults(VaultInfo),
}
```

**Benefits**:
- Type-safe
- Pattern matching enforces handling
- No vtable overhead

### 2. Fallback Chain: Price Providers

**Problem**: Need resilience against API failures

**Solution**: Chain of Responsibility pattern

```rust
for provider in &self.providers {
    match provider.fetch_price(mint).await {
        Ok(price) => {
            self.update_stale_cache(mint, price);
            return Ok(price);
        }
        Err(_) => continue,
    }
}
// Last resort: stale cache
self.get_from_stale_cache(mint)
```

**Benefits**:
- Automatic failover
- No single point of failure
- Stale data better than no data

### 3. Circuit Breaker: Rate Limit Protection

**Problem**: API rate limits cause cascading failures

**Solution**: Circuit Breaker pattern

```rust
if !circuit_breaker.can_request() {
    return Err(AppError::ServiceUnavailable);
}

match make_request().await {
    Ok(result) => {
        circuit_breaker.record_success();
        Ok(result)
    }
    Err(e) => {
        circuit_breaker.record_failure();
        Err(e)
    }
}
```

**Benefits**:
- Prevents hammering failing services
- Automatic recovery testing
- Graceful degradation

### 4. Token Bucket: Rate Limiting

**Problem**: Need to limit notification processing per pool

**Solution**: Token Bucket algorithm

```rust
let now = Instant::now();
let elapsed = (now - bucket.last_update).as_secs_f64();

// Refill bucket
bucket.tokens = (bucket.tokens + elapsed * rate).min(capacity);
bucket.last_update = now;

// Consume token
if bucket.tokens >= 1.0 {
    bucket.tokens -= 1.0;
    true  // Allow
} else {
    false  // Throttle
}
```

**Benefits**:
- Smooth rate limiting
- Burst capacity
- Per-pool fairness

### 5. Builder Pattern: Configuration

**Problem**: Complex configuration with many optional parameters

**Solution**: Builder pattern

```rust
let config = CsvWriterConfig::builder()
    .append(true)
    .batch_size(500)
    .max_file_size(100_000_000)
    .build();
```

**Benefits**:
- Fluent API
- Optional parameters
- Type-safe defaults

## Data Models

### Core Types

```rust
// Pool identification
pub enum DexType {
    PumpFun,
    Raydium,
}

// Pool state snapshot
pub struct PoolSnapshot {
    pub pool_address: String,
    pub token_mint: String,
    pub dex_type: DexType,
    pub reserve_base: u64,
    pub reserve_quote: u64,
    pub timestamp: i64,
    pub price: f64,
    pub liquidity_usd: Option<f64>,
}

// Reserve information
pub enum ReserveInfo {
    Direct {
        base: u64,
        quote: u64,
    },
    RequiresVaults(VaultInfo),
}

pub struct VaultInfo {
    pub coin_vault: Pubkey,
    pub pc_vault: Pubkey,
}

// Token metadata
pub struct TokenMetadata {
    pub mint: String,
    pub decimals: u8,
    pub supply: Option<u64>,
}
```

### Raydium On-Chain Structures

```rust
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct AmmInfo {
    pub status: u64,
    pub nonce: u64,
    pub order_num: u64,
    pub depth: u64,
    pub coin_decimals: u64,
    pub pc_decimals: u64,
    pub state: u64,
    pub reset_flag: u64,
    pub min_size: u64,
    pub vol_max_cut_ratio: u64,
    pub amount_wave_ratio: u64,
    pub coin_lot_size: u64,
    pub pc_lot_size: u64,
    pub min_price_multiplier: u64,
    pub max_price_multiplier: u64,
    pub system_decimals_value: u64,
    // Fee structure
    pub fees: Fees,
    // State data
    pub state_data: StateData,
    // Pubkeys (32 bytes each)
    pub coin_vault: Pubkey,
    pub pc_vault: Pubkey,
    // ... more fields
}
```

## API Contracts

### DexDecoder Trait

```rust
pub trait DexDecoder: Send + Sync {
    /// Decode reserve information from account data
    fn decode_reserve_info(&self, data: &[u8]) -> Result<ReserveInfo, AppError>;
}
```

**Contract**:
- Must be thread-safe (Send + Sync)
- Must validate input data
- Must return meaningful errors
- Should not panic

### PriceProvider Trait

```rust
#[async_trait::async_trait]
pub trait PriceProvider: Send + Sync {
    /// Fetch current price for a token
    async fn fetch_price(&self, mint: &str) -> Result<f64, AppError>;
    
    /// Provider name for logging
    fn name(&self) -> &str;
    
    /// Check if provider is available
    async fn is_available(&self) -> bool;
}
```

**Contract**:
- Async-safe
- Thread-safe
- Must handle rate limits gracefully
- Should use circuit breaker

### TokenMappingProvider Trait

```rust
#[async_trait::async_trait]
pub trait TokenMappingProvider: Send + Sync {
    /// Get CoinGecko token ID for a mint
    async fn get_token_id(&self, mint: &str) -> Result<Option<String>, AppError>;
    
    /// Get cache TTL for a specific mint
    async fn get_cache_ttl(&self, mint: &str) -> Option<u64>;
}
```

**Contract**:
- Return `Ok(None)` for unknown mints (not an error)
- Must be thread-safe
- Should implement caching internally

## Error Handling

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("RPC error: {0}")]
    RpcError(String),
    
    #[error("Price fetch error: {0}")]
    PriceError(String),
    
    #[error("Decoding error: {0}")]
    DecodingError(String),
    
    #[error("I/O error: {0}")]
    IoError(String),
    
    #[error("Service unavailable")]
    ServiceUnavailable,
}
```

### Error Handling Strategy

1. **Validation Errors**: Return immediately, don't retry
2. **Network Errors**: Retry with exponential backoff
3. **Rate Limit Errors**: Activate circuit breaker
4. **Transient Errors**: Use fallback (stale cache)
5. **Fatal Errors**: Log and restart component

### Error Propagation

```rust
// Good: Use ? operator for propagation
let data = fetch_data().await?;

// Good: Add context to errors
.map_err(|e| AppError::RpcError(format!("Failed to fetch: {}", e)))?;

// Good: Handle recoverable errors
match risky_operation() {
    Ok(result) => result,
    Err(e) => {
        log::warn!("Operation failed, using fallback: {}", e);
        fallback_value
    }
}
```

## Performance Considerations

### Bottlenecks

1. **RPC Calls**: 100-500ms latency per call
   - **Mitigation**: Caching, batching, circuit breaker
   
2. **CSV I/O**: Disk write latency
   - **Mitigation**: Buffering, batching, async flush
   
3. **JSON Parsing**: CPU-intensive for large responses
   - **Mitigation**: Streaming parser (future), smaller payloads

4. **Lock Contention**: Multiple threads accessing shared state
   - **Mitigation**: RwLock (multiple readers), fine-grained locking

### Optimization Techniques

1. **Zero-Copy Deserialization**: `bytemuck` for on-chain data
2. **Lazy Evaluation**: Only fetch data when needed
3. **Caching**: TTL-based caches for all external data
4. **Batching**: Group CSV writes, RPC calls
5. **Async I/O**: Non-blocking operations with tokio

### Resource Limits

| Resource | Limit | Reason |
|----------|-------|--------|
| Memory | <500MB | Prevent OOM |
| CPU | <50% | Leave headroom |
| Disk | <10GB/day | CSV rotation |
| RPC Rate | <100/min | Avoid 429 |
| WebSocket | 1 connection | Protocol limit |

## Security Architecture

### Threat Model

**In Scope**:
- Malicious on-chain data
- Rate limiting / DoS
- Data corruption
- Dependency vulnerabilities

**Out of Scope**:
- Physical security
- Network-level attacks
- Browser-based attacks (no web UI)

### Security Measures

1. **Input Validation**:
   - All on-chain data validated before use
   - Pubkey format validation
   - Range checks on numeric values

2. **Memory Safety**:
   - No unsafe code in application layer
   - Pod-safe structures for zero-copy
   - Borrow checker prevents data races

3. **Rate Limiting**:
   - Circuit breaker for external APIs
   - Token bucket for notification processing
   - Prevents resource exhaustion

4. **Error Handling**:
   - No panics in production code
   - Graceful degradation
   - Detailed error logging

5. **Dependency Auditing**:
   - cargo-deny checks
   - Regular updates
   - Security advisories documented

### Data Privacy

- **No Personal Data**: Only public blockchain data
- **No Authentication**: Public APIs only
- **No Credentials**: No secrets stored
- **Logging**: No sensitive data in logs

## Future Architecture

### Planned Enhancements

1. **Horizontal Scaling**:
   - Distribute pools across multiple instances
   - Shared state via Redis
   - Load balancer for health checks

2. **Database Integration**:
   - PostgreSQL for historical data
   - TimescaleDB for time-series
   - GraphQL API for queries

3. **Advanced Analytics**:
   - Real-time aggregations
   - Trend detection
   - Anomaly detection

4. **WebSocket API**:
   - Real-time updates for clients
   - Subscription management
   - Authentication/authorization

5. **Dynamic Configuration**:
   - Hot reload of config
   - Dynamic pool addition/removal
   - Remote configuration management

### Scalability Roadmap

```
Current:
┌──────────────┐
│  Single Node │  → 100s of pools
└──────────────┘

Phase 1 (Q2 2025):
┌──────────────┐  ┌──────────────┐
│  Node 1      │  │  Node 2      │  → 1000s of pools
│  (Pools 1-N) │  │  (Pools N+1..│
└──────────────┘  └──────────────┘
        │                │
        └────────┬───────┘
                 ▼
         ┌──────────────┐
         │  PostgreSQL  │
         └──────────────┘

Phase 2 (Q4 2025):
         ┌──────────────┐
         │ Load Balancer│
         └──────┬───────┘
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
┌────────┐  ┌────────┐  ┌────────┐
│ Node 1 │  │ Node 2 │  │ Node N │  → 10,000s of pools
└────────┘  └────────┘  └────────┘
    │           │           │
    └───────────┼───────────┘
                ▼
         ┌──────────────┐
         │   Database   │
         │   Cluster    │
         └──────────────┘
```

---

**Document Version**: 1.0
**Last Updated**: 2025-10-25
**Authors**: DataAnalyzer Team
