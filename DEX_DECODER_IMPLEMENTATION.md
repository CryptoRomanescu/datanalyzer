# DEX Decoder Implementation

This document provides details about the DEX decoder implementation for parsing Solana liquidity pool account data.

## Overview

The DEX decoder module provides a unified interface for extracting reserve data from different DEX implementations on Solana. Currently implemented:

- ✅ **Pump.fun**: Fully implemented bonding curve decoder
- 🚧 **Raydium**: Placeholder (requires multi-account fetching)

## Architecture

### DexDecoder Trait

The `DexDecoder` trait defines a common interface for all DEX decoders:

```rust
pub trait DexDecoder: Send + Sync {
    fn decode_reserves(&self, account_data: &[u8]) -> Result<(u64, u64), AppError>;
    fn validate_account(&self, account_data: &[u8]) -> Result<(), AppError>;
}
```

Key features:
- Thread-safe (Send + Sync) for async contexts
- Returns (base_reserve, quote_reserve) tuple
- Validates data before decoding to prevent panics
- Uses little-endian byte order (Solana standard)

## Pump.fun Decoder

### Account Structure

Based on research from GitHub implementations and community documentation:

```
Offset  | Size | Field                  | Description
--------|------|------------------------|------------------------------------------
0x00    | 8    | discriminator          | Account type identifier
0x08    | 8    | virtualTokenReserves   | Virtual token reserves (for pricing)
0x10    | 8    | virtualSolReserves     | Virtual SOL reserves (for pricing)
0x18    | 8    | realTokenReserves      | ⭐ Actual token amount in pool
0x20    | 8    | realSolReserves        | ⭐ Actual SOL amount in pool
0x28    | 8    | tokenTotalSupply       | Total supply of the token
0x30    | 1    | complete               | Boolean: graduated to AMM?
```

### Key Points

- **Account Size**: 256 bytes
- **Token Reserve Offset**: 0x18 (24 bytes)
- **SOL Reserve Offset**: 0x20 (32 bytes)
- **Endianness**: Little-endian (standard for Solana)
- **Reserve Types**: 
  - Virtual reserves: Used for bonding curve pricing calculations
  - Real reserves: Actual liquidity in the pool (what we decode)

### Usage Example

```rust
use crate::dex::{DexDecoder, pumpfun::PumpFunDecoder};

fn example_usage() -> Result<(), AppError> {
    let decoder = PumpFunDecoder;
    
    // Assume account_data fetched from Solana RPC
    let account_data: Vec<u8> = /* ... */;
    
    // Validate first
    decoder.validate_account(&account_data)?;
    
    // Decode reserves
    let (token_reserve, sol_reserve) = decoder.decode_reserves(&account_data)?;
    
    println!("Token Reserve: {} tokens", token_reserve);
    println!("SOL Reserve: {} lamports", sol_reserve);
    
    // Calculate price (SOL per token)
    let price = sol_reserve as f64 / token_reserve as f64;
    println!("Price: {} SOL per token", price);
    
    Ok(())
}
```

### Validation

The Pump.fun decoder performs the following validations:

1. **Size Check**: Account data must be exactly 256 bytes
2. **Reserve Range Check**: Values must be within reasonable bounds
   - Min: 0 (allowed for new/completed pools)
   - Max: 1 trillion tokens/SOL (prevents corrupted data)
3. **Data Integrity**: Ensures offsets are within bounds before extraction

### Error Handling

The decoder returns detailed error messages:

```rust
// Example error messages:
"Invalid Pump.fun account size: expected 256, got 128"
"Failed to extract token reserves at offset 0x18"
"SOL reserve value (9999999999999999999) exceeds maximum reasonable value"
```

## Testing

The implementation includes comprehensive unit tests:

### Pump.fun Tests (13 tests)
- ✅ Valid data decoding
- ✅ Zero reserve values
- ✅ Invalid account sizes
- ✅ Reserve values exceeding limits
- ✅ Little-endian byte order verification
- ✅ Out-of-bounds offset handling
- ✅ Real-world example values

### Test Coverage

All 38 tests in the project pass, including:
- 13 tests for Pump.fun decoder
- 3 tests for Raydium placeholder
- 1 test for trait object safety
- 21 tests for existing config and models

## Integration with Existing Code

The decoder integrates with the existing error handling system:

```rust
pub enum AppError {
    ConfigError(String),
    RpcError(String),
    DecodingError(String),  // ← Used by decoders
    CsvError(String),
    PriceError(String),
}
```

## Future Work

### Raydium Implementation

Raydium requires a more complex approach:
1. Parse AmmInfo account to get vault pubkeys
2. Fetch vault token accounts (2 separate RPC calls)
3. Extract amount from each vault account

Expected structure:
- Account Size: 752 bytes
- Requires multi-account fetching
- More complex than Pump.fun's single-account approach

### Potential Enhancements

1. **Discriminator Validation**: Add magic byte checking for additional safety
2. **Retry Logic**: Implement fallback for incomplete data
3. **Logging**: Add debug logging for decoding operations
4. **Performance**: Consider caching account parsers
5. **More DEXs**: Add support for Orca, Meteora, etc.

## References

### Pump.fun Research
- [Solana Bonding Curve Implementation](https://github.com/saifaleee/solana-bonding-curve)
- [Solana Stack Exchange Discussion](https://solana.stackexchange.com/questions/19990/how-to-get-pumpfun-bonding-curve-data)
- [Bonding Curve State Fetching](https://gist.github.com/rubpy/6c57e9d12acd4b6ed84e9f205372631d)
- [Pump.fun Bonding Curve Docs](https://deepwiki.com/pump-fun/pump-public-docs)

### Raydium Research
- [Raydium AMM GitHub](https://github.com/raydium-io/raydium-amm)
- [Shyft Streaming Documentation](https://docs.shyft.to/solana-yellowstone-grpc)
- [Raydium Pool Creation Docs](https://docs.raydium.io/raydium/pool-creation)

## Security Considerations

✅ **No vulnerabilities detected** by CodeQL analysis

The implementation:
- Validates all input data before processing
- Uses safe array indexing with bounds checking
- Returns Result types for proper error propagation
- Avoids panics through validation
- Uses defensive programming practices
