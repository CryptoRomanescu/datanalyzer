# PumpSwap Pool Discovery Implementation Summary

## Overview

This implementation adds automatic discovery and subscription of PumpSwap liquidity pools without requiring manual configuration. Users can enable discovery in the config file, and the system will automatically find and subscribe to all relevant pools.

## Features Implemented

### 1. PumpSwap DEX Support
- **New DEX Type**: Added `DexType::PumpSwap` to support PumpSwap AMM pools
- **Account Decoder**: Created `PumpSwapDecoder` to parse 324-byte pool accounts
- **Extract Mints**: Decoder can extract both base and quote mint addresses from pool data
- **Extract Reserves**: Decoder extracts base and quote reserve amounts

**Files Modified**:
- `src/models.rs`: Added `PumpSwap` variant to `DexType` enum
- `src/dex/pumpswap.rs`: New decoder implementation
- `src/dex/mod.rs`: Integrated PumpSwap decoder into factory

### 2. Discovery Module
- **Backfill Discovery**: Queries all existing pools using `getProgramAccounts`
- **Smart Filtering**: Filters pools by:
  - Quote token allowlist (USDC, USDT, SOL by default)
  - Minimum quote liquidity threshold
  - Maximum pool count limit
- **Duplicate Tracking**: Tracks discovered pools to avoid re-subscription
- **Account Size Filter**: Only fetches accounts with correct size (324 bytes)

**Files Created**:
- `src/discovery.rs`: Complete discovery implementation

### 3. Configuration
- **Discovery Section**: New `[discovery]` config section with:
  - `enable_pumpswap`: Enable/disable discovery (default: false)
  - `pumpswap_program_id`: Program ID (default: pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA)
  - `quote_allowlist`: List of allowed quote mints
  - `min_quote_liquidity`: Minimum liquidity threshold (default: 1000.0)
  - `max_pools`: Maximum pools to track (default: 2000)
  - `rescan_interval_secs`: Future rescan interval (default: 300)

**Files Modified**:
- `src/config.rs`: Added `DiscoveryConfig` struct and integration
- `config.example.toml`: Added discovery section with examples

### 4. Dynamic Pool Registration
- **Runtime Registration**: Orchestrator can register pools after startup
- **Thread-Safe**: Pool management uses `Arc<Mutex<>>` for concurrent access
- **Automatic Subscription**: Discovered pools are automatically subscribed via WebSocket

**Files Modified**:
- `src/orchestrator.rs`: Added `register_pool()` method and thread-safe pool tracking
- `src/main.rs`: Integrated discovery backfill on startup

### 5. Testing
- **8 New Integration Tests**: Cover all aspects of discovery
- **232 Total Tests Passing**: All existing tests still pass
- **Test Coverage**:
  - Configuration defaults
  - Discovery creation and validation
  - Pool filtering logic
  - Discovery tracking
  - PumpSwap decoder integration

**Files Created**:
- `tests/discovery_integration_tests.rs`: Complete test suite

### 6. Documentation
- **README Updates**: Added discovery section with detailed explanations
- **Config Examples**: Updated config.example.toml with discovery settings
- **Architecture Diagrams**: Updated to mention PumpSwap support

**Files Modified**:
- `README.md`: Added discovery documentation
- `config.example.toml`: Added discovery examples

## Usage

### Basic Configuration

```toml
[discovery]
enable_pumpswap = true
pumpswap_program_id = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"
quote_allowlist = [
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", # USDC
  "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", # USDT
  "So11111111111111111111111111111111111111112",  # SOL
]
min_quote_liquidity = 1000.0
max_pools = 2000
```

### Startup Behavior

1. Service starts and loads configuration
2. If `enable_pumpswap = true`:
   - Queries all PumpSwap program accounts
   - Filters by size (324 bytes)
   - Extracts quote mint from each pool
   - Filters by quote_allowlist
   - Checks minimum liquidity threshold
   - Registers filtered pools with orchestrator
   - Subscribes to pool updates via WebSocket
3. Continues monitoring all pools (manual + discovered)

### Example Output

```
INFO: Starting PumpSwap pool discovery...
INFO: Fetching PumpSwap program accounts...
INFO: Found 5432 PumpSwap accounts, filtering...
INFO: Discovered PumpSwap pool: 7abc...def
INFO: Discovered PumpSwap pool: 9xyz...123
INFO: Backfill complete: discovered 847 new PumpSwap pools
INFO: Successfully subscribed to 847 discovered pools
INFO: Subscribed 850 pools. Output dir: ./snapshots. Press Ctrl+C to stop.
```

## Technical Details

### PumpSwap Account Layout

```
Offset  | Size | Field
--------|------|------------------
0x00    | 8    | discriminator
0x08    | 32   | base_mint
0x28    | 32   | quote_mint
0x48    | 8    | base_reserve
0x50    | 8    | quote_reserve
...     | ...  | (other fields)
Total: 324 bytes
```

### Discovery Flow

```
1. getProgramAccounts(pumpswap_program_id)
   ↓
2. Filter by account size (324 bytes)
   ↓
3. Extract quote_mint from each pool
   ↓
4. Filter by quote_allowlist
   ↓
5. Decode reserves and check min_liquidity
   ↓
6. Register with orchestrator
   ↓
7. Subscribe via WebSocket
```

### Thread Safety

- Pool types and mints stored in `Arc<Mutex<HashMap<>>>`
- Concurrent access from discovery and orchestrator
- No race conditions on pool registration

## Dependencies Added

- `solana-account-decoder = "1.18"`: For RPC account filtering and encoding

## Performance Considerations

- **Backfill Time**: Depends on number of pools (typically 5-30 seconds for thousands of pools)
- **Memory Usage**: ~100 bytes per discovered pool
- **RPC Load**: One `getProgramAccounts` call on startup
- **WebSocket**: One subscription per discovered pool

## Future Enhancements (Not Implemented)

1. **Live programSubscribe**: Real-time detection of new pools without restart
   - Would use WebSocket `programSubscribe` to monitor pool creation events
   - Automatically add new pools as they are created
   
2. **Periodic Rescans**: Scheduled backfill to catch any missed pools
   - Uses `rescan_interval_secs` config parameter
   
3. **Pool Health Monitoring**: Detect and unsubscribe from inactive pools
   - Track last update time
   - Remove pools with no activity

4. **Raydium Discovery**: Similar implementation for Raydium pools
   - Would reuse most of the discovery infrastructure

## Testing

Run all tests:
```bash
cargo test
```

Run discovery tests only:
```bash
cargo test --test discovery_integration_tests
```

Run with logging:
```bash
RUST_LOG=info cargo run --release
```

## Validation

✅ All 232 tests passing (224 existing + 8 new)
✅ Code compiles without warnings
✅ Integration with existing orchestrator works
✅ Discovery can be enabled/disabled via config
✅ Filters work correctly (tested)
✅ Thread-safe pool registration
✅ Documentation updated

## Security Considerations

- **Input Validation**: All mint addresses validated before use
- **Size Checking**: Account size validated to prevent buffer overflows
- **Error Handling**: Graceful handling of invalid accounts
- **Rate Limiting**: Uses existing RPC rate limiting
- **No Secrets**: All configuration is public (program IDs, mint addresses)

## Breaking Changes

None. The implementation is fully backward compatible:
- Discovery is disabled by default
- Existing manual pool configuration still works
- No changes to existing APIs or behaviors

## Migration Guide

To enable discovery in an existing deployment:

1. Update `config.toml`:
   ```toml
   [discovery]
   enable_pumpswap = true
   ```

2. Restart the service

3. Verify logs show discovery working:
   ```
   grep "Discovery" logs.txt
   ```

That's it! No code changes needed.
