/// Orchestrator module for async DEX operations.
///
/// This module provides async functionality for fetching reserves from DEX pools,
/// particularly those that store reserves in separate vault accounts (like Raydium).

use crate::dex::raydium::VaultInfo;
use crate::error::AppError;
use solana_client::rpc_client::RpcClient;
use solana_sdk::program_pack::Pack;
use spl_token::state::Account as TokenAccount;

/// Result of decoding pool reserves.
///
/// Different DEXs store reserves differently:
/// - Some store reserves directly in the pool account (Pump.fun)
/// - Some store references to vault accounts that hold reserves (Raydium)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveInfo {
    /// Reserves are stored directly in the account data.
    /// Returns (base_reserve, quote_reserve).
    Direct { base: u64, quote: u64 },

    /// Reserves are stored in separate vault accounts.
    /// Requires async RPC calls to fetch actual balances.
    RequiresVaults(VaultInfo),
}

/// Async orchestrator for fetching DEX pool reserves.
///
/// This orchestrator handles both direct reserve access (Pump.fun) and
/// vault-based reserves (Raydium) through RPC calls.
pub struct ReserveOrchestrator {
    rpc_client: RpcClient,
}

impl ReserveOrchestrator {
    /// Create a new reserve orchestrator with an RPC client.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The Solana RPC endpoint URL
    ///
    /// # Returns
    ///
    /// A new ReserveOrchestrator instance
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_client: RpcClient::new(rpc_url),
        }
    }

    /// Create a new reserve orchestrator with a custom RPC client.
    ///
    /// # Arguments
    ///
    /// * `rpc_client` - A configured RPC client
    ///
    /// # Returns
    ///
    /// A new ReserveOrchestrator instance
    pub fn with_client(rpc_client: RpcClient) -> Self {
        Self { rpc_client }
    }

    /// Fetch vault balances from Raydium vault accounts.
    ///
    /// This method fetches the actual token balances from the vault accounts
    /// referenced in the Raydium AmmInfo structure.
    ///
    /// # Arguments
    ///
    /// * `vault_info` - Vault information extracted from AmmInfo
    ///
    /// # Returns
    ///
    /// * `Ok((u64, u64))` - Tuple of (base_reserve, quote_reserve)
    /// * `Err(AppError)` - If RPC call fails or vault parsing fails
    ///
    /// # Errors
    ///
    /// This function will return an error if:
    /// - RPC call fails (network error, account not found, etc.)
    /// - Account data is not a valid SPL token account
    /// - Account data is too small
    pub fn fetch_vault_balances(&self, vault_info: &VaultInfo) -> Result<(u64, u64), AppError> {
        // Fetch coin vault account
        let coin_vault_account = self
            .rpc_client
            .get_account(&vault_info.coin_vault)
            .map_err(|e| {
                AppError::RpcError(format!("Failed to fetch coin vault account: {}", e))
            })?;

        // Fetch PC vault account
        let pc_vault_account = self
            .rpc_client
            .get_account(&vault_info.pc_vault)
            .map_err(|e| AppError::RpcError(format!("Failed to fetch PC vault account: {}", e)))?;

        // Parse coin vault as SPL token account
        let coin_token_account = TokenAccount::unpack_from_slice(&coin_vault_account.data).map_err(|e| {
            AppError::DecodingError(format!("Failed to parse coin vault as token account: {}", e))
        })?;

        // Parse PC vault as SPL token account
        let pc_token_account = TokenAccount::unpack_from_slice(&pc_vault_account.data).map_err(|e| {
            AppError::DecodingError(format!("Failed to parse PC vault as token account: {}", e))
        })?;

        // Verify mint addresses match
        if coin_token_account.mint != vault_info.coin_mint {
            return Err(AppError::DecodingError(format!(
                "Coin vault mint mismatch: expected {}, got {}",
                vault_info.coin_mint, coin_token_account.mint
            )));
        }

        if pc_token_account.mint != vault_info.pc_mint {
            return Err(AppError::DecodingError(format!(
                "PC vault mint mismatch: expected {}, got {}",
                vault_info.pc_mint, pc_token_account.mint
            )));
        }

        Ok((coin_token_account.amount, pc_token_account.amount))
    }

    /// Resolve reserve info into actual reserve amounts.
    ///
    /// This method handles both direct reserves and vault-based reserves,
    /// fetching from RPC when necessary.
    ///
    /// # Arguments
    ///
    /// * `reserve_info` - Reserve information from decoder
    ///
    /// # Returns
    ///
    /// * `Ok((u64, u64))` - Tuple of (base_reserve, quote_reserve)
    /// * `Err(AppError)` - If RPC call fails or parsing fails
    pub fn resolve_reserves(&self, reserve_info: &ReserveInfo) -> Result<(u64, u64), AppError> {
        match reserve_info {
            ReserveInfo::Direct { base, quote } => Ok((*base, *quote)),
            ReserveInfo::RequiresVaults(vault_info) => self.fetch_vault_balances(vault_info),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_reserve_info_direct() {
        let reserve_info = ReserveInfo::Direct {
            base: 1000,
            quote: 2000,
        };

        match reserve_info {
            ReserveInfo::Direct { base, quote } => {
                assert_eq!(base, 1000);
                assert_eq!(quote, 2000);
            }
            _ => panic!("Expected Direct variant"),
        }
    }

    #[test]
    fn test_reserve_info_requires_vaults() {
        let vault_info = VaultInfo {
            coin_vault: Pubkey::new_unique(),
            pc_vault: Pubkey::new_unique(),
            coin_mint: Pubkey::new_unique(),
            pc_mint: Pubkey::new_unique(),
        };

        let reserve_info = ReserveInfo::RequiresVaults(vault_info);

        match reserve_info {
            ReserveInfo::RequiresVaults(info) => {
                assert_eq!(info.coin_vault, vault_info.coin_vault);
                assert_eq!(info.pc_vault, vault_info.pc_vault);
            }
            _ => panic!("Expected RequiresVaults variant"),
        }
    }

    #[test]
    fn test_orchestrator_new() {
        let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
        // Just verify it constructs without panic
        assert!(true);
    }

    #[test]
    fn test_orchestrator_resolve_direct() {
        let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
        
        let reserve_info = ReserveInfo::Direct {
            base: 5000,
            quote: 10000,
        };

        let result = orchestrator.resolve_reserves(&reserve_info);
        assert!(result.is_ok());
        let (base, quote) = result.unwrap();
        assert_eq!(base, 5000);
        assert_eq!(quote, 10000);
    }
}
