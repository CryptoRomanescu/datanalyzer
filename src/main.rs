#![allow(dead_code)]

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use datanalyzer::config::AppConfig;
use datanalyzer::discovery::PoolDiscovery;
use datanalyzer::oracle::{JupiterQuoteOracle, OracleConfig};
use datanalyzer::orchestrator::{Orchestrator, PoolUpdate};
use datanalyzer::token_metadata::TokenMetadataProvider;
use datanalyzer::websocket::{AccountUpdateCallback, WebSocketManager};

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;

/// Timeout for initial RPC backfill requests (in seconds)
const BACKFILL_RPC_TIMEOUT_SECS: u64 = 30;

/// Parse config path from command line arguments
fn parse_config_path(args: &[String]) -> String {
    let mut config_path = env::var("DATANALYZER_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    
    for i in 0..args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            config_path = args[i + 1].clone();
            break;
        }
    }
    
    config_path
}

/// Perform initial RPC backfill of pool accounts at startup.
/// 
/// Fetches all target pool accounts in one batch call and returns PoolUpdate
/// items for non-empty accounts. Logs details for each account.
async fn initial_backfill(rpc_url: &str, pools: &[Pubkey]) -> Vec<PoolUpdate> {
    if pools.is_empty() {
        log::info!("Initial backfill: no pools configured, skipping");
        return Vec::new();
    }

    log::info!("Initial backfill: fetching {} accounts from RPC ...", pools.len());
    
    // Create RPC client with timeout to prevent indefinite blocking
    let rpc_client = RpcClient::new_with_timeout(
        rpc_url.to_string(),
        Duration::from_secs(BACKFILL_RPC_TIMEOUT_SECS)
    );
    
    // Fetch all accounts in one batch call with confirmed commitment
    let accounts_result = rpc_client.get_multiple_accounts_with_commitment(
        pools,
        CommitmentConfig::confirmed(),
    ).await;
    
    let mut updates = Vec::new();
    
    match accounts_result {
        Ok(response) => {
            let slot = response.context.slot;
            
            for (i, account_opt) in response.value.into_iter().enumerate() {
                let pool_pubkey = pools[i];
                
                match account_opt {
                    Some(account) => {
                        let data_len = account.data.len();
                        
                        if data_len == 0 {
                            log::warn!(
                                "Backfill: account {} has empty data (slot {})",
                                pool_pubkey,
                                slot
                            );
                        } else {
                            log::info!(
                                "Backfill: account {} fetched: {} bytes, slot {}",
                                pool_pubkey,
                                data_len,
                                slot
                            );
                            
                            updates.push(PoolUpdate {
                                pool: pool_pubkey,
                                slot,
                                account_data: account.data,
                            });
                        }
                    }
                    None => {
                        log::warn!(
                            "Backfill: account {} not found (slot {})",
                            pool_pubkey,
                            slot
                        );
                    }
                }
            }
            
            log::info!(
                "Initial backfill complete: {} / {} accounts with data",
                updates.len(),
                pools.len()
            );
        }
        Err(e) => {
            log::error!("Initial backfill RPC request failed: {}", e);
            log::warn!("Continuing startup without initial backfill data. WebSocket updates will provide data once available.");
        }
    }
    
    updates
}

/// Demo mode for Issue 1: Core runtime and CSV pipeline
/// 
/// Demonstrates:
/// - Loading configuration from TOML
/// - Creating synthetic PoolSnapshot data  
/// - Writing to CSV with proper headers
/// - Clean exit after writing a few rows
async fn run_demo_mode(args: &[String]) -> Result<(), Box<dyn Error>> {
    use datanalyzer::csv_writer::CsvWriter;
    use datanalyzer::models::create_demo_snapshots;
    
    log::info!("Datanalyzer - Demo Mode (Issue 1)");
    
    // Parse config path
    let config_path = parse_config_path(args);
    
    log::info!("Loading configuration from: {}", config_path);
    
    // Load configuration
    let app_config = AppConfig::load(&config_path)?;
    let csv_config = app_config.csv;
    
    log::info!("Configuration loaded successfully");
    log::info!("Output directory: {}", app_config.output_dir);
    log::info!("CSV batch size: {}", csv_config.batch_size);
    
    // Create output directory
    std::fs::create_dir_all(&app_config.output_dir)?;
    
    // CSV file path
    let csv_path = format!("{}/demo_snapshots.csv", app_config.output_dir);
    
    // CSV headers matching PoolSnapshot::to_csv_row()
    let headers = &[
        "pool_address",
        "token_mint",
        "dex_type",
        "reserve_base",
        "reserve_quote",
        "timestamp",
        "price",
        "liquidity_usd",
    ];
    
    log::info!("Initializing CSV writer at: {}", csv_path);
    
    // Create CSV writer with configuration
    let writer_config = csv_config.to_csv_writer_config();
    let mut csv_writer = CsvWriter::with_config(&csv_path, headers, writer_config)?;
    
    log::info!("Writing synthetic pool snapshots...");
    
    // Generate synthetic snapshots using shared helper
    let snapshots = create_demo_snapshots()?;
    
    // Write each snapshot to CSV
    for (i, snapshot) in snapshots.iter().enumerate() {
        csv_writer.write_record(snapshot.to_csv_row())?;
        log::info!("Wrote snapshot {} to CSV", i + 1);
    }
    
    // Flush to ensure all data is written
    csv_writer.flush()?;
    
    log::info!("Successfully wrote {} snapshots to {}", snapshots.len(), csv_path);
    log::info!("Demo completed successfully");
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    
    // Check for --demo flag (Issue 1: minimal demonstration mode)
    let demo_mode = args.contains(&"--demo".to_string());
    
    if demo_mode {
        return run_demo_mode(&args).await;
    }
    
    log::info!("Datanalyzer (production) starting...");

    // Config path: --config <path> | DATANALYZER_CONFIG | ./config.toml
    let config_path = parse_config_path(&args);
    log::info!("Loading config from: {}", &config_path);

    let app_cfg = AppConfig::load(&config_path)?;
    let runtime_cfg = app_cfg.into_runtime_config()?;

    // Build Oracle from configuration
    let mut stable_mints = HashSet::new();
    for mint in &runtime_cfg.oracle.stable_mints {
        stable_mints.insert(mint.clone());
    }

    let oracle_config = OracleConfig {
        stable_mints,
        jupiter_url: runtime_cfg.oracle.jupiter_url.clone(),
        cache_ttl_secs: runtime_cfg.oracle.cache_ttl_secs,
    };
    let oracle = Arc::new(JupiterQuoteOracle::new(oracle_config));

    // Token metadata provider for decimal caching
    let metadata_provider = Arc::new(TokenMetadataProvider::new(
        runtime_cfg.rpc_url.clone(),
        Duration::from_secs(3600), // 1 hour TTL for decimals (they never change)
    ));

    // Convert CsvConfig to CsvWriterConfig
    let csv_writer_config = runtime_cfg.csv.to_csv_writer_config();

    // Orchestrator: RPC (HTTP), compute, CSV
    let orch = Arc::new(Orchestrator::new(
        runtime_cfg.rpc_url.clone(),
        oracle as Arc<dyn datanalyzer::oracle::Oracle>,
        Arc::clone(&metadata_provider),
        &runtime_cfg.output_dir,
        &runtime_cfg.pools,
        csv_writer_config,
    ));

    // WebSocket + connect
    let mut ws = WebSocketManager::new(
        runtime_cfg.rpc_ws_url.clone(),
        runtime_cfg.snapshot_interval_ms,
    );
    if let Err(e) = ws.connect().await {
        log::warn!("WS connect failed: {}. Retrying with reconnect loop...", e);
        if let Err(e2) = ws.reconnect_loop(Some(3)).await {
            return Err(e2.into());
        }
    }

    // Update queue and start orchestrator with workers
    let (tx, rx) = mpsc::channel::<PoolUpdate>(1024);
    let workers = 4usize;
    let orch_task = {
        let orch_clone = Arc::clone(&orch);
        tokio::spawn(async move {
            if let Err(e) = orch_clone.run(rx, workers).await {
                log::error!("Orchestrator stopped: {}", e);
            }
        })
    };

    // Build list of pool Pubkeys for initial backfill and WebSocket subscription
    let pools: Vec<Pubkey> = runtime_cfg
        .pools
        .iter()
        .map(|p| *p.pool_address())
        .collect();

    // Perform initial RPC backfill before Raydium resolver and WS listen.
    // Note: Raydium resolver only validates addresses; it does not modify them.
    // Pool addresses in runtime_cfg are already the correct ones to use for backfill.
    let backfilled_updates = initial_backfill(&runtime_cfg.rpc_url, &pools).await;
    
    // Enqueue all backfilled updates into the orchestrator queue
    let total_backfilled = backfilled_updates.len();
    let mut enqueued_count = 0;
    for update in backfilled_updates {
        if let Err(e) = tx.send(update).await {
            log::warn!("Failed to enqueue backfilled update: {}", e);
        } else {
            enqueued_count += 1;
        }
    }
    
    if total_backfilled > 0 {
        log::info!(
            "Enqueued {} / {} backfilled updates to orchestrator",
            enqueued_count,
            total_backfilled
        );
    }

    // Run discovery backfill if enabled
    if runtime_cfg.discovery.enable_pumpswap {
        log::info!("Starting PumpSwap pool discovery...");
        let discovery = match PoolDiscovery::new(
            runtime_cfg.discovery.clone(),
            runtime_cfg.rpc_url.clone(),
        ) {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to create discovery service: {}", e);
                return Err(e.into());
            }
        };

        // Backfill existing pools
        match discovery.backfill_pumpswap_pools().await {
            Ok(discovered_pools) => {
                log::info!(
                    "Discovery backfill found {} pools",
                    discovered_pools.len()
                );

                // Register discovered pools with orchestrator
                for pool_config in discovered_pools {
                    let pool_addr = *pool_config.pool_address();
                    
                    // Register with orchestrator
                    if let Err(e) = orch.register_pool(pool_config).await {
                        log::warn!("Failed to register pool {}: {}", pool_addr, e);
                        continue;
                    }

                    // Subscribe to pool updates
                    if let Err(e) = ws.subscribe_pool(pool_addr).await {
                        log::warn!("Failed to subscribe to pool {}: {}", pool_addr, e);
                    }
                }

                log::info!(
                    "Successfully subscribed to {} discovered pools",
                    discovery.discovered_count().await
                );
            }
            Err(e) => {
                log::error!("Discovery backfill failed: {}", e);
                // Continue anyway with manually configured pools
            }
        }
    }

    // Raydium pool address resolver (optional)
    if runtime_cfg.raydium_resolver.enabled {
        log::info!("Raydium pool address resolver is enabled, fetching pool data...");
        
        let resolver = datanalyzer::RaydiumResolver::with_config(
            runtime_cfg.raydium_resolver.api_url.clone(),
            runtime_cfg.raydium_resolver.timeout_secs,
        );
        
        match resolver.fetch_pool_data().await {
            Ok(()) => {
                let pool_count = resolver.pool_count().await;
                log::info!("✓ Raydium resolver loaded {} official pools", pool_count);
                
                // Validate pool addresses for Raydium pools
                for pool_cfg in &runtime_cfg.pools {
                    // Only validate Raydium pools
                    if pool_cfg.dex_type() != datanalyzer::DexType::Raydium {
                        continue;
                    }
                    
                    let current_addr = pool_cfg.pool_address().to_string();
                    match resolver.resolve(&current_addr).await {
                        Ok(Some(resolved_addr)) => {
                            if resolved_addr == current_addr {
                                log::info!("✓ Verified Raydium pool address: {}", current_addr);
                            } else {
                                log::warn!(
                                    "Pool address {} resolved to different address {}. Using configured address.",
                                    current_addr,
                                    resolved_addr
                                );
                            }
                        }
                        Ok(None) => {
                            log::warn!(
                                "⚠ Pool address {} not found in Raydium API. Proceeding anyway (may be a new pool).",
                                current_addr
                            );
                        }
                        Err(e) => {
                            log::debug!("Failed to resolve pool {}: {}", current_addr, e);
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Failed to fetch Raydium pool data: {}. Continuing with original addresses.",
                    e
                );
            }
        }
    } else {
        log::debug!("Raydium pool address resolver is disabled");
    }

    // Callback: push to queue
    let tx_cb = tx.clone();
    let callback: AccountUpdateCallback =
        Arc::new(move |pool: Pubkey, data: Vec<u8>, slot: u64| {
            let tx_inner = tx_cb.clone();
            let update = PoolUpdate {
                pool,
                slot,
                account_data: data,
            };
            tokio::spawn(async move {
                if let Err(e) = tx_inner.send(update).await {
                    log::warn!("Dropping update (queue full?): {}", e);
                }
            });
        });

    // Start listening (pools list already built earlier for backfill)
    ws.listen(&pools, callback, 30).await?;

    log::info!(
        "Subscribed {} pools. Output dir: {}. Press Ctrl+C to stop.",
        pools.len(),
        &runtime_cfg.output_dir
    );

    // Graceful shutdown
    tokio::signal::ctrl_c().await.ok();
    log::info!("Shutting down...");

    drop(tx);
    orch_task.abort();

    Ok(())
}
