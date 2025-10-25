/// Raydium AMM pool decoder implementation.
///
/// This module decodes Raydium AMM V4 pool account data to extract vault pubkeys
/// and pool information.
///
/// # Raydium Account Structure Research
///
/// Based on official Raydium AMM source code:
/// - Repository: https://github.com/raydium-io/raydium-amm
/// - Source file: program/src/state.rs
/// - Raydium AMM V4 program ID: 675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8
///
/// ## Serialization Method
///
/// Raydium uses **bytemuck** for zero-copy deserialization, NOT Borsh or Anchor.
/// The AmmInfo structure is defined with `#[repr(C, packed)]` for predictable memory layout.
///
/// ## Account Data Layout (AMMv4)
///
/// The AmmInfo account is 752 bytes and contains:
/// - Pool configuration (status, decimals, fees, etc.)
/// - Vault pubkeys (coin_vault, pc_vault) - references to SPL token accounts
/// - Market and OpenBook integration data
/// - Statistical data (swap volumes, PnL, etc.)
///
/// ## Reserve Storage Architecture
///
/// **IMPORTANT**: Raydium does NOT store actual reserve amounts in the AmmInfo account.
/// Instead, it stores Pubkey references to vault accounts:
/// - `coin_vault`: Pubkey of the SPL token account holding base token reserves
/// - `pc_vault`: Pubkey of the SPL token account holding quote token reserves
///
/// To get actual reserve balances, you must:
/// 1. Decode AmmInfo to extract vault pubkeys
/// 2. Fetch vault account data via RPC (getAccountInfo)
/// 3. Parse the SPL token account to get the `amount` field
///
/// This decoder extracts the vault pubkeys. Fetching actual balances requires
/// an async RPC client, which is beyond the scope of this synchronous decoder trait.

use crate::dex::DexDecoder;
use crate::error::AppError;
use bytemuck::{Pod, Zeroable};
use solana_sdk::pubkey::Pubkey;

/// Fee structure for Raydium AMM pools.
///
/// Contains various fee ratios used in AMM operations:
/// - Trading fees
/// - Swap fees
/// - PnL distribution
/// - Minimum separation parameters
///
/// All fees are represented as numerator/denominator pairs to allow precise
/// fractional values without floating point arithmetic.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fees {
    /// Numerator of the minimum separation fee
    pub min_separate_numerator: u64,
    /// Denominator of the minimum separation fee
    pub min_separate_denominator: u64,
    
    /// Numerator of the trade fee (typically 25/10000 = 0.25%)
    pub trade_fee_numerator: u64,
    /// Denominator of the trade fee
    /// Must equal min_separate_denominator
    pub trade_fee_denominator: u64,
    
    /// Numerator of PnL distribution (typically 12/100 = 12%)
    pub pnl_numerator: u64,
    /// Denominator of PnL distribution
    pub pnl_denominator: u64,
    
    /// Numerator of swap fee (typically 25/10000 = 0.25%)
    pub swap_fee_numerator: u64,
    /// Denominator of swap fee
    pub swap_fee_denominator: u64,
}

// SAFETY: Fees contains only u64 fields with no padding or references
unsafe impl Zeroable for Fees {}
unsafe impl Pod for Fees {}

/// Statistical data for pool operations.
///
/// Tracks cumulative trading statistics including:
/// - Pending PnL amounts
/// - Total PnL accumulated
/// - Pool opening times
/// - Swap volumes and fees
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StateData {
    /// Pending PnL in coin to be collected
    pub need_take_pnl_coin: u64,
    /// Pending PnL in PC to be collected
    pub need_take_pnl_pc: u64,
    /// Total cumulative PnL in PC
    pub total_pnl_pc: u64,
    /// Total cumulative PnL in coin
    pub total_pnl_coin: u64,
    /// Unix timestamp when pool opened for trading
    pub pool_open_time: u64,
    /// Reserved for future use
    pub padding: [u64; 2],
    /// Time when transitioning from OrderBookOnly to Initialized
    pub orderbook_to_init_time: u64,
    
    /// Cumulative amount of coin tokens swapped in (128-bit for large volumes)
    pub swap_coin_in_amount: u128,
    /// Cumulative amount of PC swapped out
    pub swap_pc_out_amount: u128,
    /// Accumulated fees in PC from coin->PC swaps
    pub swap_acc_pc_fee: u64,
    
    /// Cumulative amount of PC swapped in
    pub swap_pc_in_amount: u128,
    /// Cumulative amount of coin swapped out
    pub swap_coin_out_amount: u128,
    /// Accumulated fees in coin from PC->coin swaps
    pub swap_acc_coin_fee: u64,
}

// SAFETY: StateData contains only primitive types (u64, u128, arrays)
unsafe impl Zeroable for StateData {}
unsafe impl Pod for StateData {}

/// Main AMM pool information structure (AMMv4).
///
/// This is the core account data structure for Raydium AMM V4 pools.
/// Total size: 752 bytes (packed).
///
/// # Important Fields for Reserve Calculation
///
/// - `coin_vault`: Pubkey of SPL token account holding base token (e.g., SOL)
/// - `pc_vault`: Pubkey of SPL token account holding quote token (e.g., USDC)
///
/// These vault pubkeys must be used to fetch actual token balances via RPC.
/// The AmmInfo structure does NOT contain the actual reserve amounts directly.
///
/// # Memory Layout
///
/// Uses `#[repr(C, packed)]` to ensure consistent byte layout matching
/// the on-chain program structure. This allows zero-copy deserialization
/// using bytemuck.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AmmInfo {
    /// Pool status (Uninitialized=0, Initialized=1, Disabled=2, etc.)
    pub status: u64,
    /// Nonce used for program address derivation
    pub nonce: u64,
    /// Maximum number of orders
    pub order_num: u64,
    /// Order depth percentage (e.g., 5 = 5% range)
    pub depth: u64,
    /// Decimal places for coin/base token
    pub coin_decimals: u64,
    /// Decimal places for PC/quote token
    pub pc_decimals: u64,
    /// Current AMM state machine state
    pub state: u64,
    /// Reset flag for pool operations
    pub reset_flag: u64,
    /// Minimum order size (1 = 0.000001)
    pub min_size: u64,
    /// Volume max cut ratio (numerator, with sys_decimal_value as denominator)
    pub vol_max_cut_ratio: u64,
    /// Amount wave (numerator, with sys_decimal_value as denominator)
    pub amount_wave: u64,
    /// Coin lot size (1 = 0.000001)
    pub coin_lot_size: u64,
    /// PC lot size (1 = 0.000001)
    pub pc_lot_size: u64,
    /// Minimum price multiplier
    pub min_price_multiplier: u64,
    /// Maximum price multiplier
    pub max_price_multiplier: u64,
    /// System decimal value for normalization
    pub sys_decimal_value: u64,
    
    /// Fee configuration (64 bytes)
    pub fees: Fees,
    
    /// Statistical data (144 bytes)
    pub state_data: StateData,
    
    /// **CRITICAL**: Pubkey of SPL token account holding coin/base reserves
    /// To get actual coin reserve amount, fetch this account via RPC and read the 'amount' field
    pub coin_vault: Pubkey,
    
    /// **CRITICAL**: Pubkey of SPL token account holding PC/quote reserves
    /// To get actual PC reserve amount, fetch this account via RPC and read the 'amount' field
    pub pc_vault: Pubkey,
    
    /// Mint address of coin/base token
    pub coin_vault_mint: Pubkey,
    /// Mint address of PC/quote token
    pub pc_vault_mint: Pubkey,
    /// LP token mint address
    pub lp_mint: Pubkey,
    /// OpenBook open orders account
    pub open_orders: Pubkey,
    /// Serum/OpenBook market address
    pub market: Pubkey,
    /// Serum/OpenBook program address
    pub market_program: Pubkey,
    /// Target orders account
    pub target_orders: Pubkey,
    
    /// Reserved for future use
    pub padding1: [u64; 8],
    
    /// AMM owner/admin pubkey
    pub amm_owner: Pubkey,
    /// Current LP token supply in the pool
    pub lp_amount: u64,
    /// Client order ID counter
    pub client_order_id: u64,
    /// Recent epoch for time-based operations
    pub recent_epoch: u64,
    /// Reserved for future use
    pub padding2: u64,
}

// SAFETY: AmmInfo is repr(C, packed) with only Pod-safe types (u64, Pubkey, nested Pod structs)
unsafe impl Zeroable for AmmInfo {}
unsafe impl Pod for AmmInfo {}

/// Helper structure to hold vault account information from AmmInfo.
///
/// This provides a convenient way to extract and work with vault pubkeys
/// without having to deal with the packed AmmInfo structure directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultInfo {
    /// Pubkey of the SPL token account holding coin/base reserves
    pub coin_vault: Pubkey,
    /// Pubkey of the SPL token account holding PC/quote reserves
    pub pc_vault: Pubkey,
    /// Mint address of the coin/base token
    pub coin_mint: Pubkey,
    /// Mint address of the PC/quote token
    pub pc_mint: Pubkey,
}

impl VaultInfo {
    /// Extract vault information from AmmInfo structure.
    ///
    /// # Arguments
    ///
    /// * `amm_info` - Reference to deserialized AmmInfo
    ///
    /// # Returns
    ///
    /// VaultInfo containing the vault pubkeys
    pub fn from_amm_info(amm_info: &AmmInfo) -> Self {
        Self {
            coin_vault: amm_info.coin_vault,
            pc_vault: amm_info.pc_vault,
            coin_mint: amm_info.coin_vault_mint,
            pc_mint: amm_info.pc_vault_mint,
        }
    }

    /// Format vault information as a string for display/logging.
    pub fn display(&self) -> String {
        format!(
            "Coin vault: {}, PC vault: {}, Coin mint: {}, PC mint: {}",
            self.coin_vault, self.pc_vault, self.coin_mint, self.pc_mint
        )
    }
}

/// Decoder for Raydium AMM pool accounts.
///
/// This decoder parses Raydium AMM V4 account data to extract vault pubkeys.
/// Note that actual reserve balances must be fetched separately from the vault accounts.
pub struct RaydiumDecoder;

impl RaydiumDecoder {
    /// Expected size of a Raydium AMMv4 pool account.
    /// This must match the size of the AmmInfo structure (752 bytes).
    const ACCOUNT_SIZE: usize = 752;

    /// Deserialize account data into AmmInfo structure.
    ///
    /// Uses bytemuck for zero-copy deserialization, matching the official
    /// Raydium implementation.
    ///
    /// # Arguments
    ///
    /// * `account_data` - Raw account data bytes from Solana
    ///
    /// # Returns
    ///
    /// * `Ok(&AmmInfo)` - Reference to deserialized pool info
    /// * `Err(AppError)` - If data is invalid or wrong size
    fn deserialize_amm_info(account_data: &[u8]) -> Result<&AmmInfo, AppError> {
        if account_data.len() != Self::ACCOUNT_SIZE {
            return Err(AppError::DecodingError(format!(
                "Invalid Raydium account size: expected {}, got {}",
                Self::ACCOUNT_SIZE,
                account_data.len()
            )));
        }

        // Use bytemuck for zero-copy deserialization
        // This is safe because we've verified the size and AmmInfo is Pod
        bytemuck::try_from_bytes::<AmmInfo>(account_data).map_err(|e| {
            AppError::DecodingError(format!("Failed to deserialize AmmInfo: {}", e))
        })
    }

    /// Extract vault information from account data.
    ///
    /// This is a convenience method that deserializes the AmmInfo and
    /// extracts just the vault-related pubkeys.
    ///
    /// # Arguments
    ///
    /// * `account_data` - Raw account data bytes from Solana
    ///
    /// # Returns
    ///
    /// * `Ok(VaultInfo)` - Vault pubkeys
    /// * `Err(AppError)` - If data is invalid
    ///
    /// # Example
    ///
    /// ```ignore
    /// let decoder = RaydiumDecoder;
    /// let vault_info = decoder.get_vault_info(&account_data)?;
    /// println!("Coin vault: {}", vault_info.coin_vault);
    /// ```
    pub fn get_vault_info(&self, account_data: &[u8]) -> Result<VaultInfo, AppError> {
        self.validate_account(account_data)?;
        let amm_info = Self::deserialize_amm_info(account_data)?;
        Ok(VaultInfo::from_amm_info(amm_info))
    }
}

impl DexDecoder for RaydiumDecoder {
    fn decode_reserves(&self, account_data: &[u8]) -> Result<(u64, u64), AppError> {
        // Validate first
        self.validate_account(account_data)?;

        // Deserialize the AmmInfo structure
        let amm_info = Self::deserialize_amm_info(account_data)?;

        // IMPORTANT NOTE:
        // Raydium stores reserves in separate vault accounts, not in AmmInfo.
        // The coin_vault and pc_vault fields contain Pubkeys of SPL token accounts.
        //
        // To get actual reserves, you would need to:
        // 1. Extract vault pubkeys: amm_info.coin_vault and amm_info.pc_vault
        // 2. Fetch those accounts via RPC (requires async client)
        // 3. Parse as SPL token accounts and read the 'amount' field
        //
        // This synchronous decoder cannot perform RPC calls.
        // For now, we return an informative error with the vault pubkeys.

        Err(AppError::DecodingError(format!(
            "Raydium reserves require fetching vault accounts. \
             Coin vault: {}, PC vault: {}. \
             Use an async RPC client to fetch these accounts and extract the 'amount' field \
             from the SPL token account data.",
            amm_info.coin_vault,
            amm_info.pc_vault
        )))
    }

    fn validate_account(&self, account_data: &[u8]) -> Result<(), AppError> {
        // Check account size
        if account_data.len() != Self::ACCOUNT_SIZE {
            return Err(AppError::DecodingError(format!(
                "Invalid Raydium account size: expected {}, got {}",
                Self::ACCOUNT_SIZE,
                account_data.len()
            )));
        }

        // Try to deserialize to validate structure
        let amm_info = Self::deserialize_amm_info(account_data)?;

        // Validate status is not uninitialized (status != 0)
        if amm_info.status == 0 {
            return Err(AppError::DecodingError(
                "Pool is uninitialized (status = 0)".to_string()
            ));
        }

        // Validate vault pubkeys are not default/zero
        if amm_info.coin_vault == Pubkey::default() {
            return Err(AppError::DecodingError(
                "Coin vault pubkey is default/zero".to_string()
            ));
        }

        if amm_info.pc_vault == Pubkey::default() {
            return Err(AppError::DecodingError(
                "PC vault pubkey is default/zero".to_string()
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    /// Helper function to create a valid AmmInfo structure for testing.
    fn create_test_amm_info() -> Vec<u8> {
        let mut data = vec![0u8; RaydiumDecoder::ACCOUNT_SIZE];
        
        // Create a minimal valid AmmInfo structure
        let mut amm_info = AmmInfo::default();
        
        // Set status to Initialized (1)
        amm_info.status = 1;
        amm_info.nonce = 255;
        amm_info.coin_decimals = 9;  // SOL decimals
        amm_info.pc_decimals = 6;    // USDC decimals
        
        // Set vault pubkeys (non-default)
        amm_info.coin_vault = Pubkey::new_unique();
        amm_info.pc_vault = Pubkey::new_unique();
        amm_info.coin_vault_mint = Pubkey::new_unique();
        amm_info.pc_vault_mint = Pubkey::new_unique();
        amm_info.lp_mint = Pubkey::new_unique();
        amm_info.open_orders = Pubkey::new_unique();
        amm_info.market = Pubkey::new_unique();
        amm_info.market_program = Pubkey::new_unique();
        amm_info.target_orders = Pubkey::new_unique();
        amm_info.amm_owner = Pubkey::new_unique();
        
        // Copy the structure to bytes
        let amm_bytes = bytemuck::bytes_of(&amm_info);
        data.copy_from_slice(amm_bytes);
        
        data
    }

    #[test]
    fn test_amm_info_size() {
        // Verify the AmmInfo structure is exactly 752 bytes as per Raydium spec
        assert_eq!(size_of::<AmmInfo>(), 752);
        assert_eq!(size_of::<AmmInfo>(), RaydiumDecoder::ACCOUNT_SIZE);
    }

    #[test]
    fn test_fees_size() {
        // Fees should be 64 bytes (8 u64 fields)
        assert_eq!(size_of::<Fees>(), 64);
    }

    #[test]
    fn test_state_data_size() {
        // StateData actual size is 144 bytes
        // 8 u64 fields (64 bytes) + 4 u128 fields (64 bytes) + 2 u64 fields (16 bytes) = 144 bytes
        assert_eq!(size_of::<StateData>(), 144);
    }

    #[test]
    fn test_validate_account_size() {
        let decoder = RaydiumDecoder;
        let valid_data = create_test_amm_info();
        
        let result = decoder.validate_account(&valid_data);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_account_invalid_size() {
        let decoder = RaydiumDecoder;
        let invalid_data = vec![0u8; 100];
        
        let result = decoder.validate_account(&invalid_data);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid Raydium account size"));
    }

    #[test]
    fn test_validate_account_uninitialized() {
        let decoder = RaydiumDecoder;
        // Create account with status = 0 (uninitialized)
        let data = vec![0u8; RaydiumDecoder::ACCOUNT_SIZE];
        
        let result = decoder.validate_account(&data);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("uninitialized"));
    }

    #[test]
    fn test_validate_account_default_vault_pubkeys() {
        let decoder = RaydiumDecoder;
        let mut data = vec![0u8; RaydiumDecoder::ACCOUNT_SIZE];
        
        // Set status to 1 (initialized) but leave vaults as default
        data[0] = 1; // status = 1 (little-endian u64)
        
        let result = decoder.validate_account(&data);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("vault pubkey"));
    }

    #[test]
    fn test_deserialize_amm_info() {
        let data = create_test_amm_info();
        
        let result = RaydiumDecoder::deserialize_amm_info(&data);
        assert!(result.is_ok());
        
        let amm_info = result.unwrap();
        // Copy values to avoid packed struct alignment issues
        let status = amm_info.status;
        let nonce = amm_info.nonce;
        let coin_decimals = amm_info.coin_decimals;
        let pc_decimals = amm_info.pc_decimals;
        let coin_vault = amm_info.coin_vault;
        let pc_vault = amm_info.pc_vault;
        
        assert_eq!(status, 1);
        assert_eq!(nonce, 255);
        assert_eq!(coin_decimals, 9);
        assert_eq!(pc_decimals, 6);
        assert_ne!(coin_vault, Pubkey::default());
        assert_ne!(pc_vault, Pubkey::default());
    }

    #[test]
    fn test_decode_reserves_returns_vault_info() {
        let decoder = RaydiumDecoder;
        let data = create_test_amm_info();
        
        let result = decoder.decode_reserves(&data);
        assert!(result.is_err());
        
        // The error should contain information about vault pubkeys
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("vault"));
        assert!(err_msg.contains("Coin vault:"));
        assert!(err_msg.contains("PC vault:"));
    }

    #[test]
    fn test_amm_info_field_layout() {
        // Test that we can correctly deserialize specific fields
        let data = create_test_amm_info();
        let amm_info = RaydiumDecoder::deserialize_amm_info(&data).unwrap();
        
        // Copy values to avoid packed struct alignment issues
        let status = amm_info.status;
        let coin_vault = amm_info.coin_vault;
        let pc_vault = amm_info.pc_vault;
        
        // Verify basic fields
        assert_eq!(status, 1);
        assert!(status > 0);
        
        // Verify vault pubkeys are non-zero
        assert_ne!(coin_vault, Pubkey::default());
        assert_ne!(pc_vault, Pubkey::default());
    }

    #[test]
    fn test_fees_default() {
        let fees = Fees::default();
        // Copy values to avoid packed struct alignment issues
        let min_sep_num = fees.min_separate_numerator;
        let min_sep_den = fees.min_separate_denominator;
        let trade_num = fees.trade_fee_numerator;
        let trade_den = fees.trade_fee_denominator;
        
        assert_eq!(min_sep_num, 0);
        assert_eq!(min_sep_den, 0);
        assert_eq!(trade_num, 0);
        assert_eq!(trade_den, 0);
    }

    #[test]
    fn test_state_data_default() {
        let state_data = StateData::default();
        // Copy values to avoid packed struct alignment issues
        let pnl_coin = state_data.need_take_pnl_coin;
        let pnl_pc = state_data.need_take_pnl_pc;
        let open_time = state_data.pool_open_time;
        
        assert_eq!(pnl_coin, 0);
        assert_eq!(pnl_pc, 0);
        assert_eq!(open_time, 0);
    }

    /// Test with realistic Raydium pool data structure
    #[test]
    fn test_realistic_pool_structure() {
        let mut data = vec![0u8; RaydiumDecoder::ACCOUNT_SIZE];
        let mut amm_info = AmmInfo::default();
        
        // Set realistic values for a SOL/USDC pool
        amm_info.status = 6; // SwapOnly status
        amm_info.nonce = 255;
        amm_info.order_num = 10;
        amm_info.depth = 5;
        amm_info.coin_decimals = 9;  // SOL
        amm_info.pc_decimals = 6;    // USDC
        amm_info.state = 1; // IdleState
        amm_info.sys_decimal_value = 1_000_000_000; // 10^9
        
        // Set fee structure (typical 0.25% = 25/10000)
        amm_info.fees.trade_fee_numerator = 25;
        amm_info.fees.trade_fee_denominator = 10000;
        amm_info.fees.swap_fee_numerator = 25;
        amm_info.fees.swap_fee_denominator = 10000;
        
        // Set vault pubkeys
        amm_info.coin_vault = Pubkey::new_unique();
        amm_info.pc_vault = Pubkey::new_unique();
        amm_info.coin_vault_mint = Pubkey::new_unique();
        amm_info.pc_vault_mint = Pubkey::new_unique();
        amm_info.lp_mint = Pubkey::new_unique();
        amm_info.open_orders = Pubkey::new_unique();
        amm_info.market = Pubkey::new_unique();
        amm_info.market_program = Pubkey::new_unique();
        amm_info.target_orders = Pubkey::new_unique();
        amm_info.amm_owner = Pubkey::new_unique();
        
        // Copy to bytes
        let amm_bytes = bytemuck::bytes_of(&amm_info);
        data.copy_from_slice(amm_bytes);
        
        let decoder = RaydiumDecoder;
        
        // Should validate successfully
        assert!(decoder.validate_account(&data).is_ok());
        
        // Should deserialize correctly
        let deserialized = RaydiumDecoder::deserialize_amm_info(&data).unwrap();
        
        // Copy values to avoid packed struct alignment issues
        let status = deserialized.status;
        let coin_decimals = deserialized.coin_decimals;
        let pc_decimals = deserialized.pc_decimals;
        let trade_fee_num = deserialized.fees.trade_fee_numerator;
        let trade_fee_den = deserialized.fees.trade_fee_denominator;
        
        assert_eq!(status, 6);
        assert_eq!(coin_decimals, 9);
        assert_eq!(pc_decimals, 6);
        assert_eq!(trade_fee_num, 25);
        assert_eq!(trade_fee_den, 10000);
    }

    #[test]
    fn test_vault_info_extraction() {
        let data = create_test_amm_info();
        let decoder = RaydiumDecoder;
        
        let vault_info = decoder.get_vault_info(&data).unwrap();
        
        // Verify vault info is extracted correctly
        assert_ne!(vault_info.coin_vault, Pubkey::default());
        assert_ne!(vault_info.pc_vault, Pubkey::default());
        assert_ne!(vault_info.coin_mint, Pubkey::default());
        assert_ne!(vault_info.pc_mint, Pubkey::default());
    }

    #[test]
    fn test_vault_info_display() {
        let vault_info = VaultInfo {
            coin_vault: Pubkey::new_unique(),
            pc_vault: Pubkey::new_unique(),
            coin_mint: Pubkey::new_unique(),
            pc_mint: Pubkey::new_unique(),
        };
        
        let display = vault_info.display();
        assert!(display.contains("Coin vault:"));
        assert!(display.contains("PC vault:"));
        assert!(display.contains("Coin mint:"));
        assert!(display.contains("PC mint:"));
    }

    #[test]
    fn test_vault_info_from_amm_info() {
        let data = create_test_amm_info();
        let amm_info = RaydiumDecoder::deserialize_amm_info(&data).unwrap();
        
        let vault_info = VaultInfo::from_amm_info(amm_info);
        
        // Copy values from packed struct
        let coin_vault = amm_info.coin_vault;
        let pc_vault = amm_info.pc_vault;
        
        assert_eq!(vault_info.coin_vault, coin_vault);
        assert_eq!(vault_info.pc_vault, pc_vault);
    }
}
