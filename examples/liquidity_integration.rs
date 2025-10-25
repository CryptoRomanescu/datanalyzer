/// Example demonstrating liquidity calculation integration.
///
/// This example shows how to:
/// 1. Extract token mint from pool config
/// 2. Fetch prices using PriceFetcher
/// 3. Calculate liquidity using calculate_liquidity_usd
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

/// Example processing function that integrates all components.
///
/// This demonstrates the main processing loop mentioned in Task 10.4.
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
    
    // Step 2: Get the wrapped SOL mint address
    // In production, this would be a constant or configuration value
    let sol_mint = "So11111111111111111111111111111111111111112"
        .parse::<Pubkey>()
        .map_err(|e| AppError::ConfigError(format!("Invalid SOL mint: {}", e)))?;
    
    // Step 3: Fetch token decimals (with caching)
    let token_decimals = price_fetcher.get_token_decimals(&token_mint).await?;
    
    println!("Token decimals for {}: {}", token_mint, token_decimals);
    
    // Step 4: Call price_fetcher.fetch_prices([SOL, token_mint])
    let mints_to_fetch = vec![sol_mint, token_mint];
    
    // Try to fetch prices, handle failure case
    let prices = match price_fetcher.fetch_prices(&mints_to_fetch).await {
        Ok(prices) => prices,
        Err(e) => {
            log::warn!("Failed to fetch prices: {}. Using fallback values.", e);
            // Handle case when fetch prices fails (use default values or skip USD calculation)
            let mut fallback_prices = HashMap::new();
            fallback_prices.insert(sol_mint, 0.0);
            fallback_prices.insert(token_mint, 0.0);
            fallback_prices
        }
    };
    
    // Extract individual prices
    let sol_price = prices.get(&sol_mint).copied().unwrap_or(0.0);
    let token_price = prices.get(&token_mint).copied().unwrap_or(0.0);
    
    println!("Fetched prices - SOL: ${}, Token: ${}", sol_price, token_price);
    
    // Step 5: Calculate liquidity USD
    // Important: DEX conventions vary, but typically:
    // - PumpFun: reserve_base = token, reserve_quote = SOL
    // - Raydium: reserve_base = coin (often token), reserve_quote = PC (often SOL)
    // The calculate_liquidity_usd function expects:
    //   (sol_reserves, token_reserves, sol_price, token_price, token_decimals)
    // So we need to identify which reserve is SOL and which is token
    
    let liquidity_usd = if sol_price > 0.0 || token_price > 0.0 {
        // For this example, we assume reserve_quote is SOL and reserve_base is token
        // In production, you would determine this based on the pool's mint configuration
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
    
    // Step 6: Calculate token price (for backward compatibility)
    // Price is typically quote/base (e.g., SOL per token)
    let price = if reserve_base > 0 {
        reserve_quote as f64 / reserve_base as f64
    } else {
        0.0
    };
    
    // Step 7: Create and save result in PoolSnapshot structure
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
    
    // Initialize PriceFetcher
    let price_fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
    
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
    
    // Show cache statistics
    println!("\n=== PriceFetcher Cache Statistics ===");
    println!("Cached decimals count: {}", price_fetcher.cache_size());
    
    Ok(())
}

/// Example showing error handling when price fetch fails
async fn example_error_handling() -> Result<(), AppError> {
    println!("\n=== Error Handling Example ===\n");
    
    let price_fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
    
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
    
    let price_fetcher = PriceFetcher::new("https://api.mainnet-beta.solana.com");
    
    let pool_config = PoolConfig::builder()
        .pool_address_pubkey(Pubkey::new_unique())
        .dex_type(DexType::PumpFun)
        .token_mint_pubkey(Pubkey::new_unique())
        .build()?;
    
    // Empty pool with zero reserves - this is valid per Task 10.3
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
