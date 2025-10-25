/// Example demonstrating the decoder registry and factory pattern.
///
/// This example shows how to:
/// 1. Create a decoder factory
/// 2. Use the decoder registry with multiple pools
/// 3. Lazy initialization
/// 4. Track and view statistics

use datanalyzer::config::PoolConfig;
use datanalyzer::dex::{create_decoder, DecoderRegistry};
use datanalyzer::models::DexType;
use solana_sdk::pubkey::Pubkey;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Decoder Factory Demo ===\n");

    // Example 1: Direct decoder usage via factory
    demo_factory()?;

    // Example 2: Registry with immediate initialization
    demo_registry_immediate()?;

    // Example 3: Registry with lazy initialization
    demo_registry_lazy()?;

    // Example 4: Statistics tracking
    demo_statistics()?;

    Ok(())
}

fn demo_factory() -> Result<(), Box<dyn std::error::Error>> {
    println!("1. FACTORY PATTERN");
    println!("-----------------");

    // Create a PumpFun decoder using the factory
    let pumpfun_decoder = create_decoder(DexType::PumpFun);
    println!("Created PumpFun decoder via factory");

    // Create a Raydium decoder using the factory
    let _raydium_decoder = create_decoder(DexType::Raydium);
    println!("Created Raydium decoder via factory");

    // Create sample PumpFun account data (256 bytes)
    let mut pumpfun_data = vec![0u8; 256];
    pumpfun_data[0x18..0x20].copy_from_slice(&1_000_000_000u64.to_le_bytes()); // Token reserve
    pumpfun_data[0x20..0x28].copy_from_slice(&50_000_000_000u64.to_le_bytes()); // SOL reserve

    // Decode the data
    let (token_reserve, sol_reserve) = pumpfun_decoder.decode_reserves(&pumpfun_data)?;
    println!(
        "Decoded PumpFun pool: {} tokens, {} lamports SOL\n",
        token_reserve, sol_reserve
    );

    Ok(())
}

fn demo_registry_immediate() -> Result<(), Box<dyn std::error::Error>> {
    println!("2. REGISTRY WITH IMMEDIATE INITIALIZATION");
    println!("-----------------------------------------");

    let registry = DecoderRegistry::new();

    // Register multiple pools (decoders created immediately)
    let pool1_addr = Pubkey::new_unique();
    let pool2_addr = Pubkey::new_unique();

    let pool1_config = PoolConfig::new(pool1_addr, DexType::PumpFun, Pubkey::new_unique())?;
    let pool2_config = PoolConfig::new(pool2_addr, DexType::Raydium, Pubkey::new_unique())?;

    registry.register_pool(pool1_config)?;
    registry.register_pool(pool2_config)?;

    println!("Registered {} pools", registry.pool_count());

    // Create sample data for pool1
    let mut pool1_data = vec![0u8; 256];
    pool1_data[0x18..0x20].copy_from_slice(&2_000_000_000u64.to_le_bytes());
    pool1_data[0x20..0x28].copy_from_slice(&100_000_000_000u64.to_le_bytes());

    // Decode using registry
    let (token, sol) = registry.decode_pool_data(&pool1_addr, &pool1_data)?;
    println!(
        "Decoded pool1: {} tokens, {} lamports SOL\n",
        token, sol
    );

    Ok(())
}

fn demo_registry_lazy() -> Result<(), Box<dyn std::error::Error>> {
    println!("3. REGISTRY WITH LAZY INITIALIZATION");
    println!("------------------------------------");

    let registry = DecoderRegistry::new();

    // Register pool type only (no decoder created yet)
    let pool_addr = Pubkey::new_unique();
    let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, Pubkey::new_unique())?;

    registry.register_pool_type(pool_config)?;
    println!("Registered pool type (no decoder created yet)");
    println!("Decoder count: {}", registry.pool_count());

    // Create sample data
    let mut data = vec![0u8; 256];
    data[0x18..0x20].copy_from_slice(&3_000_000_000u64.to_le_bytes());
    data[0x20..0x28].copy_from_slice(&150_000_000_000u64.to_le_bytes());

    // Decode with lazy initialization (decoder created on first use)
    let (token, sol) = registry.decode_pool_data_lazy(&pool_addr, &data)?;
    println!(
        "Decoded with lazy init: {} tokens, {} lamports SOL",
        token, sol
    );
    println!("Decoder count after lazy init: {}\n", registry.pool_count());

    Ok(())
}

fn demo_statistics() -> Result<(), Box<dyn std::error::Error>> {
    println!("4. STATISTICS TRACKING");
    println!("---------------------");

    let registry = DecoderRegistry::new();
    let pool_addr = Pubkey::new_unique();
    let pool_config = PoolConfig::new(pool_addr, DexType::PumpFun, Pubkey::new_unique())?;

    registry.register_pool(pool_config)?;

    // Create valid data
    let mut valid_data = vec![0u8; 256];
    valid_data[0x18..0x20].copy_from_slice(&1_000_000u64.to_le_bytes());
    valid_data[0x20..0x28].copy_from_slice(&2_000_000u64.to_le_bytes());

    // Create invalid data (too small)
    let invalid_data = vec![0u8; 50];

    // Perform some successful decodings
    for _ in 0..5 {
        let _ = registry.decode_pool_data(&pool_addr, &valid_data);
    }

    // Perform some failed decodings
    for _ in 0..2 {
        let _ = registry.decode_pool_data(&pool_addr, &invalid_data);
    }

    // Get and display statistics
    if let Some(stats) = registry.get_stats(&pool_addr) {
        println!("Pool: {}", pool_addr);
        println!("  Total attempts: {}", stats.total_attempts());
        println!("  Successful: {}", stats.success_count);
        println!("  Failed: {}", stats.error_count);
        println!("  Success rate: {:.2}%", stats.success_rate());
    }

    // Log all statistics
    println!("\nLogging all statistics:");
    registry.log_stats(true);

    Ok(())
}
