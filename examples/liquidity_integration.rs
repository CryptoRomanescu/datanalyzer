/// Example demonstrating liquidity calculation integration with CoinGecko price fetching.
///
/// This example shows how to:
/// 1. Create PriceFetcher with Duration-based TTL
/// 2. Use CoinGecko token IDs instead of Pubkey
/// 3. Fetch prices and calculate liquidity
/// 4. Handle errors when price fetching fails
/// 5. Store results in PoolSnapshot

use datanalyzer::{
    config::PoolConfig,
    error::AppError,
    liquidity::{calculate_liquidity_usd, check_liquidity_change},
    models::{DexType, PoolSnapshot},
    price_fetcher::PriceFetcher,
};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::time::Duration;

/// Example placeholder mapping from mint address to CoinGecko token ID
/// In production, this would be a more comprehensive mapping or lookup service
fn get_coingecko_id(mint: &Pubkey) -> Option<&'static str> {
    // This is a placeholder mapping. In production, you would maintain
    // a proper database or mapping service for mint -> token_id
    let mint_str = mint.to_string();
    
    // Example mappings (these are illustrative only)
    if mint_str == "So11111111111111111111111111111111111111112" {
        Some("solana")
    } else {
        // For other tokens, you'd need actual mappings
        // For this example, we'll use None to simulate unknown tokens
        None
    }
}

/// Example processing function that integrates all components.
async fn process_pool_with_liquidity(
    pool_config: &PoolConfig,
    reserve_base: u64,
    reserve_quote: u64,
    price_fetcher: &PriceFetcher,
    previous_liquidity: Option<f64>,
) -> Result<PoolSnapshot, AppError> {
    // Step 1: Extract token mint from pool config
    let token_mint = *pool_config.token_mint();
    let pool_address = pool_config.pool_address().to_string();
    
    // Step 2: Get CoinGecko token IDs
    let sol_token_id = "solana"; // SOL is always "solana" on CoinGecko
    let token_id = get_coingecko_id(&token_mint);
    
    println!("Token mint: {}", token_mint);
    
    // Step 3: Fetch prices using CoinGecko token IDs
    let mut token_ids = vec![sol_token_id.to_string()];
    if let Some(tid) = token_id {
        token_ids.push(tid.to_string());
    }
    
    // Try to fetch prices, handle failure case
    let prices = match price_fetcher.fetch_prices(&token_ids).await {
        Ok(prices) => prices,
        Err(e) => {
            log::warn!("Failed to fetch prices: {}. Using fallback values.", e);
            // Handle case when fetch prices fails (use default values or skip USD calculation)
            let mut fallback_prices = HashMap::new();
            fallback_prices.insert(sol_token_id.to_string(), 0.0);
            if let Some(tid) = token_id {
                fallback_prices.insert(tid.to_string(), 0.0);
            }
            fallback_prices
        }
    };
    
    // Extract individual prices
    let sol_price = prices.get(sol_token_id).copied().unwrap_or(0.0);
    let token_price = if let Some(tid) = token_id {
        prices.get(tid).copied().unwrap_or(0.0)
    } else {
        0.0
    };
    
    println!("Fetched prices - SOL: ${}, Token: ${}", sol_price, token_price);
    
    // Step 4: Calculate liquidity USD
    // Important: DEX conventions vary, but typically:
    // - PumpFun: reserve_base = token, reserve_quote = SOL
    // - Raydium: reserve_base = coin (often token), reserve_quote = PC (often SOL)
    // The calculate_liquidity_usd function expects:
    //   (sol_reserves, token_reserves, sol_price, token_price, token_decimals)
    // So we need to identify which reserve is SOL and which is token
    
    // For simplicity, assume 9 decimals (SOL standard)
    let token_decimals = 9u8;
    
    let liquidity_usd = if sol_price > 0.0 || token_price > 0.0 {
        // For this example, we assume reserve_quote is SOL and reserve_base is token
        let sol_reserves = reserve_quote;
        let token_reserves = reserve_base;
        
        match calculate_liquidity_usd(
            sol_reserves,      // SOL reserves in lamports
            token_reserves,    // Token reserves in smallest units
            sol_price,         // SOL price in USD
            token_price,       // Token price in USD
            token_decimals,    // Token decimals (SOL uses hardcoded 9 in function)
        ) {
            Ok(liquidity) => {
                println!("Calculated liquidity: ${:.2}", liquidity);
                
                // Check for drastic changes if we have previous liquidity
                if let Some(prev_liq) = previous_liquidity {
                    check_liquidity_change(prev_liq, liquidity, 50.0);  // 50% threshold
                }
                
                Some(liquidity)
            }
            Err(e) => {
                log::error!("Failed to calculate liquidity: {}", e);
                None
            }
        }
    } else {
        log::warn!("Prices are zero, skipping liquidity calculation");
        None
    };
    
    // Step 5: Calculate token price (for backward compatibility)
    // Price is typically quote/base (e.g., SOL per token)
    let price = if reserve_base > 0 {
        reserve_quote as f64 / reserve_base as f64
    } else {
        0.0
    };
    
    // Step 6: Create and save result in PoolSnapshot structure
    let snapshot = if let Some(liquidity) = liquidity_usd {
        PoolSnapshot::with_liquidity(
            pool_address,
            token_mint.to_string(),
            pool_config.dex_type(),
            reserve_base,
            reserve_quote,
            chrono::Utc::now().timestamp(),
            price,
            liquidity,
        )?
    } else {
        // If liquidity calculation failed, create snapshot without it
        PoolSnapshot::new(
            pool_address,
            token_mint.to_string(),
            pool_config.dex_type(),
            reserve_base,
            reserve_quote,
            chrono::Utc::now().timestamp(),
            price,
        )?
    };
    
    Ok(snapshot)
}

/// Example showing simplified integration in a typical processing loop
async fn example_processing_loop() -> Result<(), AppError> {
    println!("=== Liquidity Calculation Integration Example ===\n");
    
    // Initialize PriceFetcher with 5-minute TTL cache
    let price_fetcher = PriceFetcher::new(Duration::from_secs(300));
    
    // Create example pool configs
    let pool_configs = vec![
        PoolConfig::builder()
            .pool_address_pubkey(Pubkey::new_unique())
            .dex_type(DexType::PumpFun)
            .token_mint_pubkey(Pubkey::new_unique())
            .build()?,
        PoolConfig::builder()
            .pool_address_pubkey(Pubkey::new_unique())
            .dex_type(DexType::Raydium)
            .token_mint_pubkey(Pubkey::new_unique())
            .build()?,
    ];
    
    // Track previous liquidity values for change detection
    let mut previous_liquidity: HashMap<String, f64> = HashMap::new();
    
    // Process each pool
    for pool_config in &pool_configs {
        println!("\nProcessing pool: {}", pool_config.pool_address());
        println!("DEX Type: {}", pool_config.dex_type());
        
        // In real implementation, these would come from decoding account data
        let reserve_base = 1_000_000_000_000;   // 1000 tokens (with 9 decimals)
        let reserve_quote = 10_000_000_000;     // 10 SOL (with 9 decimals)
        
        // Get previous liquidity for this pool
        let prev_liq = previous_liquidity.get(&pool_config.pool_address().to_string()).copied();
        
        // Process the pool
        match process_pool_with_liquidity(
            pool_config,
            reserve_base,
            reserve_quote,
            &price_fetcher,
            prev_liq,
        ).await {
            Ok(snapshot) => {
                println!("✓ Successfully created snapshot");
                println!("  Pool: {}", snapshot.pool_address);
                println!("  Reserve Base: {}", snapshot.reserve_base);
                println!("  Reserve Quote: {}", snapshot.reserve_quote);
                if let Some(liquidity) = snapshot.liquidity_usd {
                    println!("  Liquidity USD: ${:.2}", liquidity);
                    // Store for next iteration
                    previous_liquidity.insert(snapshot.pool_address.clone(), liquidity);
                } else {
                    println!("  Liquidity USD: Not available");
                }
                
                // Example: Write to CSV
                let csv_row = snapshot.to_csv_row();
                println!("  CSV Row: {:?}", csv_row);
            }
            Err(e) => {
                log::error!("Failed to process pool: {}", e);
            }
        }
    }
    
    // Show metrics
    println!("\n=== PriceFetcher Metrics ===");
    let metrics = price_fetcher.get_metrics().await;
    println!("Total requests: {}", metrics.total_requests);
    println!("Successful requests: {}", metrics.successful_requests);
    println!("Failed requests: {}", metrics.failed_requests);
    println!("Success rate: {:.2}%", metrics.success_rate());
    println!("Avg response time: {:.2}ms", metrics.avg_response_time_ms());
    
    Ok(())
}

/// Example showing error handling when price fetch fails
async fn example_error_handling() -> Result<(), AppError> {
    println!("\n=== Error Handling Example ===\n");
    
    let price_fetcher = PriceFetcher::new(Duration::from_secs(300));
    
    let pool_config = PoolConfig::builder()
        .pool_address_pubkey(Pubkey::new_unique())
        .dex_type(DexType::PumpFun)
        .token_mint_pubkey(Pubkey::new_unique())
        .build()?;
    
    // Simulate a scenario where price fetch might fail
    // The function handles it gracefully by using fallback values
    let snapshot = process_pool_with_liquidity(
        &pool_config,
        1_000_000_000,  // 1 token
        1_000_000_000,  // 1 SOL
        &price_fetcher,
        None,
    ).await?;
    
    println!("Created snapshot even with potential price fetch issues");
    println!("Snapshot has liquidity: {}", snapshot.liquidity_usd.is_some());
    
    Ok(())
}

/// Example showing how to handle zero reserves (valid empty pool)
async fn example_zero_reserves() -> Result<(), AppError> {
    println!("\n=== Zero Reserves Example ===\n");
    
    let price_fetcher = PriceFetcher::new(Duration::from_secs(300));
    
    let pool_config = PoolConfig::builder()
        .pool_address_pubkey(Pubkey::new_unique())
        .dex_type(DexType::PumpFun)
        .token_mint_pubkey(Pubkey::new_unique())
        .build()?;
    
    // Empty pool with zero reserves - this is valid
    let snapshot = process_pool_with_liquidity(
        &pool_config,
        0,  // No tokens
        0,  // No SOL
        &price_fetcher,
        None,
    ).await?;
    
    println!("Empty pool handled correctly");
    if let Some(liquidity) = snapshot.liquidity_usd {
        println!("Liquidity for empty pool: ${:.2}", liquidity);
        assert_eq!(liquidity, 0.0, "Empty pool should have 0 liquidity");
    }
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    // Initialize logger
    env_logger::init();
    
    // Run examples
    example_processing_loop().await?;
    example_error_handling().await?;
    example_zero_reserves().await?;
    
    println!("\n=== All Examples Completed Successfully ===");
    
    Ok(())
}
