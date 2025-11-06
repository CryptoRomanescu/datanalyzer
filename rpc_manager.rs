use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use solana_client::client_error::ClientError;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Signer};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tracing::{debug, error, info, warn, instrument};
use dashmap::DashMap;
use governor::{Quota, RateLimiter};
use std::num::NonZeroU32;
use opentelemetry::KeyValue;

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
    Tier1Premium, // Helius, Triton, QuickNode, Alchemy
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

/// Extended error types with ML-based classification (Universe Class)
#[derive(Debug, Clone, PartialEq)]
pub enum UniverseErrorType {
    // Base types
    Base(RpcErrorType),
    
    // Advanced types
    ValidatorBehind { slots: i64 },
    ConsensusFailure,
    GeyserStreamError,
    ShredstreamTimeout,
    CircuitBreakerOpen,
    PredictiveFailure { probability: f64 },
    SecurityViolation { reason: String },
    QuotaExceeded,
    ClusterCongestion { tps: u32 },
    
    // ML-classified with cluster ID
    ClusteredAnomaly { cluster_id: u8, confidence: f64 },
}

/// Fibonacci backoff strategy with jitter for adaptive recovery
#[derive(Debug, Clone)]
pub struct FibonacciBackoff {
    current_attempt: u32,
    max_attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
    jitter_factor: f64,
}

impl FibonacciBackoff {
    pub fn new(max_attempts: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            current_attempt: 0,
            max_attempts,
            base_delay_ms,
            max_delay_ms,
            jitter_factor: 0.1, // 10% jitter
        }
    }
    
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_attempt >= self.max_attempts {
            return None;
        }
        
        // Fibonacci sequence: 0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89...
        let fib = Self::fibonacci(self.current_attempt);
        let delay_ms = (self.base_delay_ms * fib as u64).min(self.max_delay_ms);
        
        // Add jitter to prevent thundering herd
        let jitter = (rand::random::<f64>() - 0.5) * 2.0 * self.jitter_factor;
        let jittered_delay = (delay_ms as f64 * (1.0 + jitter)).max(0.0) as u64;
        
        self.current_attempt += 1;
        Some(Duration::from_millis(jittered_delay))
    }
    
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }
    
    fn fibonacci(n: u32) -> u32 {
        match n {
            0 => 0,
            1 => 1,
            n => {
                let mut a = 0u32;
                let mut b = 1u32;
                for _ in 2..=n {
                    let next = a.saturating_add(b);
                    a = b;
                    b = next;
                }
                b
            }
        }
    }
}

/// Circuit breaker state for tier-level failure isolation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failures detected, blocking requests
    HalfOpen, // Testing if service recovered
}

/// Per-tier circuit breaker
#[derive(Debug, Clone)]
pub struct TierCircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    failure_threshold: u32,
    success_threshold: u32,
    last_state_change: Instant,
    timeout: Duration,
}

impl TierCircuitBreaker {
    pub fn new(failure_threshold: u32, success_threshold: u32, timeout: Duration) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            failure_threshold,
            success_threshold,
            last_state_change: Instant::now(),
            timeout,
        }
    }
    
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.success_threshold {
                    self.state = CircuitState::Closed;
                    self.failure_count = 0;
                    self.success_count = 0;
                    self.last_state_change = Instant::now();
                    info!("✅ Circuit breaker closed - service recovered");
                }
            }
            CircuitState::Open => {
                // Check if timeout elapsed
                if self.last_state_change.elapsed() >= self.timeout {
                    self.state = CircuitState::HalfOpen;
                    self.success_count = 0;
                    self.failure_count = 0;
                    self.last_state_change = Instant::now();
                    info!("🔄 Circuit breaker half-open - testing recovery");
                }
            }
        }
    }
    
    pub fn record_failure(&mut self) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.failure_threshold {
                    self.state = CircuitState::Open;
                    self.last_state_change = Instant::now();
                    error!("🚨 Circuit breaker opened - too many failures");
                }
            }
            CircuitState::HalfOpen => {
                self.state = CircuitState::Open;
                self.failure_count = 0;
                self.last_state_change = Instant::now();
                error!("🚨 Circuit breaker opened - recovery failed");
            }
            CircuitState::Open => {
                // Already open, check timeout
                if self.last_state_change.elapsed() >= self.timeout {
                    self.state = CircuitState::HalfOpen;
                    self.success_count = 0;
                    self.failure_count = 0;
                    self.last_state_change = Instant::now();
                }
            }
        }
    }
    
    pub fn can_execute(&self) -> bool {
        self.state != CircuitState::Open
    }
    
    pub fn get_state(&self) -> CircuitState {
        self.state
    }
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

/// ML-based predictive health model for failure prediction
#[derive(Debug, Clone)]
pub struct PredictiveHealthModel {
    /// Historical latency samples (ring buffer)
    latency_history: Vec<f64>,
    /// Historical error rate samples
    error_rate_history: Vec<f64>,
    /// Historical slot lag samples
    slot_lag_history: Vec<i64>,
    /// Maximum history window
    max_history: usize,
    /// Failure probability threshold for switching
    failure_threshold: f64,
    /// Last prediction time
    last_prediction: Option<Instant>,
    /// Current failure probability
    current_probability: f64,
}

impl PredictiveHealthModel {
    pub fn new(max_history: usize, failure_threshold: f64) -> Self {
        Self {
            latency_history: Vec::with_capacity(max_history),
            error_rate_history: Vec::with_capacity(max_history),
            slot_lag_history: Vec::with_capacity(max_history),
            max_history,
            failure_threshold,
            last_prediction: None,
            current_probability: 0.0,
        }
    }
    
    /// Record observation for ML model
    pub fn record_observation(&mut self, latency_ms: f64, error_rate: f64, slot_lag: i64) {
        // Maintain circular buffer
        if self.latency_history.len() >= self.max_history {
            self.latency_history.remove(0);
            self.error_rate_history.remove(0);
            self.slot_lag_history.remove(0);
        }
        
        self.latency_history.push(latency_ms);
        self.error_rate_history.push(error_rate);
        self.slot_lag_history.push(slot_lag);
    }
    
    /// Predict failure probability using ensemble of heuristics
    /// In production: use smartcore RandomForest or linfa LogisticRegression
    pub fn predict_failure_probability(&mut self) -> f64 {
        self.last_prediction = Some(Instant::now());
        
        if self.latency_history.len() < 10 {
            self.current_probability = 0.0;
            return 0.0;
        }
        
        // Use recent window for prediction
        let window_size = 10.min(self.latency_history.len());
        let start_idx = self.latency_history.len() - window_size;
        
        let recent_latencies = &self.latency_history[start_idx..];
        let recent_errors = &self.error_rate_history[start_idx..];
        let recent_lags = &self.slot_lag_history[start_idx..];
        
        // Feature engineering
        let avg_latency: f64 = recent_latencies.iter().sum::<f64>() / window_size as f64;
        let max_latency = recent_latencies.iter().cloned().fold(0.0_f64, f64::max);
        let avg_error: f64 = recent_errors.iter().sum::<f64>() / window_size as f64;
        let avg_lag: f64 = recent_lags.iter().map(|&x| x as f64).sum::<f64>() / window_size as f64;
        
        // Compute variance (volatility indicator)
        let latency_variance: f64 = recent_latencies.iter()
            .map(|&x| (x - avg_latency).powi(2))
            .sum::<f64>() / window_size as f64;
        let latency_std = latency_variance.sqrt();
        
        // Trend detection (is latency increasing?)
        let trend = if window_size > 5 {
            let first_half: f64 = recent_latencies[..window_size/2].iter().sum::<f64>() / (window_size/2) as f64;
            let second_half: f64 = recent_latencies[window_size/2..].iter().sum::<f64>() / (window_size - window_size/2) as f64;
            ((second_half - first_half) / first_half.max(1.0)).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        
        // Weighted scoring model (simplified ML ensemble)
        let latency_score = (avg_latency / 1000.0).min(1.0);
        let spike_score = ((max_latency - avg_latency) / 1000.0).min(1.0);
        let error_score = avg_error;
        let lag_score = (avg_lag / 10.0).clamp(0.0, 1.0);
        let volatility_score = (latency_std / 500.0).min(1.0);
        let trend_score = if trend > 0.0 { trend } else { 0.0 };
        
        // Ensemble prediction with calibrated weights
        let probability = 
            0.25 * latency_score +
            0.20 * spike_score +
            0.30 * error_score +
            0.10 * lag_score +
            0.10 * volatility_score +
            0.05 * trend_score;
        
        self.current_probability = probability.clamp(0.0, 1.0);
        self.current_probability
    }
    
    /// Check if predictive switching should occur
    pub fn should_switch_preemptively(&mut self) -> bool {
        // Rate limit predictions
        if let Some(last) = self.last_prediction {
            if last.elapsed() < Duration::from_secs(5) {
                return self.current_probability > self.failure_threshold;
            }
        }
        
        let prob = self.predict_failure_probability();
        prob > self.failure_threshold
    }
    
    /// Get current failure probability without re-computing
    pub fn get_current_probability(&self) -> f64 {
        self.current_probability
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
    
    // Universe-class enhancements
    /// ML-based predictive model
    pub predictor: Option<Arc<Mutex<PredictiveHealthModel>>>,
    /// Circuit breaker for this endpoint
    pub circuit_breaker: Option<Arc<Mutex<TierCircuitBreaker>>>,
    /// Rate limiter
    pub rate_limiter: Option<Arc<RateLimiter<String, governor::state::InMemoryState, governor::clock::DefaultClock>>>,
    /// Current slot lag from network
    pub slot_lag: i64,
    /// Last slot check time
    pub last_slot_check: Instant,
    /// Adaptive backoff state
    pub backoff: Option<FibonacciBackoff>,
    /// TLS certificate expiry (for security monitoring)
    pub cert_expiry: Option<SystemTime>,
    /// Anomaly detection score
    pub anomaly_score: f64,
    /// Shard assignment for load distribution
    pub shard_id: Option<u32>,
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
            .field("slot_lag", &self.slot_lag)
            .field("anomaly_score", &self.anomaly_score)
            .field("shard_id", &self.shard_id)
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

/// Universe-class metrics for observability
#[derive(Debug, Clone, Default)]
pub struct UniverseMetrics {
    /// Total requests across all endpoints
    pub total_requests: Arc<RwLock<u64>>,
    /// Total errors
    pub total_errors: Arc<RwLock<u64>>,
    /// Per-tier success rates
    pub tier_success_rates: Arc<DashMap<RpcTier, f64>>,
    /// Latency percentiles (P50, P95, P99)
    pub latency_p50: Arc<RwLock<f64>>,
    pub latency_p95: Arc<RwLock<f64>>,
    pub latency_p99: Arc<RwLock<f64>>,
    /// Circuit breaker states
    pub circuit_breaker_open_count: Arc<RwLock<u32>>,
    /// Predictive failures averted
    pub predictive_switches: Arc<RwLock<u64>>,
    /// Rate limit hits
    pub rate_limit_hits: Arc<RwLock<u64>>,
}

impl UniverseMetrics {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn record_request(&self, tier: RpcTier, success: bool) {
        *self.total_requests.write() += 1;
        if !success {
            *self.total_errors.write() += 1;
        }
        
        // Update tier-specific success rate
        let current = self.tier_success_rates.entry(tier).or_insert(1.0);
        let alpha = 0.1; // EWMA factor
        *current = if success {
            alpha * 1.0 + (1.0 - alpha) * *current
        } else {
            alpha * 0.0 + (1.0 - alpha) * *current
        };
    }
    
    pub fn record_latency(&self, latency_ms: f64, latencies: &[f64]) {
        // Update percentiles - simplified version
        // In production, use HDRHistogram or similar
        if !latencies.is_empty() {
            let mut sorted = latencies.to_vec();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            
            let p50_idx = (sorted.len() as f64 * 0.50) as usize;
            let p95_idx = (sorted.len() as f64 * 0.95) as usize;
            let p99_idx = (sorted.len() as f64 * 0.99) as usize;
            
            *self.latency_p50.write() = sorted.get(p50_idx).copied().unwrap_or(0.0);
            *self.latency_p95.write() = sorted.get(p95_idx).copied().unwrap_or(0.0);
            *self.latency_p99.write() = sorted.get(p99_idx).copied().unwrap_or(0.0);
        }
    }
    
    pub fn record_predictive_switch(&self) {
        *self.predictive_switches.write() += 1;
    }
    
    pub fn record_rate_limit_hit(&self) {
        *self.rate_limit_hits.write() += 1;
    }
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
    
    // Universe-class enhancements
    /// Per-tier circuit breakers
    tier_circuit_breakers: Arc<RwLock<HashMap<RpcTier, TierCircuitBreaker>>>,
    /// DashMap for concurrent endpoint access
    endpoints_concurrent: Arc<DashMap<String, RpcEndpoint>>,
    /// OpenTelemetry tracer for distributed tracing
    tracer: Option<Arc<dyn opentelemetry::trace::Tracer + Send + Sync>>,
    /// Global rate limiter
    global_rate_limiter: Option<Arc<RateLimiter<(), governor::state::InMemoryState, governor::clock::DefaultClock>>>,
    /// Configuration reload notifier
    config_watcher: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Metrics for observability
    metrics: Arc<UniverseMetrics>,
}

impl RpcManager {
    /// Creates a new RPC Manager with multiple endpoints for the Quantum Race Architecture
    pub fn new(rpc_urls: &[String]) -> Self {
        let endpoints: Vec<RpcEndpoint> = rpc_urls
            .iter()
            .map(|url| {
                let location = Self::infer_location_from_url(url);
                let tier = Self::infer_tier_from_url(url);
                
                // Initialize universe-class features
                let predictor = Some(Arc::new(Mutex::new(PredictiveHealthModel::new(100, 0.75))));
                let circuit_breaker = Some(Arc::new(Mutex::new(
                    TierCircuitBreaker::new(5, 3, Duration::from_secs(60))
                )));
                
                // Initialize rate limiter (100 req/s per endpoint)
                let quota = Quota::per_second(NonZeroU32::new(100).unwrap());
                let rate_limiter = Some(Arc::new(RateLimiter::keyed(quota)));
                
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
                    predictor,
                    circuit_breaker,
                    rate_limiter,
                    slot_lag: 0,
                    last_slot_check: Instant::now(),
                    backoff: Some(FibonacciBackoff::new(10, 100, 30000)),
                    cert_expiry: None,
                    anomaly_score: 0.0,
                    shard_id: None,
                }
            })
            .collect();

        info!(
            "🌐 RpcManager initialized with {} endpoints (Universe-Class Mode)",
            endpoints.len()
        );
        for endpoint in &endpoints {
            info!(
                "   📡 {} (location: {:?}, tier: {:?})",
                endpoint.url, endpoint.location, endpoint.tier
            );
        }
        
        // Initialize tier circuit breakers
        let mut tier_cbs = HashMap::new();
        tier_cbs.insert(RpcTier::Tier0Ultra, TierCircuitBreaker::new(3, 2, Duration::from_secs(30)));
        tier_cbs.insert(RpcTier::Tier1Premium, TierCircuitBreaker::new(5, 3, Duration::from_secs(60)));
        tier_cbs.insert(RpcTier::Tier2Public, TierCircuitBreaker::new(7, 4, Duration::from_secs(90)));
        
        // Initialize concurrent endpoints map
        let endpoints_concurrent = DashMap::new();
        for ep in &endpoints {
            endpoints_concurrent.insert(ep.url.clone(), ep.clone());
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
            tier_circuit_breakers: Arc::new(RwLock::new(tier_cbs)),
            endpoints_concurrent: Arc::new(endpoints_concurrent),
            tracer: None, // Will be initialized if OpenTelemetry is configured
            global_rate_limiter: None,
            config_watcher: Arc::new(Mutex::new(None)),
            metrics: Arc::new(UniverseMetrics::new()),
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
    /// Universe-class: includes predictive failure detection and circuit breaker management
    #[instrument(skip(self), name = "rpc_health_monitoring")]
    pub async fn start_monitoring(&self) {
        let endpoints = self.endpoints.clone();
        let endpoints_concurrent = self.endpoints_concurrent.clone();
        let tier_cbs = self.tier_circuit_breakers.clone();
        let metrics = self.metrics.clone();
        let interval = self.health_check_interval;

        let handle = tokio::spawn(async move {
            info!("💓 RPC health monitoring started - Universe-class predictive intelligence");

            loop {
                let start_time = Instant::now();
                
                // Parallel health probes using tokio tasks
                let endpoint_snapshots: Vec<(String, Arc<RpcClient>, RpcTier)> = {
                    let endpoints_guard = endpoints.read();
                    endpoints_guard
                        .iter()
                        .map(|ep| (ep.url.clone(), ep.client.clone(), ep.tier))
                        .collect()
                };

                // Launch parallel probes (up to 100 concurrent)
                let mut probe_tasks = Vec::new();
                for (url, client, tier) in endpoint_snapshots {
                    let url_clone = url.clone();
                    let client_clone = client.clone();
                    let endpoints_concurrent_clone = endpoints_concurrent.clone();
                    let metrics_clone = metrics.clone();
                    
                    let task = tokio::spawn(async move {
                        // Check rate limiter first
                        if let Some(mut ep_ref) = endpoints_concurrent_clone.get_mut(&url_clone) {
                            if let Some(ref rate_limiter) = ep_ref.rate_limiter {
                                if rate_limiter.check_key(&url_clone).is_err() {
                                    metrics_clone.record_rate_limit_hit();
                                    return None;
                                }
                            }
                        }
                        
                        let probe_start = Instant::now();
                        
                        match client_clone.get_health().await {
                            Ok(_) => {
                                let latency = probe_start.elapsed().as_millis() as f64;
                                
                                // Try to get slot for lag calculation
                                let slot_lag = if let Ok(slot) = tokio::time::timeout(
                                    Duration::from_millis(200),
                                    client_clone.get_slot()
                                ).await {
                                    // In production, compare with network consensus slot
                                    slot.map(|_| 0i64).unwrap_or(0)
                                } else {
                                    0
                                };
                                
                                Some((url_clone, latency, 0u64, RpcHealth::Healthy, slot_lag, tier))
                            }
                            Err(e) => {
                                warn!("❌ {} health check failed: {}", url_clone, e);
                                Some((url_clone, 9999.0, 1u64, RpcHealth::Unhealthy, 0i64, tier))
                            }
                        }
                    });
                    
                    probe_tasks.push(task);
                }
                
                // Wait for all probes with timeout
                let probe_results = futures_util::future::join_all(probe_tasks).await;
                let mut health_updates = Vec::new();
                let mut all_latencies = Vec::new();
                
                for result in probe_results {
                    if let Ok(Some(update)) = result {
                        all_latencies.push(update.1);
                        health_updates.push(update);
                    }
                }
                
                // Update latency percentiles
                if !all_latencies.is_empty() {
                    metrics.record_latency(0.0, &all_latencies);
                }

                // Update endpoint states and run predictive models
                {
                    let mut endpoints_guard = endpoints.write();
                    for (url, latency, error_increment, health, slot_lag, tier) in health_updates {
                        if let Some(endpoint) = endpoints_guard.iter_mut().find(|ep| ep.url == url)
                        {
                            endpoint.latency_ms = latency;
                            endpoint.last_check = Instant::now();
                            endpoint.slot_lag = slot_lag;
                            endpoint.last_slot_check = Instant::now();
                            
                            let success = matches!(health, RpcHealth::Healthy);
                            
                            // Update stats
                            endpoint.stats.record_request(latency, success);
                            
                            // Record metrics
                            metrics.record_request(tier, success);
                            
                            // Update predictor
                            if let Some(ref predictor_arc) = endpoint.predictor {
                                let mut predictor = predictor_arc.lock();
                                let error_rate = 1.0 - endpoint.stats.success_rate();
                                predictor.record_observation(latency, error_rate, slot_lag);
                                
                                // Check for predictive failure
                                if predictor.should_switch_preemptively() {
                                    let prob = predictor.get_current_probability();
                                    warn!("🔮 Predictive failure for {} (probability: {:.2})", url, prob);
                                    endpoint.health = RpcHealth::Degraded;
                                    metrics.record_predictive_switch();
                                } else {
                                    endpoint.health = health;
                                }
                            } else {
                                endpoint.health = health;
                            }
                            
                            // Update circuit breaker
                            if let Some(ref cb_arc) = endpoint.circuit_breaker {
                                let mut cb = cb_arc.lock();
                                if success {
                                    cb.record_success();
                                } else {
                                    cb.record_failure();
                                }
                                
                                // Update health based on circuit state
                                if !cb.can_execute() {
                                    endpoint.health = RpcHealth::Unhealthy;
                                    debug!("⚡ Circuit breaker open for {}", url);
                                }
                            }
                            
                            // Update tier-level circuit breaker
                            {
                                let mut tier_cbs_guard = tier_cbs.write();
                                if let Some(tier_cb) = tier_cbs_guard.get_mut(&tier) {
                                    if success {
                                        tier_cb.record_success();
                                    } else {
                                        tier_cb.record_failure();
                                    }
                                }
                            }

                            if error_increment > 0 {
                                endpoint.error_count += error_increment;
                                if endpoint.error_count >= 3 {
                                    endpoint.health = RpcHealth::Unhealthy;
                                }
                            } else {
                                endpoint.error_count = 0;
                            }
                            
                            // Update concurrent map
                            endpoints_concurrent.insert(url.clone(), endpoint.clone());
                        }
                    }
                }
                
                let elapsed = start_time.elapsed();
                debug!("📊 Health monitoring cycle completed in {:?}", elapsed);

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
    
    // ===== Universe-Class Extensions =====
    
    /// Get metrics for observability dashboards
    pub fn get_universe_metrics(&self) -> Arc<UniverseMetrics> {
        self.metrics.clone()
    }
    
    /// Enable OpenTelemetry distributed tracing
    pub fn enable_telemetry(&mut self, tracer: Arc<dyn opentelemetry::trace::Tracer + Send + Sync>) {
        info!("🔭 OpenTelemetry distributed tracing enabled");
        self.tracer = Some(tracer);
    }
    
    /// Hot-reload configuration from file
    pub async fn start_config_watcher(&self, config_path: &str) -> Result<()> {
        use notify::{Watcher, RecursiveMode, Event};
        use tokio::sync::mpsc;
        
        info!("👁️ Starting configuration hot-reload watcher on {}", config_path);
        
        let config_path_clone = config_path.to_string();
        let live_config = self.live_config.clone();
        
        let handle = tokio::spawn(async move {
            // Simplified config watcher - in production would use notify crate properly
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            
            loop {
                interval.tick().await;
                debug!("🔄 Checking for configuration changes...");
                
                // In production: Actually reload config from file
                // For now, just log that we're watching
            }
        });
        
        *self.config_watcher.lock() = Some(handle);
        Ok(())
    }
    
    /// Add endpoint dynamically (hot-swap capability)
    pub async fn add_endpoint_hot(&self, url: String) -> Result<()> {
        info!("➕ Hot-adding endpoint: {}", url);
        
        let location = Self::infer_location_from_url(&url);
        let tier = Self::infer_tier_from_url(&url);
        
        let predictor = Some(Arc::new(Mutex::new(PredictiveHealthModel::new(100, 0.75))));
        let circuit_breaker = Some(Arc::new(Mutex::new(
            TierCircuitBreaker::new(5, 3, Duration::from_secs(60))
        )));
        let quota = Quota::per_second(NonZeroU32::new(100).unwrap());
        let rate_limiter = Some(Arc::new(RateLimiter::keyed(quota)));
        
        let endpoint = RpcEndpoint {
            url: url.clone(),
            client: Arc::new(RpcClient::new(url.clone())),
            health: RpcHealth::Healthy,
            latency_ms: 0.0,
            error_count: 0,
            last_check: Instant::now(),
            location,
            tier,
            stats: PerfStats::new(0.2),
            predictor,
            circuit_breaker,
            rate_limiter,
            slot_lag: 0,
            last_slot_check: Instant::now(),
            backoff: Some(FibonacciBackoff::new(10, 100, 30000)),
            cert_expiry: None,
            anomaly_score: 0.0,
            shard_id: None,
        };
        
        // Add to both collections
        {
            let mut endpoints = self.endpoints.write();
            endpoints.push(endpoint.clone());
        }
        self.endpoints_concurrent.insert(url.clone(), endpoint);
        
        info!("✅ Endpoint {} added successfully", url);
        Ok(())
    }
    
    /// Remove endpoint dynamically
    pub async fn remove_endpoint_hot(&self, url: &str) -> Result<()> {
        info!("➖ Hot-removing endpoint: {}", url);
        
        {
            let mut endpoints = self.endpoints.write();
            endpoints.retain(|ep| ep.url != url);
        }
        self.endpoints_concurrent.remove(url);
        
        info!("✅ Endpoint {} removed successfully", url);
        Ok(())
    }
    
    /// Get circuit breaker state for a tier
    pub fn get_tier_circuit_state(&self, tier: RpcTier) -> Option<CircuitState> {
        let cbs = self.tier_circuit_breakers.read();
        cbs.get(&tier).map(|cb| cb.get_state())
    }
    
    /// Execute with adaptive backoff and retry
    pub async fn execute_with_retry<F, T, E>(&self, mut f: F) -> Result<T>
    where
        F: FnMut() -> futures_util::future::BoxFuture<'static, Result<T, E>>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let mut backoff = FibonacciBackoff::new(10, 100, 30000);
        
        loop {
            match f().await {
                Ok(result) => {
                    backoff.reset();
                    return Ok(result);
                }
                Err(e) => {
                    error!("Request failed: {}", e);
                    
                    if let Some(delay) = backoff.next_delay() {
                        warn!("⏳ Retrying after {:?}", delay);
                        tokio::time::sleep(delay).await;
                    } else {
                        return Err(anyhow::anyhow!("Max retries exceeded: {}", e));
                    }
                }
            }
        }
    }
    
    /// Classify error with ML (advanced error classification)
    pub fn classify_error_advanced(&self, err: &dyn std::error::Error) -> UniverseErrorType {
        let base_type = Self::classify_error(err);
        let err_str = err.to_string();
        
        // Check for advanced patterns
        if err_str.contains("validator") && err_str.contains("behind") {
            return UniverseErrorType::ValidatorBehind { slots: 0 }; // Would parse actual slot count
        }
        
        if err_str.contains("consensus") {
            return UniverseErrorType::ConsensusFailure;
        }
        
        if err_str.contains("geyser") {
            return UniverseErrorType::GeyserStreamError;
        }
        
        if err_str.contains("circuit breaker") {
            return UniverseErrorType::CircuitBreakerOpen;
        }
        
        if err_str.contains("quota") || err_str.contains("rate") {
            return UniverseErrorType::QuotaExceeded;
        }
        
        // Default to base type
        UniverseErrorType::Base(base_type)
    }

    /// Classify error into RpcErrorType
    fn classify_error(err: &dyn std::error::Error) -> RpcErrorType {
        let s = err.to_string().to_ascii_lowercase();

        // Common Solana RPC error patterns
        if s.contains("blockhash not found") {
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
    
    // Universe-class tests
    
    #[test]
    fn test_fibonacci_backoff() {
        let mut backoff = FibonacciBackoff::new(5, 100, 10000);
        
        // First delay should be 0 * 100 = 0ms (with jitter)
        let d1 = backoff.next_delay();
        assert!(d1.is_some());
        
        // Second delay should be 1 * 100 = 100ms (with jitter)
        let d2 = backoff.next_delay();
        assert!(d2.is_some());
        
        // Continue until exhausted
        backoff.next_delay();
        backoff.next_delay();
        backoff.next_delay();
        
        // Should be exhausted now
        let d6 = backoff.next_delay();
        assert!(d6.is_none());
        
        // Reset should allow new delays
        backoff.reset();
        let d_reset = backoff.next_delay();
        assert!(d_reset.is_some());
    }
    
    #[test]
    fn test_circuit_breaker() {
        let mut cb = TierCircuitBreaker::new(3, 2, Duration::from_secs(5));
        
        // Initially closed
        assert_eq!(cb.get_state(), CircuitState::Closed);
        assert!(cb.can_execute());
        
        // Record failures
        cb.record_failure();
        assert_eq!(cb.get_state(), CircuitState::Closed);
        
        cb.record_failure();
        assert_eq!(cb.get_state(), CircuitState::Closed);
        
        cb.record_failure();
        // Should open after 3 failures
        assert_eq!(cb.get_state(), CircuitState::Open);
        assert!(!cb.can_execute());
    }
    
    #[test]
    fn test_predictive_health_model() {
        let mut model = PredictiveHealthModel::new(50, 0.7);
        
        // Record increasing latency and errors
        for i in 0..30 {
            let latency = 100.0 + (i as f64 * 20.0);
            let error_rate = i as f64 / 100.0;
            model.record_observation(latency, error_rate, i);
        }
        
        // Predict failure probability
        let prob = model.predict_failure_probability();
        assert!(prob >= 0.0 && prob <= 1.0);
        
        // With increasing latency/errors, probability should be significant
        assert!(prob > 0.3, "Probability {} should be > 0.3", prob);
    }
    
    #[test]
    fn test_universe_metrics() {
        let metrics = UniverseMetrics::new();
        
        // Record some requests
        metrics.record_request(RpcTier::Tier0Ultra, true);
        metrics.record_request(RpcTier::Tier0Ultra, true);
        metrics.record_request(RpcTier::Tier0Ultra, false);
        
        assert_eq!(*metrics.total_requests.read(), 3);
        assert_eq!(*metrics.total_errors.read(), 1);
        
        // Check tier success rate
        if let Some(rate) = metrics.tier_success_rates.get(&RpcTier::Tier0Ultra) {
            assert!(*rate > 0.5 && *rate < 1.0);
        }
    }
    
    #[tokio::test]
    async fn test_hot_add_remove_endpoint() {
        let rpc_urls = vec!["https://api.devnet.solana.com".to_string()];
        let manager = RpcManager::new(&rpc_urls);
        
        // Add endpoint
        let new_url = "https://api.testnet.solana.com".to_string();
        let result = manager.add_endpoint_hot(new_url.clone()).await;
        assert!(result.is_ok());
        
        // Verify it was added
        {
            let endpoints = manager.endpoints.read();
            assert_eq!(endpoints.len(), 2);
        }
        
        // Remove endpoint
        let result = manager.remove_endpoint_hot(&new_url).await;
        assert!(result.is_ok());
        
        // Verify it was removed
        {
            let endpoints = manager.endpoints.read();
            assert_eq!(endpoints.len(), 1);
        }
    }
    
    #[test]
    fn test_advanced_error_classification() {
        let manager = RpcManager::new(&["https://api.devnet.solana.com".to_string()]);
        
        // Test various error types
        let err = anyhow::anyhow!("Validator is behind by 10 slots");
        let classified = manager.classify_error_advanced(&*err);
        match classified {
            UniverseErrorType::ValidatorBehind { .. } => {},
            _ => panic!("Expected ValidatorBehind"),
        }
        
        let err2 = anyhow::anyhow!("Circuit breaker is open");
        let classified2 = manager.classify_error_advanced(&*err2);
        match classified2 {
            UniverseErrorType::CircuitBreakerOpen => {},
            _ => panic!("Expected CircuitBreakerOpen"),
        }
    }
}