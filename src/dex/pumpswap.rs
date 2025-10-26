/// PumpSwap AMM pool decoder implementation.
///
/// This module decodes PumpSwap AMM pool account data to extract reserve amounts and mint addresses.
///
/// # PumpSwap AMM Pool Structure
///
/// PumpSwap is an automated market maker (AMM) on Solana that provides liquidity pools.
/// The program ID is: pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA
///
/// ## Account Data Layout
///
/// PumpSwap AMM pool accounts use the following structure:
///
/// ```text
/// Offset  | Size | Field                  | Description
/// --------|------|------------------------|------------------------------------------
/// 0x00    | 8    | discriminator          | Account type identifier
/// 0x08    | 32   | base_mint              | Base token mint address
/// 0x28    | 32   | quote_mint             | Quote token mint address
/// 0x48    | 8    | base_reserve           | Base token reserve amount
/// 0x50    | 8    | quote_reserve          | Quote token reserve amount
/// ...     | ...  | ...                    | Other fields
/// ```
///
/// All u64 values are stored in **little-endian** format (standard for Solana).
/// All Pubkey values are 32 bytes.
///
/// ## Reserve Fields
///
/// - **base_reserve (offset 0x48)**: The base token amount in the pool
/// - **quote_reserve (offset 0x50)**: The quote token amount in the pool
///
/// ## Mint Fields
///
/// - **base_mint (offset 0x08)**: The mint address of the base token
/// - **quote_mint (offset 0x28)**: The mint address of the quote token (usually SOL, USDC, or USDT)
use crate::dex::DexDecoder;
use crate::error::AppError;
use solana_sdk::pubkey::Pubkey;

/// Decoder for PumpSwap AMM pool accounts.
///
/// This decoder extracts reserve data and mint addresses from PumpSwap pool accounts.
pub struct PumpSwapDecoder;

impl PumpSwapDecoder {
    /// Expected size of a PumpSwap AMM pool account.
    pub const ACCOUNT_SIZE: usize = 324;

    /// Offset for base mint in the account data.
    pub const BASE_MINT_OFFSET: usize = 0x08; // 8 bytes

    /// Offset for quote mint in the account data.
    pub const QUOTE_MINT_OFFSET: usize = 0x28; // 40 bytes

    /// Offset for base reserve in the account data.
    pub const BASE_RESERVE_OFFSET: usize = 0x48; // 72 bytes

    /// Offset for quote reserve in the account data.
    pub const QUOTE_RESERVE_OFFSET: usize = 0x50; // 80 bytes

    /// Size of a u64 field in bytes.
    const U64_SIZE: usize = 8;

    /// Size of a Pubkey field in bytes.
    const PUBKEY_SIZE: usize = 32;

    /// Maximum reasonable reserve value (to detect corrupted data).
    /// Set to 1 trillion tokens in base units.
    const MAX_RESERVE_VALUE: u64 = 1_000_000_000_000_000_000;

    /// Extract a u64 value from account data at the specified offset.
    fn extract_u64(data: &[u8], offset: usize) -> Option<u64> {
        if offset + Self::U64_SIZE > data.len() {
            return None;
        }

        let bytes = &data[offset..offset + Self::U64_SIZE];
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Some(u64::from_le_bytes(array))
    }

    /// Extract a Pubkey from account data at the specified offset.
    pub fn extract_pubkey(data: &[u8], offset: usize) -> Option<Pubkey> {
        if offset + Self::PUBKEY_SIZE > data.len() {
            return None;
        }

        let bytes = &data[offset..offset + Self::PUBKEY_SIZE];
        let mut array = [0u8; 32];
        array.copy_from_slice(bytes);
        Some(Pubkey::new_from_array(array))
    }

    /// Extract the base mint from pool account data.
    pub fn extract_base_mint(data: &[u8]) -> Result<Pubkey, AppError> {
        Self::extract_pubkey(data, Self::BASE_MINT_OFFSET).ok_or_else(|| {
            AppError::DecodingError(format!(
                "Failed to extract base mint at offset 0x{:02X}",
                Self::BASE_MINT_OFFSET
            ))
        })
    }

    /// Extract the quote mint from pool account data.
    pub fn extract_quote_mint(data: &[u8]) -> Result<Pubkey, AppError> {
        Self::extract_pubkey(data, Self::QUOTE_MINT_OFFSET).ok_or_else(|| {
            AppError::DecodingError(format!(
                "Failed to extract quote mint at offset 0x{:02X}",
                Self::QUOTE_MINT_OFFSET
            ))
        })
    }
}

impl DexDecoder for PumpSwapDecoder {
    fn decode_reserves(&self, account_data: &[u8]) -> Result<(u64, u64), AppError> {
        // First validate the account
        self.validate_account(account_data)?;

        // Extract base reserve (offset 0x48)
        let base_reserve = Self::extract_u64(account_data, Self::BASE_RESERVE_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(format!(
                    "Failed to extract base reserves at offset 0x{:02X}",
                    Self::BASE_RESERVE_OFFSET
                ))
            })?;

        // Extract quote reserve (offset 0x50)
        let quote_reserve = Self::extract_u64(account_data, Self::QUOTE_RESERVE_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(format!(
                    "Failed to extract quote reserves at offset 0x{:02X}",
                    Self::QUOTE_RESERVE_OFFSET
                ))
            })?;

        Ok((base_reserve, quote_reserve))
    }

    fn validate_account(&self, account_data: &[u8]) -> Result<(), AppError> {
        // Check account size
        if account_data.len() != Self::ACCOUNT_SIZE {
            return Err(AppError::DecodingError(format!(
                "Invalid PumpSwap account size: expected {}, got {}",
                Self::ACCOUNT_SIZE,
                account_data.len()
            )));
        }

        // Verify we can extract both reserve values
        let base_reserve = Self::extract_u64(account_data, Self::BASE_RESERVE_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(
                    "Account data too small to contain base reserves".to_string(),
                )
            })?;

        let quote_reserve = Self::extract_u64(account_data, Self::QUOTE_RESERVE_OFFSET)
            .ok_or_else(|| {
                AppError::DecodingError(
                    "Account data too small to contain quote reserves".to_string(),
                )
            })?;

        // Validate reserve values are in reasonable range
        // Note: We allow zero reserves for newly created pools
        if base_reserve > Self::MAX_RESERVE_VALUE {
            return Err(AppError::DecodingError(format!(
                "Base reserve value ({}) exceeds maximum reasonable value ({}). Data may be corrupted.",
                base_reserve,
                Self::MAX_RESERVE_VALUE
            )));
        }

        if quote_reserve > Self::MAX_RESERVE_VALUE {
            return Err(AppError::DecodingError(format!(
                "Quote reserve value ({}) exceeds maximum reasonable value ({}). Data may be corrupted.",
                quote_reserve,
                Self::MAX_RESERVE_VALUE
            )));
        }

        // Verify we can extract mint addresses
        Self::extract_base_mint(account_data)?;
        Self::extract_quote_mint(account_data)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create mock account data for testing.
    fn create_mock_account_data(
        base_mint: Pubkey,
        quote_mint: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Vec<u8> {
        let mut data = vec![0u8; PumpSwapDecoder::ACCOUNT_SIZE];

        // Set discriminator (first 8 bytes) - using a simple pattern
        data[0..8].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

        // Set base mint at offset 0x08
        data[PumpSwapDecoder::BASE_MINT_OFFSET
            ..PumpSwapDecoder::BASE_MINT_OFFSET + 32]
            .copy_from_slice(&base_mint.to_bytes());

        // Set quote mint at offset 0x28
        data[PumpSwapDecoder::QUOTE_MINT_OFFSET
            ..PumpSwapDecoder::QUOTE_MINT_OFFSET + 32]
            .copy_from_slice(&quote_mint.to_bytes());

        // Set base reserve at offset 0x48
        data[PumpSwapDecoder::BASE_RESERVE_OFFSET
            ..PumpSwapDecoder::BASE_RESERVE_OFFSET + 8]
            .copy_from_slice(&base_reserve.to_le_bytes());

        // Set quote reserve at offset 0x50
        data[PumpSwapDecoder::QUOTE_RESERVE_OFFSET
            ..PumpSwapDecoder::QUOTE_RESERVE_OFFSET + 8]
            .copy_from_slice(&quote_reserve.to_le_bytes());

        data
    }

    #[test]
    fn test_decode_reserves_valid_data() {
        let decoder = PumpSwapDecoder;
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();
        let base_reserve = 1_000_000_000; // 1 billion tokens
        let quote_reserve = 50_000_000_000; // 50 tokens in base units

        let account_data = create_mock_account_data(base_mint, quote_mint, base_reserve, quote_reserve);

        let result = decoder.decode_reserves(&account_data);
        assert!(result.is_ok());

        let (base, quote) = result.unwrap();
        assert_eq!(base, base_reserve);
        assert_eq!(quote, quote_reserve);
    }

    #[test]
    fn test_decode_reserves_zero_reserves() {
        let decoder = PumpSwapDecoder;
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();

        let account_data = create_mock_account_data(base_mint, quote_mint, 0, 0);

        let result = decoder.decode_reserves(&account_data);
        assert!(result.is_ok());

        let (base, quote) = result.unwrap();
        assert_eq!(base, 0);
        assert_eq!(quote, 0);
    }

    #[test]
    fn test_validate_account_invalid_size() {
        let decoder = PumpSwapDecoder;
        let data = vec![0u8; 100]; // Invalid size

        let result = decoder.validate_account(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid PumpSwap account size"));
    }

    #[test]
    fn test_validate_account_corrupted_reserves() {
        let decoder = PumpSwapDecoder;
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();
        let invalid_reserve = PumpSwapDecoder::MAX_RESERVE_VALUE + 1;

        let account_data = create_mock_account_data(base_mint, quote_mint, invalid_reserve, 1000);

        let result = decoder.validate_account(&account_data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum reasonable value"));
    }

    #[test]
    fn test_extract_mints() {
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();

        let account_data = create_mock_account_data(base_mint, quote_mint, 1000, 2000);

        let extracted_base = PumpSwapDecoder::extract_base_mint(&account_data).unwrap();
        let extracted_quote = PumpSwapDecoder::extract_quote_mint(&account_data).unwrap();

        assert_eq!(extracted_base, base_mint);
        assert_eq!(extracted_quote, quote_mint);
    }

    #[test]
    fn test_extract_mints_invalid_data() {
        let data = vec![0u8; 30]; // Too small for both mints (BASE_MINT needs 40, QUOTE_MINT needs 72)

        let result = PumpSwapDecoder::extract_base_mint(&data);
        assert!(result.is_err());

        let result = PumpSwapDecoder::extract_quote_mint(&data);
        assert!(result.is_err());
    }
}
