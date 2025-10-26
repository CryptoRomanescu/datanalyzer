use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::PoolConfig;
use crate::csv_writer::{CsvWriter, CsvWriterConfig};
use crate::dex::{pumpfun::PumpFunDecoder, pumpswap::PumpSwapDecoder, raydium::RaydiumDecoder, DexDecoder};
use crate::error::AppError;
use crate::models::{DexType, PoolSnapshot};
use crate::oracle::Oracle;
use crate::token_metadata::TokenMetadataProvider;

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
use spl_token::state::Account as SplTokenAccount;

use tokio::sync::{mpsc, Mutex};

/// Zdarzenie do przetworzenia (dostarczane z WebSocket callback)
#[derive(Debug, Clone)]
pub struct PoolUpdate {
    pub pool: Pubkey,
    pub slot: u64,
    pub account_data: Vec<u8>,
}

/// Wynikowe dane do zapisu (snapshot)
#[derive(Debug)]
struct SnapshotRecord {
    pub snapshot: PoolSnapshot,
    pub dex_type: DexType,
}

pub struct Orchestrator {
    rpc: Arc<RpcClient>,
    oracle: Arc<dyn Oracle>,
    metadata_provider: Arc<TokenMetadataProvider>,
    out_dir: PathBuf,
    csv_config: CsvWriterConfig,

    // Metadane konfiguracji
    pool_types: Arc<Mutex<HashMap<Pubkey, DexType>>>,
    pool_token_mints: Arc<Mutex<HashMap<Pubkey, Pubkey>>>,

    // Writery CSV per-pool
    writers: Arc<Mutex<HashMap<Pubkey, CsvWriter>>>,
}

impl Orchestrator {
    pub fn new(
        rpc_http_url: String,
        oracle: Arc<dyn Oracle>,
        metadata_provider: Arc<TokenMetadataProvider>,
        out_dir: impl AsRef<Path>,
        pools: &[PoolConfig],
        csv_config: CsvWriterConfig,
    ) -> Self {
        let rpc = Arc::new(RpcClient::new(rpc_http_url));

        let mut pool_types = HashMap::new();
        let mut pool_token_mints = HashMap::new();

        for p in pools {
            pool_types.insert(*p.pool_address(), p.dex_type());
            pool_token_mints.insert(*p.pool_address(), *p.token_mint());
        }

        Self {
            rpc,
            oracle,
            metadata_provider,
            out_dir: out_dir.as_ref().to_path_buf(),
            csv_config,
            pool_types: Arc::new(Mutex::new(pool_types)),
            pool_token_mints: Arc::new(Mutex::new(pool_token_mints)),
            writers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new pool dynamically (for discovery)
    pub async fn register_pool(&self, pool_config: PoolConfig) -> Result<(), AppError> {
        let pool_address = *pool_config.pool_address();
        let dex_type = pool_config.dex_type();
        let token_mint = *pool_config.token_mint();

        let mut pool_types = self.pool_types.lock().await;
        let mut pool_token_mints = self.pool_token_mints.lock().await;

        pool_types.insert(pool_address, dex_type);
        pool_token_mints.insert(pool_address, token_mint);

        log::info!(
            "Registered new pool: {} (type: {}, mint: {})",
            pool_address,
            dex_type,
            token_mint
        );

        Ok(())
    }

    /// Get the number of registered pools
    pub async fn pool_count(&self) -> usize {
        self.pool_types.lock().await.len()
    }

    /// Uruchamia pętlę przetwarzania z pulą workerów i osobnym workerem zapisu CSV.
    pub async fn run(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<PoolUpdate>,
        _workers: usize,
    ) -> Result<(), AppError> {
        let (tx_snap, mut rx_snap) = mpsc::channel::<SnapshotRecord>(1024);

        // Worker zapisu CSV (single-threaded, aby uniknąć konfliktów plików)
        let me = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(rec) = rx_snap.recv().await {
                if let Err(e) = me.write_snapshot(rec).await {
                    log::error!("Write error: {}", e);
                }
            }
        });

        // Główny receiver — przetwarzaj aktualizacje
        while let Some(update) = rx.recv().await {
            if let Err(e) = self.handle_update(update, &tx_snap).await {
                log::debug!("Compute error: {}", e);
            }
        }

        Ok(())
    }

    async fn handle_update(
        &self,
        update: PoolUpdate,
        tx_snap: &mpsc::Sender<SnapshotRecord>,
    ) -> Result<(), AppError> {
        let pool_types = self.pool_types.lock().await;
        let dex = pool_types.get(&update.pool).ok_or_else(|| {
            AppError::DecodingError(format!("Unknown pool type for {}", update.pool))
        })?;
        let dex = *dex; // Copy the enum value
        drop(pool_types); // Release lock early

        match dex {
            DexType::PumpFun => {
                let decoder = PumpFunDecoder;
                decoder.validate_account(&update.account_data)?;
                let (reserve_base, reserve_quote) =
                    decoder.decode_reserves(&update.account_data)?;

                // Get mint addresses from pool data (Pump.fun specific)
                // For Pump.fun, we need to decode the bonding curve to get quote_mint
                // For simplicity, assume quote is always SOL for Pump.fun
                let pool_token_mints = self.pool_token_mints.lock().await;
                let base_mint = pool_token_mints
                    .get(&update.pool)
                    .cloned()
                    .unwrap_or_default();
                drop(pool_token_mints);
                let quote_mint = "So11111111111111111111111111111111111111112"; // SOL

                // Fetch decimals
                let base_decimals = self
                    .metadata_provider
                    .get_decimals(&base_mint.to_string())
                    .await
                    .unwrap_or(9);
                let quote_decimals = 9u8; // SOL always has 9 decimals

                // Compute price: price_base_in_quote = (quote_amount / 10^dec_quote) / (base_amount / 10^dec_base)
                let price = if reserve_base > 0 {
                    let quote_normalized =
                        reserve_quote as f64 / 10_f64.powi(quote_decimals as i32);
                    let base_normalized = reserve_base as f64 / 10_f64.powi(base_decimals as i32);
                    quote_normalized / base_normalized
                } else {
                    0.0
                };

                // Compute liquidity_usd using Oracle
                let liquidity_usd = self
                    .compute_liquidity_usd(
                        reserve_base,
                        reserve_quote,
                        base_decimals,
                        quote_decimals,
                        &base_mint.to_string(),
                        quote_mint,
                    )
                    .await
                    .unwrap_or(0.0);

                let snapshot = PoolSnapshot::with_liquidity(
                    update.pool.to_string(),
                    base_mint.to_string(),
                    dex,
                    reserve_base,
                    reserve_quote,
                    chrono::Utc::now().timestamp(),
                    price,
                    liquidity_usd,
                )?;

                tx_snap
                    .send(SnapshotRecord {
                        snapshot,
                        dex_type: dex,
                    })
                    .await
                    .map_err(|e| AppError::CsvError(format!("Send snapshot error: {}", e)))?;
            }

            DexType::PumpSwap => {
                let decoder = PumpSwapDecoder;
                decoder.validate_account(&update.account_data)?;
                let (reserve_base, reserve_quote) =
                    decoder.decode_reserves(&update.account_data)?;

                // Extract mint addresses from pool data
                let base_mint = PumpSwapDecoder::extract_base_mint(&update.account_data)?;
                let quote_mint = PumpSwapDecoder::extract_quote_mint(&update.account_data)?;

                // Fetch decimals
                let base_decimals = self
                    .metadata_provider
                    .get_decimals(&base_mint.to_string())
                    .await
                    .unwrap_or(9);
                let quote_decimals = self
                    .metadata_provider
                    .get_decimals(&quote_mint.to_string())
                    .await
                    .unwrap_or(9);

                // Compute price: price_base_in_quote = (quote_amount / 10^dec_quote) / (base_amount / 10^dec_base)
                let price = if reserve_base > 0 {
                    let quote_normalized =
                        reserve_quote as f64 / 10_f64.powi(quote_decimals as i32);
                    let base_normalized = reserve_base as f64 / 10_f64.powi(base_decimals as i32);
                    quote_normalized / base_normalized
                } else {
                    0.0
                };

                // Compute liquidity_usd using Oracle
                let liquidity_usd = self
                    .compute_liquidity_usd(
                        reserve_base,
                        reserve_quote,
                        base_decimals,
                        quote_decimals,
                        &base_mint.to_string(),
                        &quote_mint.to_string(),
                    )
                    .await
                    .unwrap_or(0.0);

                let snapshot = PoolSnapshot::with_liquidity(
                    update.pool.to_string(),
                    base_mint.to_string(),
                    dex,
                    reserve_base,
                    reserve_quote,
                    chrono::Utc::now().timestamp(),
                    price,
                    liquidity_usd,
                )?;

                tx_snap
                    .send(SnapshotRecord {
                        snapshot,
                        dex_type: dex,
                    })
                    .await
                    .map_err(|e| AppError::CsvError(format!("Send snapshot error: {}", e)))?;
            }

            DexType::Raydium => {
                // 1) Extract vault info from AmmInfo
                let decoder = RaydiumDecoder;
                decoder.validate_account(&update.account_data)?;
                let vault_info = decoder.get_vault_info(&update.account_data)?;

                // 2) Fetch vault SPL Token Account data via RPC
                let coin_data = self
                    .rpc
                    .get_account_data(&vault_info.coin_vault)
                    .await
                    .map_err(|e| {
                        AppError::RpcError(format!("get_account_data coin_vault: {}", e))
                    })?;
                let pc_data = self
                    .rpc
                    .get_account_data(&vault_info.pc_vault)
                    .await
                    .map_err(|e| AppError::RpcError(format!("get_account_data pc_vault: {}", e)))?;

                // 3) Parse SPL Token Account and extract amounts
                let coin_acc = SplTokenAccount::unpack(&coin_data)
                    .map_err(|e| AppError::DecodingError(format!("SPL unpack coin: {}", e)))?;
                let pc_acc = SplTokenAccount::unpack(&pc_data)
                    .map_err(|e| AppError::DecodingError(format!("SPL unpack pc: {}", e)))?;

                let reserve_base = coin_acc.amount; // Raydium: coin_vault = base
                let reserve_quote = pc_acc.amount; // pc_vault = quote (usually SOL or USDC)

                // Get mint addresses for decimal lookup
                let base_mint = vault_info.coin_mint.to_string();
                let quote_mint = vault_info.pc_mint.to_string();

                // Fetch decimals from on-chain with caching
                let base_decimals = self
                    .metadata_provider
                    .get_decimals(&base_mint)
                    .await
                    .unwrap_or(9);
                let quote_decimals = self
                    .metadata_provider
                    .get_decimals(&quote_mint)
                    .await
                    .unwrap_or(9);

                // Compute price: price_base_in_quote = (quote_amount / 10^dec_quote) / (base_amount / 10^dec_base)
                let price = if reserve_base > 0 {
                    let quote_normalized =
                        reserve_quote as f64 / 10_f64.powi(quote_decimals as i32);
                    let base_normalized = reserve_base as f64 / 10_f64.powi(base_decimals as i32);
                    quote_normalized / base_normalized
                } else {
                    0.0
                };

                // Compute liquidity_usd using Oracle
                let liquidity_usd = self
                    .compute_liquidity_usd(
                        reserve_base,
                        reserve_quote,
                        base_decimals,
                        quote_decimals,
                        &base_mint,
                        &quote_mint,
                    )
                    .await
                    .unwrap_or(0.0);

                let pool_token_mints = self.pool_token_mints.lock().await;
                let token_mint = pool_token_mints
                    .get(&update.pool)
                    .cloned()
                    .unwrap_or_default();
                drop(pool_token_mints);

                let snapshot = PoolSnapshot::with_liquidity(
                    update.pool.to_string(),
                    token_mint.to_string(),
                    dex,
                    reserve_base,
                    reserve_quote,
                    chrono::Utc::now().timestamp(),
                    price,
                    liquidity_usd,
                )?;

                tx_snap
                    .send(SnapshotRecord {
                        snapshot,
                        dex_type: dex,
                    })
                    .await
                    .map_err(|e| AppError::CsvError(format!("Send snapshot error: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Compute liquidity in USD using the Oracle
    ///
    /// This method:
    /// 1. Checks if quote mint is a stable coin (returns 1.0)
    /// 2. Otherwise queries Oracle for quote→USD price
    /// 3. Computes liquidity as: base_value_usd + quote_value_usd
    async fn compute_liquidity_usd(
        &self,
        reserve_base: u64,
        reserve_quote: u64,
        base_decimals: u8,
        quote_decimals: u8,
        _base_mint: &str,
        quote_mint: &str,
    ) -> Result<f64, AppError> {
        // Get quote price in USD via Oracle
        let quote_price_usd = self.oracle.fetch_price_usd(quote_mint).await?;

        // Normalize reserves to actual token amounts
        let quote_amount = reserve_quote as f64 / 10_f64.powi(quote_decimals as i32);
        let _base_amount = reserve_base as f64 / 10_f64.powi(base_decimals as i32);

        // Compute quote value in USD
        let quote_value_usd = quote_amount * quote_price_usd;

        // Compute base value using the price already calculated
        // price = quote_amount / base_amount
        // base_in_quote_terms = base_amount * price = quote_amount
        // So total liquidity = 2 * quote_value_usd
        let base_value_usd = quote_value_usd; // Because in AMM pools, value is balanced

        let total_liquidity = base_value_usd + quote_value_usd;

        Ok(total_liquidity)
    }

    async fn write_snapshot(&self, rec: SnapshotRecord) -> Result<(), AppError> {
        let pool_pk = rec
            .snapshot
            .pool_address
            .parse::<Pubkey>()
            .map_err(|e| AppError::DecodingError(format!("Invalid pool pubkey: {}", e)))?;

        let mut writers = self.writers.lock().await;

        // Get or create writer for this pool
        if let std::collections::hash_map::Entry::Vacant(e) = writers.entry(pool_pk) {
            let filename = format!(
                "{}_{}.csv",
                rec.dex_type.to_string().to_lowercase(),
                &pool_pk.to_string()[..8]
            );
            let path = self.out_dir.join(filename);

            // Headers that match PoolSnapshot::to_csv_row()
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

            let writer = CsvWriter::with_config(&path, headers, self.csv_config.clone())?;
            e.insert(writer);
        }

        // Write the record
        let writer = writers
            .get_mut(&pool_pk)
            .ok_or_else(|| AppError::CsvError("Writer not found after creation".to_string()))?;

        let csv_row = rec.snapshot.to_csv_row();
        writer.write_record(&csv_row)?;

        Ok(())
    }
}
