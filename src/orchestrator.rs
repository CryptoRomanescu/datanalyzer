use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::PoolConfig;
use crate::dex::{pumpfun::PumpFunDecoder, raydium::RaydiumDecoder, DexDecoder};
use crate::error::AppError;
use crate::liquidity::calculate_liquidity_usd;
use crate::models::{DexType, PoolSnapshot};
use crate::price_fetcher::PriceFetcher;
use crate::csv_writer::CsvWriter;

use solana_client::nonblocking::rpc_client::RpcClient;
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
    price_fetcher: Arc<PriceFetcher>,
    out_dir: PathBuf,

    // Metadane konfiguracji
    pool_types: HashMap<Pubkey, DexType>,
    pool_token_mints: HashMap<Pubkey, Pubkey>,

    // Opcjonalne mapowanie mint -> CoinGecko ID
    token_map: HashMap<Pubkey, String>,

    // Writery CSV per-pool
    writers: Arc<Mutex<HashMap<Pubkey, CsvWriter>>>,
}

impl Orchestrator {
    pub fn new(
        rpc_http_url: String,
        price_fetcher: Arc<PriceFetcher>,
        out_dir: impl AsRef<Path>,
        pools: &[PoolConfig],
        token_map: HashMap<Pubkey, String>,
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
            price_fetcher,
            out_dir: out_dir.as_ref().to_path_buf(),
            pool_types,
            pool_token_mints,
            token_map,
            writers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Uruchamia pętlę przetwarzania z pulą workerów i osobnym workerem zapisu CSV.
    pub async fn run(
        self: Arc<Self>,
        mut rx: mpsc::Receiver<PoolUpdate>,
        workers: usize,
    ) -> Result<(), AppError> {
        let (tx_snap, mut rx_snap) = mpsc::channel::<SnapshotRecord>(1024);

        // Workerzy compute
        for _ in 0..workers {
            let me = Arc::clone(&self);
            let mut rx_clone = rx.clone();
            let tx_snap_clone = tx_snap.clone();

            tokio::spawn(async move {
                while let Some(update) = rx_clone.recv().await {
                    if let Err(e) = me.handle_update(update, &tx_snap_clone).await {
                        log::debug!("Compute error: {}", e);
                    }
                }
            });
        }
        drop(tx_snap); // główny nadajnik niepotrzebny

        // Worker zapisu CSV (single-threaded, aby uniknąć konfliktów plików)
        let me = Arc::clone(&self);
        tokio::spawn(async move {
            while let Some(rec) = rx_snap.recv().await {
                if let Err(e) = me.write_snapshot(rec).await {
                    log::error!("Write error: {}", e);
                }
            }
        });

        // Główny receiver — niech żyje aż do zamknięcia kanału przez wywołującego
        while let Some(update) = rx.recv().await {
            // Jeżeli brak workerów, przetwarzaj inline
            if workers == 0 {
                if let Err(e) = self.handle_update(update, &rx_snap).await {
                    log::debug!("Inline compute error: {}", e);
                }
            } else {
                // W workerach już czeka rx_clone
            }
        }

        Ok(())
    }

    async fn handle_update(
        &self,
        update: PoolUpdate,
        tx_snap: &mpsc::Sender<SnapshotRecord>,
    ) -> Result<(), AppError> {
        let dex = self
            .pool_types
            .get(&update.pool)
            .ok_or_else(|| AppError::DecodingError(format!("Unknown pool type for {}", update.pool)))?;

        match dex {
            DexType::PumpFun => {
                let decoder = PumpFunDecoder;
                decoder.validate_account(&update.account_data)?;
                let (reserve_base, reserve_quote) = decoder.decode_reserves(&update.account_data)?;

                let price = if reserve_base > 0 {
                    reserve_quote as f64 / reserve_base as f64
                } else {
                    0.0
                };

                let snapshot = PoolSnapshot::new(
                    update.pool.to_string(),
                    self.pool_token_mints
                        .get(&update.pool)
                        .cloned()
                        .unwrap_or_default()
                        .to_string(),
                    *dex,
                    reserve_base,
                    reserve_quote,
                    chrono::Utc::now().timestamp(),
                    price,
                )?;

                tx_snap
                    .send(SnapshotRecord {
                        snapshot,
                        dex_type: *dex,
                    })
                    .await
                    .map_err(|e| AppError::CsvError(format!("Send snapshot error: {}", e)))?;
            }

            DexType::Raydium => {
                // 1) Wyciągnij vaulty z AmmInfo
                let decoder = RaydiumDecoder;
                decoder.validate_account(&update.account_data)?;
                let vault_info = decoder.get_vault_info(&update.account_data)?;

                // 2) Pobierz konta SPL vaultów przez HTTP RPC
                let coin_data = self
                    .rpc
                    .get_account_data(&vault_info.coin_vault)
                    .await
                    .map_err(|e| AppError::RpcError(format!("get_account_data coin_vault: {}", e)))?;
                let pc_data = self
                    .rpc
                    .get_account_data(&vault_info.pc_vault)
                    .await
                    .map_err(|e| AppError::RpcError(format!("get_account_data pc_vault: {}", e)))?;

                // 3) Parsuj SPL Token Account i wydobądź amount
                let coin_acc = SplTokenAccount::unpack(&coin_data)
                    .map_err(|e| AppError::DecodingError(format!("SPL unpack coin: {}", e)))?;
                let pc_acc = SplTokenAccount::unpack(&pc_data)
                    .map_err(|e| AppError::DecodingError(format!("SPL unpack pc: {}", e)))?;

                let reserve_base = coin_acc.amount; // Raydium: coin_vault = base
                let reserve_quote = pc_acc.amount; // pc_vault = quote (często SOL lub USDC)

                let price = if reserve_base > 0 {
                    reserve_quote as f64 / reserve_base as f64
                } else {
                    0.0
                };

                // Opcjonalnie: liquidity USD (wymaga mapowania tokenów)
                // Używamy "solana" dla SOL quote, a base wg token_map (jeśli istnieje)
                let token_mint = self
                    .pool_token_mints
                    .get(&update.pool)
                    .cloned()
                    .unwrap_or_default();

                let mut snapshot = PoolSnapshot::new(
                    update.pool.to_string(),
                    token_mint.to_string(),
                    *dex,
                    reserve_base,
                    reserve_quote,
                    chrono::Utc::now().timestamp(),
                    price,
                )?;

                // Jeśli masz mapowanie mint->coingecko id, możesz policzyć liquidity
                if let Some(token_id) = self.token_map.get(&token_mint) {
                    // Zakładamy PC=SOL i bierzemy "solana" jako quote
                    let prices = self
                        .price_fetcher
                        .fetch_prices(&vec!["solana".to_string(), token_id.clone()])
                        .await
                        .unwrap_or_default();

                    let sol_price = prices.get("solana").copied().unwrap_or(0.0);
                    let token_price = prices.get(token_id).copied().unwrap_or(0.0);

                    if sol_price > 0.0 || token_price > 0.0 {
                        // Decimals: SOL=9, token przyjmij 9 jako domyślne (lub dołóż provider decimali)
                        let token_decimals = 9u8;
                        if let Ok(liq) = calculate_liquidity_usd(
                            reserve_quote,
                            reserve_base,
                            sol_price,
                            token_price,
                            token_decimals,
                        ) {
                            snapshot = PoolSnapshot::with_liquidity(
                                snapshot.pool_address.clone(),
                                snapshot.token_mint.clone(),
                                snapshot.dex_type,
                                snapshot.reserve_base,
                                snapshot.reserve_quote,
                                snapshot.timestamp,
                                snapshot.price,
                                liq,
                            )?;
                        }
                    }
                }

                tx_snap
                    .send(SnapshotRecord {
                        snapshot,
                        dex_type: *dex,
                    })
                    .await
                    .map_err(|e| AppError::CsvError(format!("Send snapshot error: {}", e)))?;
            }
        }

        Ok(())
    }

    async fn writer_for(&self, pool: &Pubkey, dex: DexType) -> Result<CsvWriter, AppError> {
        let mut writers = self.writers.lock().await;
        if let Some(w) = writers.get(pool) {
            // Clippy wymaga klonowalnego writera; prostsze: przechowuj nowy za każdym razem
            // Tu implementujemy lazy open per pool.
        }

        let filename = format!("{}_{}.csv", dex.to_string().to_lowercase(), &pool.to_string()[..8]);
        let path = self.out_dir.join(filename);
        let mut writer = CsvWriter::new(&path)?;

        // Nagłówek tylko raz: CsvWriter.new ustawia brak nagłówków — tu dopisz raz nagłówek
        writer.write_header_if_needed()?;

        writers.insert(*pool, writer);
        // Niestety nie zwrócimy referencji — otwórzmy świeży writer do zapisu (append)
        CsvWriter::new(&path)
    }

    async fn write_snapshot(&self, rec: SnapshotRecord) -> Result<(), AppError> {
        let pool_pk = Pubkey::from_str(&rec.snapshot.pool_address)
            .map_err(|e| AppError::DecodingError(format!("Invalid pool pubkey: {}", e)))?;

        let mut writer = self.writer_for(&pool_pk, rec.dex_type).await?;
        writer.write_snapshot(&rec.snapshot)?;
        writer.flush()?;
        Ok(())
    }
}

impl std::str::FromStr for Pubkey {
    type Err = solana_sdk::pubkey::ParsePubkeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Pubkey::from_str(s)
    }
}
