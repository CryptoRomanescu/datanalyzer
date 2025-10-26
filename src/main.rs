#![allow(dead_code)]

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use datanalyzer::config::AppConfig;
use datanalyzer::oracle::{JupiterQuoteOracle, OracleConfig};
use datanalyzer::orchestrator::{Orchestrator, PoolUpdate};
use datanalyzer::token_metadata::TokenMetadataProvider;
use datanalyzer::websocket::{AccountUpdateCallback, WebSocketManager};

use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    log::info!("Datanalyzer (production) starting...");

    // Config path: --config <path> | DATANALYZER_CONFIG | ./config.toml
    let args: Vec<String> = env::args().collect();
    let mut config_path =
        env::var("DATANALYZER_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    if args.len() >= 3 && args[1] == "--config" {
        config_path = args[2].clone();
    }
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

    // Start listening
    let pools: Vec<Pubkey> = runtime_cfg
        .pools
        .iter()
        .map(|p| *p.pool_address())
        .collect();
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
