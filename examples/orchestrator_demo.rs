use datanalyzer::dex::pumpfun::PumpFunDecoder;
/// Example demonstrating the Raydium orchestrator usage.
///
/// This example shows how to:
/// 1. Decode Raydium AmmInfo to get vault information
/// 2. Use the orchestrator to fetch actual reserves from vault accounts
/// 3. Handle both direct (Pump.fun) and vault-based (Raydium) reserves
use datanalyzer::dex::raydium::{AmmInfo, RaydiumDecoder};
use datanalyzer::orchestrator::{ReserveInfo, ReserveOrchestrator};
use solana_sdk::pubkey::Pubkey;

fn main() {
    println!("=== Raydium Orchestrator Example ===\n");

    // Example 1: Raydium Pool (requires vault fetching)
    example_raydium_pool();

    // Example 2: Pump.fun Pool (direct reserves)
    example_pumpfun_pool();

    // Example 3: Handling both types uniformly
    example_unified_handling();
}

fn example_raydium_pool() {
    println!("Example 1: Raydium Pool\n");

    // Create a mock Raydium AmmInfo for demonstration
    let mut amm_info = AmmInfo::default();
    amm_info.status = 1; // Initialized
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

    // Convert to bytes (as would come from RPC)
    let mut account_data = vec![0u8; 752];
    let amm_bytes = bytemuck::bytes_of(&amm_info);
    account_data.copy_from_slice(amm_bytes);

    // Step 1: Decode AmmInfo to get ReserveInfo
    let decoder = RaydiumDecoder;
    match decoder.decode_reserve_info(&account_data) {
        Ok(reserve_info) => {
            println!("✓ Decoded AmmInfo successfully");

            match reserve_info {
                ReserveInfo::RequiresVaults(vault_info) => {
                    println!("  Coin vault: {}", vault_info.coin_vault);
                    println!("  PC vault: {}", vault_info.pc_vault);
                    println!("  Coin mint: {}", vault_info.coin_mint);
                    println!("  PC mint: {}", vault_info.pc_mint);

                    // Step 2: Use orchestrator to fetch reserves
                    // Note: In this example, RPC would fail since these are mock accounts
                    println!("\n  To fetch actual reserves, use:");
                    println!("    let orchestrator = ReserveOrchestrator::new(rpc_url);");
                    println!(
                        "    let (base, quote) = orchestrator.resolve_reserves(&reserve_info)?;"
                    );
                }
                _ => println!("✗ Unexpected reserve info type"),
            }
        }
        Err(e) => println!("✗ Failed to decode: {}", e),
    }

    println!("\n{}\n", "=".repeat(50));
}

fn example_pumpfun_pool() {
    println!("Example 2: Pump.fun Pool\n");

    // Create mock Pump.fun account data
    let mut account_data = vec![0u8; 256];
    let token_reserve = 1_000_000_000u64; // 1 billion tokens
    let sol_reserve = 500_000_000u64; // 0.5 SOL in lamports

    // Set reserves at expected offsets
    account_data[0x18..0x20].copy_from_slice(&token_reserve.to_le_bytes());
    account_data[0x20..0x28].copy_from_slice(&sol_reserve.to_le_bytes());

    // Decode to ReserveInfo
    let decoder = PumpFunDecoder;
    match decoder.decode_reserve_info(&account_data) {
        Ok(reserve_info) => {
            println!("✓ Decoded Pump.fun account successfully");

            match reserve_info {
                ReserveInfo::Direct { base, quote } => {
                    println!("  Token reserve: {}", base);
                    println!("  SOL reserve: {} lamports", quote);

                    // For direct reserves, orchestrator returns them immediately
                    let orchestrator =
                        ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());
                    match orchestrator.resolve_reserves(&reserve_info) {
                        Ok((b, q)) => {
                            println!("\n✓ Resolved reserves:");
                            println!("  Base: {}", b);
                            println!("  Quote: {}", q);
                        }
                        Err(e) => println!("✗ Failed to resolve: {}", e),
                    }
                }
                _ => println!("✗ Unexpected reserve info type"),
            }
        }
        Err(e) => println!("✗ Failed to decode: {}", e),
    }

    println!("\n{}\n", "=".repeat(50));
}

fn example_unified_handling() {
    println!("Example 3: Unified Reserve Handling\n");

    let orchestrator = ReserveOrchestrator::new("https://api.mainnet-beta.solana.com".to_string());

    // This function works for both DEX types
    fn process_reserves(reserve_info: &ReserveInfo, orchestrator: &ReserveOrchestrator) {
        match orchestrator.resolve_reserves(reserve_info) {
            Ok((base, quote)) => {
                println!("  ✓ Base reserve: {}", base);
                println!("  ✓ Quote reserve: {}", quote);
            }
            Err(e) => {
                println!("  ✗ Error: {}", e);
            }
        }
    }

    // Test with direct reserves
    println!("Processing direct reserves (Pump.fun):");
    let direct = ReserveInfo::Direct {
        base: 1000,
        quote: 2000,
    };
    process_reserves(&direct, &orchestrator);

    // Test with vault-based reserves (would need real RPC in production)
    println!("\nProcessing vault-based reserves (Raydium):");
    println!("  (Would require real vault accounts and RPC in production)");

    println!("\n{}\n", "=".repeat(50));
}
