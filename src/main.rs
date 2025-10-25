#![allow(dead_code)]

use std::env;
use std::error::Error;
use std::sync::Arc;

use datanalyzer::config::AppConfig;
use datanalyzer::price_fetcher::PriceFetcher;
use datanalyzer::websocket::{AccountUpdateCallback, WebSocketManager};

use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;

mod orchestrator;
use orchestrator::{Orchestrator, PoolUpdate};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    log::info!("Datanalyzer (production) starting...");

    // Config path: --config <path> | DATANALYZER_CONFIG | ./config.toml
    let args: Vec<String> = env::args().collect();
    let mut config_path = env::var("DATANALYZER_CONFIG").unwrap_or_else(|_| "config.toml".to_string());
    if args.len() >= 3 && args[1] == "--config" {
        config_path = args[2].clone();
    }
    log::info!("Loading config from: {}", &config_path);

    let app_cfg = AppConfig::load(&config_path)?;
    let runtime_cfg = app_cfg.into_runtime_config()?;

    // Price fetcher (TTL można przenieść do configu)
    let price_fetcher = Arc::new(PriceFetcher::new(std::time::Duration::from_secs(300)));

    // Orchestrator: RPC (HTTP), compute, CSV; przekazujemy mapę mint->coingecko z configu
    let orch = Arc::new(Orchestrator::new(
        runtime_cfg.rpc_url.clone(),
        Arc::clone(&price_fetcher),
        &runtime_cfg.output_dir,
        &runtime_cfg.pools,
        runtime_cfg.mint_map.clone(),
    ));

    // WebSocket + połączenie
    let mut ws = WebSocketManager::new(runtime_cfg.rpc_ws_url.clone(), runtime_cfg.snapshot_interval_ms);
    if let Err(e) = ws.connect().await {
        log::warn!("WS connect failed: {}. Retrying with reconnect loop...", e);
        if let Err(e2) = ws.reconnect_loop(Some(3)).await {
            return Err(e2.into());
        }
    }

    // Kolejka aktualizacji i uruchomienie orchestratora z workerami
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

    // Callback: push do kolejki
    let tx_cb = tx.clone();
    let callback: AccountUpdateCallback = Arc::new(move |pool: Pubkey, data: Vec<u8>, slot: u64| {
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

    // Start nasłuchu
    let pools: Vec<Pubkey> = runtime_cfg.pools.iter().map(|p| *p.pool_address()).collect();
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
