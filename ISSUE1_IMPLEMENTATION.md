# Issue 1: Core Runtime and CSV Pipeline - Implementation

This document describes the implementation of Issue 1 core components.

## Components Implemented

### 1. Configuration (`src/config.rs`)

- **AppConfig**: Main application configuration loaded from TOML
  - `AppConfig::load(path)` - Loads and validates configuration from file
  - Supports default fallbacks for optional sections
  - Full validation of required fields

- **CsvConfig**: CSV writer configuration with defaults
  - `append: bool` (default: true)
  - `max_file_size: u64` (default: 500MB)
  - `max_file_age: u64` (default: 0 = disabled)
  - `batch_size: usize` (default: 500)
  - `batch_time_ms: u64` (default: 3000ms)
  - `to_csv_writer_config()` - Converts to CsvWriterConfig

### 2. Models (`src/models.rs`)

- **DexType**: Enumeration of supported DEX types
  - `FromStr` implementation supporting case-insensitive parsing
  - Accepts: "raydium", "pumpfun"/"pump_fun", "pumpswap"/"pump_swap"
  - `Display` returns: "Raydium", "PumpFun", "PumpSwap"
  - `get_account_size()` returns correct sizes:
    - Raydium: 752 bytes
    - PumpFun: 256 bytes
    - PumpSwap: 324 bytes

- **PoolSnapshot**: Pool state snapshot
  - `new(...)` - Creates snapshot without liquidity
  - `with_liquidity(...)` - Creates snapshot with liquidity_usd
  - `to_csv_row()` - Returns 8-field Vec<String>:
    1. pool_address
    2. token_mint
    3. dex_type
    4. reserve_base
    5. reserve_quote
    6. timestamp
    7. price
    8. liquidity_usd (empty string when None, formatted when Some)

### 3. CSV Writer (`src/csv_writer.rs`)

- **CsvWriter**: Buffered CSV writer with advanced features
  - `new(path, headers)` - Create with default config
  - `with_config(path, headers, config)` - Create with custom config
  - Append mode support
  - Batching:
    - Flushes after `batch_size` records
    - Flushes after `batch_time_ms` milliseconds
  - File rotation:
    - By `max_file_size` (0 = disabled)
    - By `max_file_age` in seconds (0 = disabled)
  - `records_written()` - Counter that resets after flush
  - `flush()` - Manual flush
  - Auto-flush on Drop

- **CsvWriterConfig**: Configuration for CSV writer
  - Builder pattern support
  - Default values aligned with CsvConfig

### 4. CLI Binary (`src/main.rs`)

The main binary supports two modes:

#### Production Mode (default)
```bash
cargo run --release -- --config ./config.example.toml
```
Full production orchestrator with:
- WebSocket connections to Solana
- Pool discovery
- Real-time pool monitoring
- Runs indefinitely until Ctrl+C

#### Demo Mode (Issue 1)
```bash
cargo run --release -- --demo --config ./config.example.toml
```
Minimal demonstration mode that:
- Loads configuration from TOML
- Initializes CsvWriter with PoolSnapshot headers
- Writes 3 synthetic PoolSnapshot rows
- Exits cleanly
- Uses env_logger for logging

**Note**: Use `RUST_LOG=info` to see logging output:
```bash
RUST_LOG=info cargo run --release -- --demo --config ./config.example.toml
```

## Test Coverage

### CSV Integration Tests (`tests/csv_integration_test.rs`)
All 7 tests passing:
- ✅ `test_csv_config_default_values` - Verifies CsvConfig defaults
- ✅ `test_csv_config_to_writer_config` - Verifies config conversion
- ✅ `test_csv_config_from_toml` - Verifies TOML parsing
- ✅ `test_csv_config_partial_override` - Verifies partial config override
- ✅ `test_csv_writer_with_config_from_toml` - Verifies CSV writer integration
- ✅ `test_csv_writer_batching_from_config` - Verifies batching behavior
- ✅ `test_orchestrator_headers_match_snapshot` - Verifies header consistency

### Performance Tests (`tests/performance_tests.rs`)
All CSV-related performance tests passing:
- ✅ CSV rotation under load
- ✅ High-frequency CSV writes
- ✅ Memory usage stability
- ✅ Concurrent CSV access

### Unit Tests
- Config module: Comprehensive validation and parsing tests
- Models module: DexType and PoolSnapshot tests
- CSV Writer module: 20+ tests covering all functionality

## Code Quality

- ✅ No clippy blockers (all warnings addressed)
- ✅ Proper error handling with AppError
- ✅ Comprehensive documentation
- ✅ All dependencies properly wired in Cargo.toml

## Example Usage

### Using the Demo Mode

```bash
# Run with default config.toml
RUST_LOG=info cargo run -- --demo

# Run with custom config
RUST_LOG=info cargo run -- --demo --config ./config.example.toml

# Release build
RUST_LOG=info cargo run --release -- --demo --config ./config.example.toml
```

### Using the Example Binary

An alternative standalone example is also provided:

```bash
cargo run --example simple_csv_demo -- --config ./config.example.toml
```

### Output

Both demo mode and the example produce a CSV file at `./snapshots/demo_snapshots.csv` with:
- Correct 8-column header
- 3 synthetic pool snapshots
- Mixed DEX types (Raydium, PumpSwap, PumpFun)
- Proper liquidity_usd formatting (empty when None, formatted when Some)

## Configuration File

See `config.example.toml` for a complete example configuration with:
- RPC endpoints
- Output directory
- CSV configuration (append, batch size, rotation settings)
- Pool configurations
- All optional sections with defaults

## Next Steps

This implementation provides the core foundation for:
- **Issue 2**: Real Solana RPC and DEX decoding
- **Issue 3**: Observability, metrics, and price providers
- **Future Issues**: Advanced features and optimizations

All core components are production-ready and fully tested.
