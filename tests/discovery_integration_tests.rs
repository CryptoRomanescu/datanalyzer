/// Integration tests for pool discovery functionality
use datanalyzer::config::DiscoveryConfig;
use datanalyzer::dex::pumpswap::PumpSwapDecoder;
use datanalyzer::discovery::PoolDiscovery;
use datanalyzer::models::DexType;
use solana_sdk::pubkey::Pubkey;

#[tokio::test]
async fn test_discovery_config_defaults() {
    let config = DiscoveryConfig::default();
    
    assert_eq!(config.enable_pumpswap, false);
    assert_eq!(config.pumpswap_program_id, "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");
    assert_eq!(config.quote_allowlist.len(), 3); // USDC, USDT, SOL
    assert_eq!(config.min_quote_liquidity, 1000.0);
    assert_eq!(config.max_pools, 2000);
    assert_eq!(config.rescan_interval_secs, 300);
}

#[tokio::test]
async fn test_pool_discovery_creation() {
    let config = DiscoveryConfig {
        enable_pumpswap: true,
        pumpswap_program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
        quote_allowlist: vec!["So11111111111111111111111111111111111111112".to_string()],
        min_quote_liquidity: 1000.0,
        max_pools: 100,
        rescan_interval_secs: 300,
    };

    let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string());
    assert!(discovery.is_ok());
}

#[tokio::test]
async fn test_pool_discovery_invalid_program_id() {
    let config = DiscoveryConfig {
        enable_pumpswap: true,
        pumpswap_program_id: "invalid_program_id".to_string(),
        quote_allowlist: vec!["So11111111111111111111111111111111111111112".to_string()],
        min_quote_liquidity: 1000.0,
        max_pools: 100,
        rescan_interval_secs: 300,
    };

    // This should succeed in creation, but fail in backfill
    let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string());
    assert!(discovery.is_ok());
}

#[tokio::test]
async fn test_pool_discovery_invalid_quote_mint() {
    let config = DiscoveryConfig {
        enable_pumpswap: true,
        pumpswap_program_id: "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
        quote_allowlist: vec!["invalid_mint".to_string()],
        min_quote_liquidity: 1000.0,
        max_pools: 100,
        rescan_interval_secs: 300,
    };

    let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string());
    assert!(discovery.is_err());
}

#[tokio::test]
async fn test_pool_discovery_tracking() {
    let config = DiscoveryConfig::default();
    let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
        .unwrap();

    let pool1 = Pubkey::new_unique();
    let pool2 = Pubkey::new_unique();

    assert_eq!(discovery.discovered_count().await, 0);
    assert!(!discovery.is_discovered(&pool1).await);

    discovery.mark_discovered(pool1).await;
    assert_eq!(discovery.discovered_count().await, 1);
    assert!(discovery.is_discovered(&pool1).await);
    assert!(!discovery.is_discovered(&pool2).await);

    discovery.mark_discovered(pool2).await;
    assert_eq!(discovery.discovered_count().await, 2);
    assert!(discovery.is_discovered(&pool2).await);
}

#[tokio::test]
async fn test_pumpswap_decoder_integration() {
    use std::str::FromStr;
    
    // Test that PumpSwapDecoder can extract mints correctly
    let base_mint = Pubkey::new_unique();
    let quote_mint = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
    
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
    let base_reserve = 1_000_000_000u64;
    let quote_reserve = 50_000_000_000u64;
    data[PumpSwapDecoder::BASE_RESERVE_OFFSET..PumpSwapDecoder::BASE_RESERVE_OFFSET + 8]
        .copy_from_slice(&base_reserve.to_le_bytes());
    data[PumpSwapDecoder::QUOTE_RESERVE_OFFSET..PumpSwapDecoder::QUOTE_RESERVE_OFFSET + 8]
        .copy_from_slice(&quote_reserve.to_le_bytes());
    
    // Test extraction
    let extracted_base = PumpSwapDecoder::extract_base_mint(&data).unwrap();
    let extracted_quote = PumpSwapDecoder::extract_quote_mint(&data).unwrap();
    
    assert_eq!(extracted_base, base_mint);
    assert_eq!(extracted_quote, quote_mint);
}

#[test]
fn test_pumpswap_dex_type() {
    use std::str::FromStr;
    
    let dex_type = DexType::from_str("pumpswap").unwrap();
    assert_eq!(dex_type, DexType::PumpSwap);
    
    let dex_type = DexType::from_str("pump_swap").unwrap();
    assert_eq!(dex_type, DexType::PumpSwap);
    
    assert_eq!(dex_type.to_string(), "PumpSwap");
    assert_eq!(dex_type.get_account_size(), 324);
}

// Note: Actual backfill tests would require a live RPC connection
// These are placeholder tests that verify the structure is correct
#[tokio::test]
async fn test_discovery_disabled_skips_backfill() {
    let config = DiscoveryConfig {
        enable_pumpswap: false,
        ..Default::default()
    };

    let discovery = PoolDiscovery::new(config, "https://api.mainnet-beta.solana.com".to_string())
        .unwrap();

    let result = discovery.backfill_pumpswap_pools().await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 0); // Should return empty when disabled
}
