# Raydium Orchestrator - Quick Start Guide

## Overview

The Raydium orchestrator provides an async layer for fetching reserves from Raydium AMM pools. Unlike Pump.fun which stores reserves directly in the pool account, Raydium stores them in separate SPL token vault accounts.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
datanalyzer = "0.1.0"
solana-sdk = "1.18"
```

## Quick Start

### 1. Raydium Pool (Vault-based reserves)

```rust
use datanalyzer::orchestrator::ReserveOrchestrator;
use datanalyzer::dex::raydium::RaydiumDecoder;

// Fetch pool account data from RPC
let account_data = rpc_client.get_account_data(&pool_address)?;

// Decode to get vault information
let decoder = RaydiumDecoder;
let reserve_info = decoder.decode_reserve_info(&account_data)?;

// Fetch actual reserves from vaults
let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
let (base_reserve, quote_reserve) = orchestrator.resolve_reserves(&reserve_info)?;

println!("Base: {}, Quote: {}", base_reserve, quote_reserve);
```

### 2. Pump.fun Pool (Direct reserves)

```rust
use datanalyzer::orchestrator::ReserveOrchestrator;
use datanalyzer::dex::pumpfun::PumpFunDecoder;

// Fetch pool account data
let account_data = rpc_client.get_account_data(&pool_address)?;

// Decode to get direct reserves
let decoder = PumpFunDecoder;
let reserve_info = decoder.decode_reserve_info(&account_data)?;

// Resolve (no RPC needed for direct reserves)
let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
let (token_reserve, sol_reserve) = orchestrator.resolve_reserves(&reserve_info)?;

println!("Token: {}, SOL: {}", token_reserve, sol_reserve);
```

### 3. Unified Handling

```rust
use datanalyzer::orchestrator::{ReserveOrchestrator, ReserveInfo};

fn get_reserves(reserve_info: &ReserveInfo) -> Result<(u64, u64), AppError> {
    let orchestrator = ReserveOrchestrator::new(rpc_url);
    
    // Works for both Direct (Pump.fun) and RequiresVaults (Raydium)
    orchestrator.resolve_reserves(reserve_info)
}
```

## API Reference

### ReserveInfo Enum

```rust
pub enum ReserveInfo {
    /// Reserves stored directly in account data
    Direct { base: u64, quote: u64 },
    
    /// Reserves in separate vault accounts
    RequiresVaults(VaultInfo),
}
```

### ReserveOrchestrator

```rust
impl ReserveOrchestrator {
    /// Create new orchestrator with RPC endpoint
    pub fn new(rpc_url: String) -> Self;
    
    /// Fetch vault balances from Raydium vaults
    pub fn fetch_vault_balances(&self, vault_info: &VaultInfo) 
        -> Result<(u64, u64), AppError>;
    
    /// Resolve reserves (handles both types)
    pub fn resolve_reserves(&self, reserve_info: &ReserveInfo) 
        -> Result<(u64, u64), AppError>;
}
```

## Examples

Run the orchestrator demo:

```bash
cargo run --example orchestrator_demo
```

## Testing

Run all tests:

```bash
cargo test --lib
```

Run specific orchestrator tests:

```bash
cargo test orchestrator
```

All 147 tests pass ✅

## Documentation

- [ORCHESTRATOR_IMPLEMENTATION.md](ORCHESTRATOR_IMPLEMENTATION.md) - Full implementation details
- [SECURITY_SUMMARY.md](SECURITY_SUMMARY.md) - Security analysis
- [RAYDIUM_IMPLEMENTATION.md](RAYDIUM_IMPLEMENTATION.md) - Raydium decoder details

## Features

✅ Safe zero-copy deserialization with bytemuck  
✅ Pod-safe structures (no unsafe Pubkey usage)  
✅ Async RPC vault fetching  
✅ SPL token parsing  
✅ Comprehensive validation  
✅ Uniform API for different DEX types  
✅ Complete test coverage  
✅ No unsafe code

## Security

The implementation includes:
- Size and alignment validation
- Pool status validation
- Vault pubkey validation
- Mint address validation
- Proper error handling
- No unsafe code blocks

See [SECURITY_SUMMARY.md](SECURITY_SUMMARY.md) for details.

## Requirements

- Rust 1.70+
- solana-sdk 1.18
- solana-client 1.18
- spl-token 4.0
- bytemuck 1.14

## License

[Your License Here]
