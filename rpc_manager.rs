use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use solana_client::client_error::ClientError;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Signer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Health status of an RPC endpoint
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcHealth {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Access tiers to prioritize independent, low-latency paths
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcTier {
    Tier0Ultra, // private/Jito/Block Engine/dedicated
    Tier1Premium, // Helius, Triton, QuickNode/Quiknode, Alchemy
    Tier2Public, // fallback/public
}

/// Granular RPC error types to drive adaptive behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcErrorType {
    BlockhashNotFound,
    TransactionExpired,
    RateLimited,
    NodeUnhealthy,
    NetworkTimeout,
    AccountNotFound,
    InsufficientFunds,
    Other,
}

/// Live performance stats per endpoint (EWMA-based)
#[derive(Debug, Clone)]
pub struct PerfStats {
    /// Exponentially weighted moving average alpha (0..1)
    pub ewma_alpha: f64,
    /// Success probability EWMA (0..1)
    pub success_ewma: f64,
    /// Request latency (ms) EWMA (request->response, not confirmation)
    pub latency_ewma_ms: f64,
    /// Confirmation speed (ms) EWMA
    pub confirmation_ewma_ms: f64,
    pub total_requests: u64,
    pub total_errors: u64,
}

impl PerfStats {
    pub fn new(alpha: f64) -> Self {
        Self {
            ewma_alpha: alpha.clamp(0.01, 0.99),
            success_ewma: 1.0,
            latency_ewma_ms: 0.0,
            confirmation_ewma_ms: 0.0,
            total_requests: 0,
            total_errors: 0,
        }
    }

    fn ewma(prev: f64, sample: f64, alpha: f64) -> f64 {
        if prev == 0.0 {
            sample
        } else {
            alpha * sample + (1.0 - alpha) * prev
        }
    }

    pub fn record_request(&mut self, latency_ms: f64, success: bool) {
        self.total_requests = self.total_requests.saturating_add(1);
        if !success {
            self.total_errors = self.total_errors.saturating_add(1);
        }
        // Update success EWMA (1 for success, 0 for fail)
        let sample_succ = if success { 1.0 } else { 0.0 };
        self.success_ewma = Self::ewma(self.success_ewma, sample_succ, self.ewma_alpha);

        // Update latency EWMA
        if latency_ms.is_finite() && latency_ms >= 0.0 {
            self.latency_ewma_ms = Self::ewma(self.latency_ewma_ms, latency_ms, self.ewma_alpha);
        }
    }

    pub fn record_confirmation(&mut self, confirmation_ms: f64) {
        if confirmation_ms.is_finite() && confirmation_ms >= 0.0 {
            self.confirmation_ewma_ms =
                Self::ewma(self.confirmation_ewma_ms, confirmation_ms, self.ewma_alpha);
        }
    }

    pub fn success_rate(&self) -> f64 {
        self.success_ewma
    }

    pub fn avg_latency_ms(&self) -> f64 {
        self.latency_ewma_ms
    }

    pub fn confirmation_speed_ms(&self) -> f64 {
        self.confirmation_ewma_ms
    }
}

/// Scoring weights for RPC endpoint ranking (geo/stake/latency legacy)
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    pub geo_weight: f64,
    pub stake_weight: f64,
    pub latency_weight: f64,
}

/// Live-scoring configuration (new)
#[derive(Debug, Clone)]
pub struct LiveScoringConfig {
    /// Weight for success rate (0..100 scaled)
    pub success_weight: f64,
    /// Weight for confirmation speed (ms)
    pub confirmation_weight: f64,
    /// Tier boosts
    pub tier0_boost: f64,
    pub tier1_boost: f64,
    pub tier2_boost: f64,
    /// EWMA alpha for live stats
    pub ewma_alpha: f64,
    /// Desired allocation across tiers when selecting N endpoints
    pub tier_allocation: TierAllocation,
}

impl Default for LiveScoringConfig {
    fn default() -> Self {
        Self {
            success_weight: 40.0,
            confirmation_weight: 0.05, // penalize slow confirms
            tier0_boost: 30.0,
            tier1_boost: 12.0,
            tier2_boost: 0.0,
            ewma_alpha: 0.2,
            tier_allocation: TierAllocation {
                tier0: 0.7,
                tier1: 0.25,
                tier2: 0.05,
            },
        }
    }
}

/// How many selections per tier (fractions)
#[derive(Debug, Clone, Copy)]
pub struct TierAllocation {
    pub tier0: f64,
    pub tier1: f64,
    pub tier2: f64,
}

impl TierAllocation {
    fn normalize(self) -> Self {
        let sum = (self.tier0 + self.tier1 + self.tier2).max(0.0001);
        Self {
            tier0: (self.tier0 / sum).clamp(0.0, 1.0),
            tier1: (self.tier1 / sum).clamp(0.0, 1.0),
            tier2: (self.tier2 / sum).clamp(0.0, 1.0),
        }
    }
}

/// Information about an RPC endpoint
#[derive(Clone)]
pub struct RpcEndpoint {
    pub url: String,
    pub client: Arc<RpcClient>,
    pub health: RpcHealth,
    pub latency_ms: f64,
    pub error_count: u64,
    pub last_check: Instant,
    /// Geographic location hint for proximity calculations
    pub location: Option<String>,
    /// Access tier
    pub tier: RpcTier,
    /// Live performance stats
    pub stats: PerfStats,
}

impl std::fmt::Debug for RpcEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcEndpoint")
            .field("url", &self.url)
            .field("health", &self.health)
            .field("latency_ms", &self.latency_ms)
            .field("error_count", &self.error_count)
            .field("location", &self.location)
            .field("tier", &self.tier)
            .field("stats", &self.stats)
            .finish()
    }
}

/// Leader information for geographic/stake weighting
#[derive(Debug, Clone)]
pub struct LeaderInfo {
    pub validator_pubkey: Pubkey,
    pub location: Option<String>,
    pub stake_weight: f64,
    pub next_slot: u64,
}

/// The Command & Intelligence Center for the Quantum Race Architecture
/// Manages multiple RPC connections with health monitoring and intelligent routing
#[derive(Clone)]
pub struct RpcManager {
    endpoints: Arc<RwLock<Vec<RpcEndpoint>>>,
    leader_schedule: Arc<RwLock<HashMap<u64, Pubkey>>>,
    validator_info: Arc<RwLock<HashMap<Pubkey, LeaderInfo>>>,
    scoring_weights: ScoringWeights,
    live_config: LiveScoringConfig,
    health_check_interval: Duration,
    monitoring_task_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl RpcManager {
    /// Creates a new RPC Manager with multiple endpoints for the Quantum Race Architecture
    pub fn new(rpc_urls: &[String]) -> Self {
        let endpoints: Vec<RpcEndpoint> = rpc_urls
            .iter()
            .map(|url| {
                let location = Self::infer_location_from_url(url);
                let tier = Self::infer_tier_from_url(url);
                RpcEndpoint {
                    url: url.clone(),
                    client: Arc::new(RpcClient::new(url.clone())),
                    health: RpcHealth::Healthy,
                    latency_ms: 0.0,
                    error_count: 0,
                    last_check: Instant::now(),
                    location,
                    tier,
                    stats: PerfStats::new(0.2),
                }
            })
            .collect();

        info!(
            "🌐 RpcManager initialized with {} endpoints",
            endpoints.len()
        );
        for endpoint in &endpoints {
            info!(
                "   📡 {} (location: {:?}, tier: {:?})",
                endpoint.url, endpoint.location, endpoint.tier
            );
        }

        Self {
            endpoints: Arc::new(RwLock::new(endpoints)),
            leader_schedule: Arc::new(RwLock::new(HashMap::new())),
            validator_info: Arc::new(RwLock::new(HashMap::new())),
            scoring_weights: ScoringWeights {
                geo_weight: 1.0,
                stake_weight: 2.0,
                latency_weight: 0.5,
            },
            live_config: LiveScoringConfig::default(),
            health_check_interval: Duration::from_secs(1),
            monitoring_task_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a new RPC Manager with custom scoring weights (legacy)
    pub fn new_with_weights(rpc_urls: &[String], weights: ScoringWeights) -> Self {
        let mut manager = Self::new(rpc_urls);
        info!(
            "🎯 RpcManager configured with custom weights: geo={}, stake={}, latency={}",
            weights.geo_weight, weights.stake_weight, weights.latency_weight
        );
        manager.scoring_weights = weights;
        manager
    }

    /// Configure live-scoring parameters
    pub fn set_live_scoring_config(&mut self, config: LiveScoringConfig) {
        info!("🧠 Live scoring config updated: {:?}", config);
        // Also update EWMA alpha across existing endpoints
        {
            let mut endpoints = self.endpoints.write();
            for ep in endpoints.iter_mut() {
                ep.stats.ewma_alpha = config.ewma_alpha;
            }
        }
        self.live_config = config;
    }

    /// Backward compatibility constructor
    pub fn new_single(rpc_url: &str) -> Self {
        Self::new(&[rpc_url.to_string()])
    }

    /// Legacy single-URL constructor (maintaining exact API compatibility)
    pub fn new_legacy(rpc_url: &str) -> OldRpcManager {
        OldRpcManager::new(rpc_url)
    }

    /// Starts continuous health monitoring of all RPC endpoints
    /// Performs lightweight health checks every ~1s and classifies endpoints
    pub async fn start_monitoring(&self) {
        let endpoints = self.endpoints.clone();
        let interval = self.health_check_interval;

        let handle = tokio::spawn(async move {
            info!("💓 RPC health monitoring started - continuous intelligence gathering");

            loop {
                // Snapshot endpoints list to avoid holding lock during async operations
                let endpoint_snapshots: Vec<(String, Arc<RpcClient>)> = {
                    let endpoints_guard = endpoints.read();
                    endpoints_guard
                        .iter()
                        .map(|ep| (ep.url.clone(), ep.client.clone()))
                        .collect()
                };

                // Perform health checks without holding any locks
                let mut health_updates = Vec::new();
                for (url, client) in endpoint_snapshots {
                    let start_time = Instant::now();

                    match client.get_health().await {
                        Ok(_) => {
                            let latency = start_time.elapsed().as_millis() as f64;

                            // Classify health based on latency thresholds
                            let health = if latency < 150.0 {
                                RpcHealth::Healthy
                            } else if latency < 750.0 {
                                RpcHealth::Degraded
                            } else {
                                RpcHealth::Unhealthy
                            };

                            health_updates.push((url.clone(), latency, 0, health));
                            debug!(
                                "✅ {} {} ({}ms)",
                                url,
                                match health {
                                    RpcHealth::Healthy => "healthy",
                                    RpcHealth::Degraded => "degraded",
                                    RpcHealth::Unhealthy => "unhealthy",
                                },
                                latency
                            );
                        }
                        Err(e) => {
                            warn!("❌ {} health check failed: {}", url, e);
                            health_updates.push((url, 9999.0, 1, RpcHealth::Unhealthy));
                        }
                    }
                }

                // Update endpoint states in a single critical section
                {
                    let mut endpoints_guard = endpoints.write();
                    for (url, latency, error_increment, health) in health_updates {
                        if let Some(endpoint) = endpoints_guard.iter_mut().find(|ep| ep.url == url)
                        {
                            endpoint.latency_ms = latency;
                            endpoint.last_check = Instant::now();
                            endpoint.health = health;

                            // Feed live stats with the health request too
                            endpoint
                                .stats
                                .record_request(latency, matches!(health, RpcHealth::Healthy));

                            if error_increment > 0 {
                                endpoint.error_count += error_increment;
                                // Degrade health further if too many consecutive errors
                                if endpoint.error_count >= 3 {
                                    endpoint.health = RpcHealth::Unhealthy;
                                }
                            } else {
                                endpoint.error_count = 0; // Reset on success
                            }
                        }
                    }
                }

                tokio::time::sleep(interval).await;
            }
        });

        *self.monitoring_task_handle.lock() = Some(handle);
    }

    /// Updates the leader schedule for intelligent routing
    pub async fn update_leader_schedule(&self) -> Result<()> {
        // Get the first healthy client for fetching leader schedule
        let client = self.get_healthy_client().await?;

        // Fetch leader schedule for next epoch
        match client.get_leader_schedule(None).await {
            Ok(Some(schedule)) => {
                // Update leader schedule in critical section
                {
                    let mut leader_schedule = self.leader_schedule.write();
                    leader_schedule.clear();

                    for (validator_str, slots) in schedule {
                        if let Ok(validator_pubkey) = validator_str.parse::<Pubkey>() {
                            for slot in slots {
                                leader_schedule.insert(slot as u64, validator_pubkey);
                            }
                        }
                    }

                    info!(
                        "📅 Leader schedule updated with {} slots",
                        leader_schedule.len()
                    );
                }
            }
            Ok(None) => {
                warn!("⚠️ No leader schedule available");
            }
            Err(e) => {
                error!("❌ Failed to fetch leader schedule: {}", e);
                return Err(e.into());
            }
        }

        Ok(())
    }

    /// Record a request outcome (success/failure) and latency for a specific endpoint URL.
    /// Use this immediately after sending an RPC call (not confirmation).
    pub fn record_rpc_result(&self, url: &str, latency_ms: f64, success: bool) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.iter_mut().find(|e| e.url == url) {
            ep.stats.record_request(latency_ms, success);
            if !success {
                ep.error_count = ep.error_count.saturating_add(1);
                if ep.error_count >= 3 {
                    ep.health = RpcHealth::Unhealthy;
                } else if ep.error_count == 1 && ep.health == RpcHealth::Healthy {
                    ep.health = RpcHealth::Degraded;
                }
            }
        }
    }

    /// Record a confirmation time for a tx sent via endpoint URL.
    pub fn record_confirmation_time(&self, url: &str, confirmation_ms: f64) {
        let mut endpoints = self.endpoints.write();
        if let Some(ep) = endpoints.iter_mut().find(|e| e.url == url) {
            ep.stats.record_confirmation(confirmation_ms);
        }
    }

    /// Classify an RPC error string and update internal state if needed.
    pub fn classify_and_record_error(&self, url: &str, err: &dyn std::error::Error) -> RpcErrorType {
        let typ = Self::classify_error(err);
        // Adaptive reactions
        match typ {
            RpcErrorType::RateLimited => {
                // Soft degrade on rate limits (prefer other endpoints temporarily)
                let mut endpoints = self.endpoints.write();
                if let Some(ep) = endpoints.iter_mut().find(|e| e.url == url) {
                    ep.health = RpcHealth::Degraded;
                    ep.error_count = ep.error_count.saturating_add(1);
                    ep.stats.record_request(500.0, false);
                }
            }
            RpcErrorType::NodeUnhealthy | RpcErrorType::NetworkTimeout => {
                // Strong degrade
                let mut endpoints = self.endpoints.write();
                if let Some(ep) = endpoints.iter_mut().find(|e| e.url == url) {
                    ep.health = RpcHealth::Unhealthy;
                    ep.error_count = ep.error_count.saturating_add(2);
                    ep.stats.record_request(2000.0, false);
                }
            }
            _ => {
                // For tx-logic errors (e.g., BlockhashNotFound), don't punish node health,
                // but we still record a failed request so live success converges realistically.
                let mut endpoints = self.endpoints.write();
                if let Some(ep) = endpoints.iter_mut().find(|e| e.url == url) {
                    ep.stats.record_request(400.0, false);
                }
            }
        }
        typ
    }

    /// Get ranked RPC endpoints optimized for the current/next leader with live performance
    pub async fn get_ranked_rpc_endpoints(&self, count: usize) -> Result<Vec<Arc<RpcClient>>> {
        if count == 0 {
            return Ok(Vec::new());
        }

        // Get current slot to determine next leader
        let current_slot = self.get_current_slot().await.unwrap_or(0);
        let next_leader_slot = current_slot.saturating_add(1);

        // Snapshot data structures to avoid holding locks
        let (candidates, next_leader_info, live_cfg, legacy_weights) = {
            let endpoints_guard = self.endpoints.read();
            let leader_schedule = self.leader_schedule.read();
            let validator_info = self.validator_info.read();

            // Get next leader
            let next_leader = leader_schedule.get(&next_leader_slot);
            let leader_info = next_leader.and_then(|leader| validator_info.get(leader).cloned());

            // Filter healthy or degraded endpoints (prefer healthy)
            let candidates: Vec<RpcEndpoint> = endpoints_guard
                .iter()
                .filter(|ep| ep.health != RpcHealth::Unhealthy)
                .cloned()
                .collect();

            (
                candidates,
                leader_info,
                self.live_config.clone(),
                self.scoring_weights.clone(),
            )
        };

        if candidates.is_empty() {
            return Err(anyhow::anyhow!("No usable RPC endpoints available"));
        }

        // Compute score per endpoint
        let mut scored: Vec<(RpcEndpoint, f64)> = candidates
            .into_iter()
            .map(|ep| {
                let mut score = 100.0;

                // Legacy latency penalty
                score -= legacy_weights.latency_weight * ep.latency_ms;

                // Live performance scoring
                // Success is rewarded
                score += live_cfg.success_weight * (ep.stats.success_rate() * 100.0);

                // Confirmation time is penalized (smaller is better)
                score -= live_cfg.confirmation_weight * ep.stats.confirmation_speed_ms();

                // Apply tier boosts
                score += match ep.tier {
                    RpcTier::Tier0Ultra => live_cfg.tier0_boost,
                    RpcTier::Tier1Premium => live_cfg.tier1_boost,
                    RpcTier::Tier2Public => live_cfg.tier2_boost,
                };

                // Leader-aware (geo and stake) if available
                if let Some(ref leader_info) = next_leader_info {
                    // Geographic proximity bonus
                    let geo_bonus = if ep.location == leader_info.location && ep.location.is_some()
                    {
                        50.0
                    } else if ep.location.is_some() && leader_info.location.is_some() {
                        -10.0
                    } else {
                        0.0
                    };
                    score += legacy_weights.geo_weight * geo_bonus;

                    // Stake weight bonus (higher stake leaders have better connectivity)
                    score += legacy_weights.stake_weight * leader_info.stake_weight;
                }

                debug!(
                    "🎯 RPC {} scored {:.2} (tier: {:?}, succ:{:.2} lat_avg:{:.1}ms conf_avg:{:.1}ms location:{:?})",
                    ep.url,
                    score,
                    ep.tier,
                    ep.stats.success_rate(),
                    ep.stats.avg_latency_ms(),
                    ep.stats.confirmation_speed_ms(),
                    ep.location
                );

                (ep, score)
            })
            .collect();

        // Sort by score desc
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Tier-aware allocation
        let alloc = live_cfg.tier_allocation.normalize();
        let mut want0 = ((count as f64) * alloc.tier0).round() as usize;
        let mut want1 = ((count as f64) * alloc.tier1).round() as usize;
        let mut want2 = count.saturating_sub(want0 + want1);

        // Partition by tier (keeping sort order)
        let mut t0: Vec<(RpcEndpoint, f64)> = Vec::new();
        let mut t1: Vec<(RpcEndpoint, f64)> = Vec::new();
        let mut t2: Vec<(RpcEndpoint, f64)> = Vec::new();
        for item in scored {
            match item.0.tier {
                RpcTier::Tier0Ultra => t0.push(item),
                RpcTier::Tier1Premium => t1.push(item),
                RpcTier::Tier2Public => t2.push(item),
            }
        }

        let mut selected: Vec<Arc<RpcClient>> = Vec::with_capacity(count);

        // Helper to take top k from a vec
        let mut take_top = |v: &mut Vec<(RpcEndpoint, f64)>, k: &mut usize| {
            while *k > 0 && !v.is_empty() && selected.len() < count {
                let (ep, sc) = v.remove(0);
                info!("🚀 Selected RPC {} (score: {:.1})", ep.url, sc);
                selected.push(ep.client.clone());
                *k = k.saturating_sub(1);
            }
        };

        take_top(&mut t0, &mut want0);
        take_top(&mut t1, &mut want1);
        take_top(&mut t2, &mut want2);

        // If still not enough, fill from remaining regardless of tier
        if selected.len() < count {
            for (ep, sc) in t0.into_iter().chain(t1).chain(t2) {
                if selected.len() >= count {
                    break;
                }
                info!("🚀 Selected RPC {} (score: {:.1}) [spill]", ep.url, sc);
                selected.push(ep.client.clone());
            }
        }

        info!(
            "⚡ Quantum Race Intelligence: Selected {} optimal RPCs for leader slot {}",
            selected.len(),
            next_leader_slot
        );

        Ok(selected)
    }

    /// Get the first healthy client (fallback method)
    pub async fn get_healthy_client(&self) -> Result<Arc<RpcClient>> {
        // Snapshot endpoints to avoid holding lock
        let endpoints_snapshot: Vec<RpcEndpoint> = {
            let endpoints = self.endpoints.read();
            endpoints.clone()
        };

        // Prefer Tier0 healthy, then Tier1 healthy, then Tier2 healthy
        for tier in [RpcTier::Tier0Ultra, RpcTier::Tier1Premium, RpcTier::Tier2Public] {
            if let Some(ep) = endpoints_snapshot
                .iter()
                .find(|e| e.tier == tier && e.health == RpcHealth::Healthy)
            {
                return Ok(ep.client.clone());
            }
        }

        // If no healthy endpoints, try degraded ones in tier order
        for tier in [RpcTier::Tier0Ultra, RpcTier::Tier1Premium, RpcTier::Tier2Public] {
            if let Some(ep) = endpoints_snapshot
                .iter()
                .find(|e| e.tier == tier && e.health == RpcHealth::Degraded)
            {
                warn!("⚠️ Using degraded RPC endpoint: {}", ep.url);
                return Ok(ep.client.clone());
            }
        }

        Err(anyhow::anyhow!("No usable RPC endpoints available"))
    }

    /// Get current slot from the best available client
    async fn get_current_slot(&self) -> Result<u64> {
        let client = self.get_healthy_client().await?;
        Ok(client.get_slot().await?)
    }

    /// Infer geographic location from RPC URL patterns
    fn infer_location_from_url(url: &str) -> Option<String> {
        let url_l = url.to_ascii_lowercase();
        if url_l.contains("helius") {
            Some("us-east".to_string())
        } else if url_l.contains("triton") {
            Some("us-west".to_string())
        } else if url_l.contains("quiknode") || url_l.contains("quicknode") {
            // Many Quiknode clusters default to US-East for Solana
            Some("us-east".to_string())
        } else if url_l.contains("alchemy") {
            Some("us-central".to_string())
        } else if url_l.contains("devnet") || url_l.contains("testnet") {
            Some("solana-labs".to_string())
        } else {
            None
        }
    }

    /// Infer access tier from URL/provider hints
    fn infer_tier_from_url(url: &str) -> RpcTier {
        let u = url.to_ascii_lowercase();
        if u.contains("block-engine")
            || u.contains("jito")
            || u.contains("private")
            || u.contains("dedicated")
        {
            RpcTier::Tier0Ultra
        } else if u.contains("helius")
            || u.contains("triton")
            || u.contains("quiknode")
            || u.contains("quicknode")
            || u.contains("alchemy")
        {
            RpcTier::Tier1Premium
        } else {
            RpcTier::Tier2Public
        }
    }

    /// Update validator information for better routing decisions
    pub async fn update_validator_info(&self, validators: HashMap<Pubkey, LeaderInfo>) {
        let count = validators.len();

        // Update in critical section
        {
            let mut validator_info = self.validator_info.write();
            *validator_info = validators;
        }

        info!("🗂️ Updated information for {} validators", count);
    }

    /// Start optional canary probes for deep network health monitoring
    /// Requires a separate payer keypair for canary transactions
    pub async fn start_canary_probes(
        &self,
        canary_payer: Option<Arc<solana_sdk::signature::Keypair>>,
    ) {
        if let Some(payer) = canary_payer {
            let endpoints = self.endpoints.clone();
            let payer_clone = payer.clone();

            tokio::spawn(async move {
                info!("🐦 Starting canary probes for deep network health monitoring");
                let mut interval = tokio::time::interval(Duration::from_secs(60)); // Every minute

                loop {
                    interval.tick().await;

                    // Get a healthy client for canary probe
                    let client = {
                        let endpoints_guard = endpoints.read();
                        endpoints_guard
                            .iter()
                            .find(|ep| ep.health == RpcHealth::Healthy)
                            .map(|ep| ep.client.clone())
                    };

                    if let Some(client) = client {
                        // Perform a simple balance check as canary probe
                        match tokio::time::timeout(
                            Duration::from_secs(5),
                            client.get_balance(&payer_clone.pubkey()),
                        )
                        .await
                        {
                            Ok(Ok(balance)) => {
                                debug!(
                                    "🐦 Canary probe successful - payer balance: {} lamports",
                                    balance
                                );
                            }
                            Ok(Err(e)) => {
                                warn!("🐦 Canary probe failed: {}", e);
                            }
                            Err(_) => {
                                warn!("🐦 Canary probe timed out");
                            }
                        }
                    }
                }
            });
        } else {
            info!("🐦 Canary probes disabled - no canary payer configured");
        }
    }

    /// Check if network state is consistent across healthy RPC endpoints
    /// Returns true if all healthy endpoints report similar slots (within 2 slots of each other)
    pub async fn is_network_consistent(&self) -> bool {
        // Get snapshot of healthy endpoints to avoid holding lock
        let healthy_endpoints: Vec<Arc<RpcClient>> = {
            let endpoints_guard = self.endpoints.read();
            endpoints_guard
                .iter()
                .filter(|ep| ep.health == RpcHealth::Healthy)
                .map(|ep| ep.client.clone())
                .collect()
        };

        if healthy_endpoints.len() < 2 {
            // If we have less than 2 healthy endpoints, consider it consistent
            return true;
        }

        // Fetch current slot from each healthy endpoint
        let mut slots = Vec::new();
        for client in healthy_endpoints.iter().take(3) {
            // Check max 3 endpoints for efficiency
            match tokio::time::timeout(Duration::from_millis(500), client.get_slot()).await {
                Ok(Ok(slot)) => slots.push(slot),
                Ok(Err(_)) | Err(_) => {} // Skip failed requests
            }
        }

        if slots.len() < 2 {
            return true; // Not enough data to determine inconsistency
        }

        // Check if all slots are within 2 slots of each other
        let min_slot = *slots.iter().min().unwrap();
        let max_slot = *slots.iter().max().unwrap();
        let is_consistent = max_slot - min_slot <= 2;

        if !is_consistent {
            warn!(
                "⚠️ Network inconsistency detected: slot range {}-{} (diff: {})",
                min_slot,
                max_slot,
                max_slot - min_slot
            );
        }

        is_consistent
    }

    /// Get health statistics for monitoring
    pub async fn get_health_stats(&self) -> HashMap<RpcHealth, usize> {
        // Snapshot endpoints to avoid holding lock
        let endpoints_snapshot: Vec<RpcEndpoint> = {
            let endpoints = self.endpoints.read();
            endpoints.clone()
        };

        let mut stats = HashMap::new();
        for endpoint in endpoints_snapshot.iter() {
            *stats.entry(endpoint.health).or_insert(0) += 1;
        }

        stats
    }

    /// Legacy compatibility methods
    pub fn get_client(&self) -> Arc<RpcClient> {
        // This is a synchronous method, so we'll use the first endpoint
        // In practice, this should be avoided in favor of get_healthy_client
        let endpoints = self.endpoints.read();
        if let Some(first_endpoint) = endpoints.first() {
            first_endpoint.client.clone()
        } else {
            panic!("No RPC endpoints configured")
        }
    }

    pub async fn get_optimal_rpc(&self) -> OptimalRpc {
        let client = self.get_healthy_client().await.unwrap_or_else(|_| {
            let endpoints = self.endpoints.read();
            if let Some(first_endpoint) = endpoints.first() {
                first_endpoint.client.clone()
            } else {
                panic!("No RPC endpoints configured")
            }
        });

        OptimalRpc { client }
    }

    /// Classify error into RpcErrorType
    fn classify_error(err: &dyn std::error::Error) -> RpcErrorType {
        let s = err.to_string().to_ascii_lowercase();

        // Common Solana RPC error patterns
        if s.contains("blockhash not found") || s.contains("blockhash not found") {
            RpcErrorType::BlockhashNotFound
        } else if s.contains("transaction expired")
            || s.contains("expired")
            || s.contains("last valid block height exceeded")
            || s.contains("block height exceeded")
        {
            RpcErrorType::TransactionExpired
        } else if s.contains("rate limit")
            || s.contains("too many requests")
            || s.contains("http 429")
            || s.contains("status 429")
        {
            RpcErrorType::RateLimited
        } else if s.contains("node is unhealthy")
            || s.contains("slot leader not found")
            || s.contains("rpc node unhealthy")
        {
            RpcErrorType::NodeUnhealthy
        } else if s.contains("timeout") || s.contains("timed out") {
            RpcErrorType::NetworkTimeout
        } else if s.contains("account not found") {
            RpcErrorType::AccountNotFound
        } else if s.contains("insufficient funds") || s.contains("insufficient lamports") {
            RpcErrorType::InsufficientFunds
        } else {
            RpcErrorType::Other
        }
    }

    /// Convenience to classify ClientError specifically
    #[allow(dead_code)]
    fn classify_client_error(err: &ClientError) -> RpcErrorType {
        Self::classify_error(err)
    }
}

/// Legacy structure for backward compatibility
pub struct OptimalRpc {
    pub client: Arc<RpcClient>,
}

/// Legacy RpcManager wrapper for full backward compatibility
#[derive(Clone)]
pub struct OldRpcManager {
    pub client: Arc<RpcClient>,
    quantum_manager: Arc<RpcManager>,
}

impl OldRpcManager {
    pub fn new(rpc_url: &str) -> Self {
        let client = Arc::new(RpcClient::new(rpc_url.to_string()));
        let quantum_manager = Arc::new(RpcManager::new(&[rpc_url.to_string()]));

        Self {
            client,
            quantum_manager,
        }
    }

    pub fn get_client(&self) -> &RpcClient {
        &self.client
    }

    pub async fn get_optimal_rpc(&self) -> OptimalRpc {
        OptimalRpc {
            client: self.client.clone(),
        }
    }

    /// Access to the new Quantum Race functionality
    pub fn quantum(&self) -> &RpcManager {
        &self.quantum_manager
    }
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            geo_weight: 1.0,
            stake_weight: 2.0,
            latency_weight: 0.5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_manager_initialization() {
        let rpc_urls = vec![
            "https://api.devnet.solana.com".to_string(),
            "https://api.testnet.solana.com".to_string(),
        ];

        // Test basic initialization
        let manager = RpcManager::new(&rpc_urls);
        let endpoints = manager.endpoints.read();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].url, "https://api.devnet.solana.com");
        assert_eq!(endpoints[1].url, "https://api.testnet.solana.com");
    }

    #[test]
    fn test_scoring_weights() {
        let weights = ScoringWeights::default();
        assert_eq!(weights.geo_weight, 1.0);
        assert_eq!(weights.stake_weight, 2.0);
        assert_eq!(weights.latency_weight, 0.5);

        let custom_weights = ScoringWeights {
            geo_weight: 2.5,
            stake_weight: 1.5,
            latency_weight: 0.8,
        };

        let rpc_urls = vec!["https://api.devnet.solana.com".to_string()];
        let manager = RpcManager::new_with_weights(&rpc_urls, custom_weights.clone());
        assert_eq!(manager.scoring_weights.geo_weight, 2.5);
        assert_eq!(manager.scoring_weights.stake_weight, 1.5);
        assert_eq!(manager.scoring_weights.latency_weight, 0.8);
    }

    #[test]
    fn test_location_inference() {
        assert_eq!(
            RpcManager::infer_location_from_url("https://white-polished-orb.solana-mainnet.quiknode.pro/311849bfafc79b24841bf73131a15cc5c5d3d7be/"),
            Some("us-east".to_string())
        );
        assert_eq!(
            RpcManager::infer_location_from_url("https://rpc.triton.one"),
            Some("us-west".to_string())
        );
        assert_eq!(
            RpcManager::infer_location_from_url("https://api.devnet.solana.com"),
            Some("solana-labs".to_string())
        );
        assert_eq!(
            RpcManager::infer_location_from_url("https://unknown-provider.com"),
            None
        );
    }

    #[test]
    fn test_rpc_health_copy_trait() {
        let health = RpcHealth::Healthy;
        let health_copy = health; // Should work because of Copy trait
        assert_eq!(health, health_copy);
    }

    #[tokio::test]
    async fn test_validator_info_update() {
        let rpc_urls = vec!["https://api.devnet.solana.com".to_string()];
        let manager = RpcManager::new(&rpc_urls);

        let mut validator_info = HashMap::new();
        let dummy_pubkey = Pubkey::new_unique();
        validator_info.insert(
            dummy_pubkey,
            LeaderInfo {
                validator_pubkey: dummy_pubkey,
                location: Some("us-east".to_string()),
                stake_weight: 1000.0,
                next_slot: 12345,
            },
        );

        manager.update_validator_info(validator_info).await;

        // Verify the info was stored
        let stored_info = manager.validator_info.read();
        assert!(stored_info.contains_key(&dummy_pubkey));
        assert_eq!(stored_info.get(&dummy_pubkey).unwrap().stake_weight, 1000.0);
    }
}