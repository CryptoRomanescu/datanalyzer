/// Pump.fun bonding curve decoder implementation.
///
/// This module decodes Pump.fun bonding curve account data to extract reserve amounts.
///
/// # Pump.fun Account Structure Research
///
/// Based on research from:
/// - GitHub implementations (https://github.com/saifaleee/solana-bonding-curve)
/// - Solana Stack Exchange discussions
/// - Gist examples (https://gist.github.com/rubpy/6c57e9d12acd4b6ed84e9f205372631d)
///
/// ## Account Data Layout
///
/// Pump.fun bonding curve accounts use the following structure:
///
/// ```text
/// Offset  | Size | Field                  | Description
/// --------|------|------------------------|------------------------------------------
/// 0x00    | 8    | discriminator          | Account type identifier/signature
/// 0x08    | 8    | virtualTokenReserves   | Virtual token reserves for curve calc
/// 0x10    | 8    | virtualSolReserves     | Virtual SOL reserves for curve calc
/// 0x18    | 8    | realTokenReserves      | Actual token amount in the pool
/// 0x20    | 8    | realSolReserves        | Actual SOL amount in the pool
/// 0x28    | 8    | tokenTotalSupply       | Total supply of the token
/// 0x30    | 1    | complete               | Boolean flag (1 if graduated, 0 if not)
/// ```
///
/// All u64 values are stored in **little-endian** format (standard for Solana).
///
/// ## Reserve Fields
///
/// - **realTokenReserves (offset 0x18)**: The actual token vault amount
/// - **realSolReserves (offset 0x20)**: The actual SOL vault amount
///
/// These are the "real" reserves as opposed to "virtual" reserves which are used
/// for price calculation in the bonding curve formula but don't represent actual
/// liquidity in the pool.
///
/// ## Validation
///
/// Expected account size: 256 bytes (as defined in models.rs)
/// The discriminator at offset 0x00 can be used for additional validation if needed.

use crate::dex::DexDecoder;
use crate::error::AppError;

/// Decoder for Pump.fun bonding curve accounts.
///
/// This decoder extracts reserve data from Pump.fun bonding curve accounts.
/// Pump.fun uses a bonding curve mechanism where tokens are bought/sold against
/// a curve until the pool "graduates" to a full AMM (typically Raydium).
pub struct PumpFunDecoder;

impl PumpFunDecoder {
    /// Expected size of a Pump.fun bonding curve account.
    const ACCOUNT_SIZE: usize = 256;

    /// Offset for real token reserves in the account data.
    const REAL_TOKEN_RESERVES_OFFSET: usize = 0x18; // 24 bytes
    
    /// Offset for real SOL reserves in the account data.
    const REAL_SOL_RESERVES_OFFSET: usize = 0x20; // 32 bytes

    /// Size of a u64 field in bytes.
    const U64_SIZE: usize = 8;

    /// Minimum reasonable reserve value (1 lamport/smallest unit).
    const MIN_RESERVE_VALUE: u64 = 1;

    /// Maximum reasonable reserve value (to detect corrupted data).
    /// Set to 1 trillion tokens/SOL in base units.
    const MAX_RESERVE_VALUE: u64 = 1_000_000_000_000_000_000;

    /// Extract a u64 value from account data at the specified offset.
    ///
    /// # Arguments
    ///
    /// * `data` - The account data byte slice
    /// * `offset` - The byte offset to read from
    ///
    /// # Returns
    ///
    /// * `Some(u64)` - The extracted value if successful
    /// * `None` - If there's not enough data at the offset
    fn extract_u64(data: &[u8], offset: usize) -> Option<u64> {
        if offset + Self::U64_SIZE > data.len() {
            return None;
        }
        
        let bytes = &data[offset..offset + Self::U64_SIZE];
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Some(u64::from_le_bytes(array))
    }
}

impl DexDecoder for PumpFunDecoder {
    fn decode_reserves(&self, account_data: &[u8]) -> Result<(u64, u64), AppError> {
        // First validate the account
        self.validate_account(account_data)?;

        // Extract real token reserves (offset 0x18)
        let token_reserve = Self::extract_u64(account_data, Self::REAL_TOKEN_RESERVES_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(format!(
                    "Failed to extract token reserves at offset 0x{:02X}",
                    Self::REAL_TOKEN_RESERVES_OFFSET
                ))
            })?;

        // Extract real SOL reserves (offset 0x20)
        let sol_reserve = Self::extract_u64(account_data, Self::REAL_SOL_RESERVES_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(format!(
                    "Failed to extract SOL reserves at offset 0x{:02X}",
                    Self::REAL_SOL_RESERVES_OFFSET
                ))
            })?;

        Ok((token_reserve, sol_reserve))
    }

    fn validate_account(&self, account_data: &[u8]) -> Result<(), AppError> {
        // Check account size
        if account_data.len() != Self::ACCOUNT_SIZE {
            return Err(AppError::DecodingError(format!(
                "Invalid Pump.fun account size: expected {}, got {}",
                Self::ACCOUNT_SIZE,
                account_data.len()
            )));
        }

        // Verify we can extract both reserve values
        let token_reserve = Self::extract_u64(account_data, Self::REAL_TOKEN_RESERVES_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(
                    "Account data too small to contain token reserves".to_string()
                )
            })?;

        let sol_reserve = Self::extract_u64(account_data, Self::REAL_SOL_RESERVES_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(
                    "Account data too small to contain SOL reserves".to_string()
                )
            })?;

        // Validate reserve values are in reasonable range
        // Note: We allow zero reserves for newly created pools or completed curves
        if token_reserve > Self::MAX_RESERVE_VALUE {
            return Err(AppError::DecodingError(format!(
                "Token reserve value ({}) exceeds maximum reasonable value ({}). Data may be corrupted.",
                token_reserve,
                Self::MAX_RESERVE_VALUE
            )));
        }

        if sol_reserve > Self::MAX_RESERVE_VALUE {
            return Err(AppError::DecodingError(format!(
                "SOL reserve value ({}) exceeds maximum reasonable value ({}). Data may be corrupted.",
                sol_reserve,
                Self::MAX_RESERVE_VALUE
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create mock account data for testing.
    fn create_mock_account_data(token_reserve: u64, sol_reserve: u64) -> Vec<u8> {
        let mut data = vec![0u8; PumpFunDecoder::ACCOUNT_SIZE];
        
        // Set discriminator (first 8 bytes) - using a simple pattern
        data[0..8].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        
        // Set real token reserves at offset 0x18
        data[PumpFunDecoder::REAL_TOKEN_RESERVES_OFFSET..PumpFunDecoder::REAL_TOKEN_RESERVES_OFFSET + 8]
            .copy_from_slice(&token_reserve.to_le_bytes());
        
        // Set real SOL reserves at offset 0x20
        data[PumpFunDecoder::REAL_SOL_RESERVES_OFFSET..PumpFunDecoder::REAL_SOL_RESERVES_OFFSET + 8]
            .copy_from_slice(&sol_reserve.to_le_bytes());
        
        data
    }

    #[test]
    fn test_decode_reserves_valid_data() {
        let decoder = PumpFunDecoder;
        let token_reserve = 1_000_000_000; // 1 billion tokens
        let sol_reserve = 50_000_000_000;  // 50 SOL in lamports
        
        let account_data = create_mock_account_data(token_reserve, sol_reserve);
        
        let result = decoder.decode_reserves(&account_data);
        assert!(result.is_ok());
        
        let (decoded_token, decoded_sol) = result.unwrap();
        assert_eq!(decoded_token, token_reserve);
        assert_eq!(decoded_sol, sol_reserve);
    }

    #[test]
    fn test_decode_reserves_zero_values() {
        let decoder = PumpFunDecoder;
        let account_data = create_mock_account_data(0, 0);
        
        // Zero reserves should be valid (for new or completed pools)
        let result = decoder.decode_reserves(&account_data);
        assert!(result.is_ok());
        
        let (token, sol) = result.unwrap();
        assert_eq!(token, 0);
        assert_eq!(sol, 0);
    }

    #[test]
    fn test_validate_account_invalid_size() {
        let decoder = PumpFunDecoder;
        let invalid_data = vec![0u8; 100]; // Too small
        
        let result = decoder.validate_account(&invalid_data);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid Pump.fun account size"));
        assert!(err_msg.contains("expected 256"));
        assert!(err_msg.contains("got 100"));
    }

    #[test]
    fn test_validate_account_reserve_too_large() {
        let decoder = PumpFunDecoder;
        let excessive_reserve = PumpFunDecoder::MAX_RESERVE_VALUE + 1;
        let account_data = create_mock_account_data(excessive_reserve, 1000);
        
        let result = decoder.validate_account(&account_data);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceeds maximum reasonable value"));
    }

    #[test]
    fn test_decode_reserves_invalid_size() {
        let decoder = PumpFunDecoder;
        let invalid_data = vec![0u8; 50]; // Too small
        
        let result = decoder.decode_reserves(&invalid_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_u64_valid() {
        let mut data = vec![0u8; 32];
        let value: u64 = 0x0123456789ABCDEF;
        data[8..16].copy_from_slice(&value.to_le_bytes());
        
        let extracted = PumpFunDecoder::extract_u64(&data, 8);
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap(), value);
    }

    #[test]
    fn test_extract_u64_out_of_bounds() {
        let data = vec![0u8; 10];
        
        // Try to extract at an offset that doesn't have enough bytes
        let extracted = PumpFunDecoder::extract_u64(&data, 5);
        assert!(extracted.is_none());
    }

    #[test]
    fn test_little_endian_decoding() {
        let decoder = PumpFunDecoder;
        
        // Create account data with specific little-endian values
        let mut data = vec![0u8; PumpFunDecoder::ACCOUNT_SIZE];
        
        // Token reserve: 0x0000000000000064 (100 in decimal)
        data[0x18..0x20].copy_from_slice(&100u64.to_le_bytes());
        
        // SOL reserve: 0x00000000000003E8 (1000 in decimal)
        data[0x20..0x28].copy_from_slice(&1000u64.to_le_bytes());
        
        let result = decoder.decode_reserves(&data);
        assert!(result.is_ok());
        
        let (token, sol) = result.unwrap();
        assert_eq!(token, 100);
        assert_eq!(sol, 1000);
    }

    #[test]
    fn test_real_world_example() {
        // This test uses realistic values from a Pump.fun pool
        let decoder = PumpFunDecoder;
        
        // Realistic values:
        // - Token reserve: 800,000,000 tokens (800M with 6 decimals = 800 tokens)
        // - SOL reserve: 30 SOL = 30,000,000,000 lamports
        let token_reserve = 800_000_000_000_000;
        let sol_reserve = 30_000_000_000;
        
        let account_data = create_mock_account_data(token_reserve, sol_reserve);
        
        let result = decoder.decode_reserves(&account_data);
        assert!(result.is_ok());
        
        let (decoded_token, decoded_sol) = result.unwrap();
        assert_eq!(decoded_token, token_reserve);
        assert_eq!(decoded_sol, sol_reserve);
        
        // Verify price calculation would work
        let price = decoded_sol as f64 / decoded_token as f64;
        assert!(price > 0.0);
    }
}
