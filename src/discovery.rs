/// Pool discovery module for automatic detection and subscription of DEX pools.
///
/// This module provides functionality to:
/// - Backfill existing pools by querying program accounts
/// - Live subscribe to new pools via programSubscribe WebSocket
/// - Filter pools based on quote token allowlist and liquidity thresholds
/// - Dynamically add pools to the orchestrator without restart

use crate::config::{DiscoveryConfig, PoolConfig};
use crate::dex::pumpswap::PumpSwapDecoder;
use crate::dex::DexDecoder;
use crate::error::AppError;
use crate::models::DexType;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Pool discovery service for automatically finding and subscribing to pools
pub struct PoolDiscovery {
    config: DiscoveryConfig,
    rpc: Arc<RpcClient>,
    discovered_pools: Arc<RwLock<HashSet<Pubkey>>>,
    quote_allowlist: HashSet<Pubkey>,
}

impl PoolDiscovery {
    /// Create a new pool discovery service
    pub fn new(config: DiscoveryConfig, rpc_url: String) -> Result<Self, AppError> {
        let rpc = Arc::new(RpcClient::new(rpc_url));

        // Parse quote allowlist
        let mut quote_allowlist = HashSet::new();
        for mint_str in &config.quote_allowlist {
            let mint = Pubkey::from_str(mint_str).map_err(|e| {
                AppError::ConfigError(format!("Invalid quote mint '{}': {}", mint_str, e))
            })?;
            quote_allowlist.insert(mint);
        }

        Ok(Self {
            config,
            rpc,
            discovered_pools: Arc::new(RwLock::new(HashSet::new())),
            quote_allowlist,
        })
    }

    /// Backfill existing PumpSwap pools from the blockchain
    pub async fn backfill_pumpswap_pools(&self) -> Result<Vec<PoolConfig>, AppError> {
        if !self.config.enable_pumpswap {
            log::info!("PumpSwap discovery disabled, skipping backfill");
            return Ok(Vec::new());
        }

        log::info!("Starting PumpSwap pool discovery backfill...");

        let program_id = Pubkey::from_str(&self.config.pumpswap_program_id).map_err(|e| {
            AppError::ConfigError(format!(
                "Invalid PumpSwap program ID '{}': {}",
                self.config.pumpswap_program_id, e
            ))
        })?;

        // Configure RPC request to filter by account size (PumpSwap pool size)
        let filters = vec![RpcFilterType::DataSize(
            PumpSwapDecoder::ACCOUNT_SIZE as u64,
        )];

        let config = RpcProgramAccountsConfig {
            filters: Some(filters),
            account_config: RpcAccountInfoConfig {
                encoding: Some(solana_account_decoder::UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::confirmed()),
                ..Default::default()
            },
            with_context: Some(false),
        };

        // Fetch all program accounts
        log::info!("Fetching PumpSwap program accounts...");
        let accounts = self
            .rpc
            .get_program_accounts_with_config(&program_id, config)
            .await
            .map_err(|e| {
                AppError::RpcError(format!("Failed to fetch program accounts: {}", e))
            })?;

        log::info!(
            "Found {} PumpSwap accounts, filtering...",
            accounts.len()
        );

        let mut pool_configs = Vec::new();
        let mut discovered = self.discovered_pools.write().await;

        for (pool_address, account) in accounts {
            // Skip if already discovered
            if discovered.contains(&pool_address) {
                continue;
            }

            // Check if we've reached max pools
            if pool_configs.len() >= self.config.max_pools {
                log::warn!(
                    "Reached max pools limit ({}), stopping backfill",
                    self.config.max_pools
                );
                break;
            }

            // Try to decode and filter the pool
            match self.filter_pool(&pool_address, &account.data) {
                Ok(Some(pool_config)) => {
                    log::info!("Discovered PumpSwap pool: {}", pool_address);
                    pool_configs.push(pool_config);
                    discovered.insert(pool_address);
                }
                Ok(None) => {
                    // Pool didn't pass filters, skip silently
                }
                Err(e) => {
                    log::debug!("Failed to decode pool {}: {}", pool_address, e);
                }
            }
        }

        log::info!(
            "Backfill complete: discovered {} new PumpSwap pools",
            pool_configs.len()
        );

        Ok(pool_configs)
    }

    /// Filter a pool based on configuration criteria
    ///
    /// Returns Ok(Some(PoolConfig)) if pool passes filters,
    /// Ok(None) if pool doesn't pass filters,
    /// Err if there's a decoding error
    fn filter_pool(
        &self,
        pool_address: &Pubkey,
        account_data: &[u8],
    ) -> Result<Option<PoolConfig>, AppError> {
        // Extract quote mint
        let quote_mint = PumpSwapDecoder::extract_quote_mint(account_data)?;

        // Check if quote mint is in allowlist
        if !self.quote_allowlist.contains(&quote_mint) {
            return Ok(None);
        }

        // Extract reserves to check liquidity
        let decoder = PumpSwapDecoder;
        let (_base_reserve, quote_reserve) = decoder.decode_reserves(account_data)?;

        // Check minimum liquidity
        if (quote_reserve as f64) < self.config.min_quote_liquidity {
            return Ok(None);
        }

        // Extract base mint for the pool config
        let base_mint = PumpSwapDecoder::extract_base_mint(account_data)?;

        // Create pool config
        let pool_config = PoolConfig::new(*pool_address, DexType::PumpSwap, base_mint)?;

        Ok(Some(pool_config))
    }

    /// Check if a pool has already been discovered
    pub async fn is_discovered(&self, pool_address: &Pubkey) -> bool {
        self.discovered_pools
            .read()
            .await
            .contains(pool_address)
    }

    /// Mark a pool as discovered
    pub async fn mark_discovered(&self, pool_address: Pubkey) {
        self.discovered_pools.write().await.insert(pool_address);
    }

    /// Get the number of discovered pools
    pub async fn discovered_count(&self) -> usize {
        self.discovered_pools.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_mock_pumpswap_account(
        base_mint: Pubkey,
        quote_mint: Pubkey,
        base_reserve: u64,
        quote_reserve: u64,
    ) -> Vec<u8> {
        let mut data = vec![0u8; PumpSwapDecoder::ACCOUNT_SIZE];

        // Set discriminator
        data[0..8].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);

        // Set base mint at offset 0x08
        data[PumpSwapDecoder::BASE_MINT_OFFSET..PumpSwapDecoder::BASE_MINT_OFFSET + 32]
            .copy_from_slice(&base_mint.to_bytes());

        // Set quote mint at offset 0x28
        data[PumpSwapDecoder::QUOTE_MINT_OFFSET..PumpSwapDecoder::QUOTE_MINT_OFFSET + 32]
            .copy_from_slice(&quote_mint.to_bytes());

        // Set reserves
        data[PumpSwapDecoder::BASE_RESERVE_OFFSET..PumpSwapDecoder::BASE_RESERVE_OFFSET + 8]
            .copy_from_slice(&base_reserve.to_le_bytes());

        data[PumpSwapDecoder::QUOTE_RESERVE_OFFSET..PumpSwapDecoder::QUOTE_RESERVE_OFFSET + 8]
            .copy_from_slice(&quote_reserve.to_le_bytes());

        data
    }

    #[tokio::test]
    async fn test_filter_pool_passes_filters() {
        let config = DiscoveryConfig {
            enable_pumpswap: true,
            pumpswap_program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
            quote_allowlist: vec!["So11111111111111111111111111111111111111112".to_string()],
            min_quote_liquidity: 1000.0,
            max_pools: 100,
            rescan_interval_secs: 300,
        };

        let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
            .unwrap();

        let pool_address = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

        let account_data = create_mock_pumpswap_account(base_mint, quote_mint, 1_000_000, 2000);

        let result = discovery.filter_pool(&pool_address, &account_data).unwrap();
        assert!(result.is_some());

        let pool_config = result.unwrap();
        assert_eq!(pool_config.pool_address(), &pool_address);
        assert_eq!(pool_config.dex_type(), DexType::PumpSwap);
        assert_eq!(pool_config.token_mint(), &base_mint);
    }

    #[tokio::test]
    async fn test_filter_pool_fails_quote_allowlist() {
        let config = DiscoveryConfig {
            enable_pumpswap: true,
            pumpswap_program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
            quote_allowlist: vec!["So11111111111111111111111111111111111111112".to_string()],
            min_quote_liquidity: 1000.0,
            max_pools: 100,
            rescan_interval_secs: 300,
        };

        let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
            .unwrap();

        let pool_address = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique(); // Random mint not in allowlist

        let account_data = create_mock_pumpswap_account(base_mint, quote_mint, 1_000_000, 2000);

        let result = discovery.filter_pool(&pool_address, &account_data).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_filter_pool_fails_min_liquidity() {
        let config = DiscoveryConfig {
            enable_pumpswap: true,
            pumpswap_program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
            quote_allowlist: vec!["So11111111111111111111111111111111111111112".to_string()],
            min_quote_liquidity: 10000.0,
            max_pools: 100,
            rescan_interval_secs: 300,
        };

        let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
            .unwrap();

        let pool_address = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

        let account_data = create_mock_pumpswap_account(base_mint, quote_mint, 1_000_000, 500);

        let result = discovery.filter_pool(&pool_address, &account_data).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_discovered_tracking() {
        let config = DiscoveryConfig::default();
        let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
            .unwrap();

        let pool_address = Pubkey::new_unique();

        assert!(!discovery.is_discovered(&pool_address).await);
        assert_eq!(discovery.discovered_count().await, 0);

        discovery.mark_discovered(pool_address).await;

        assert!(discovery.is_discovered(&pool_address).await);
        assert_eq!(discovery.discovered_count().await, 1);
    }
}
