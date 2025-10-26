# DataAnalyzer Runbook

Operational guide for running, monitoring, and troubleshooting the DataAnalyzer system in production.

## Table of Contents

1. [System Overview](#system-overview)
2. [Deployment](#deployment)
3. [Monitoring & Health](#monitoring--health)
4. [Common Operations](#common-operations)
5. [Troubleshooting](#troubleshooting)
6. [Performance Tuning](#performance-tuning)
7. [Recovery Procedures](#recovery-procedures)
8. [Maintenance](#maintenance)

## System Overview

### Components

- **WebSocket Manager**: Maintains connections to Solana, handles subscriptions
- **Reserve Orchestrator**: Fetches vault balances via RPC for Raydium pools
- **Price Providers**: Jupiter (primary), CoinGecko (fallback), Stale Cache (last resort)
- **Token Metadata Provider**: Fetches token decimals and supply
- **CSV Writer**: Persists pool snapshots to disk
- **Observability**: Metrics (Prometheus), Health checks (HTTP), Logging (tracing)

### Key Metrics

- **Target Throughput**: 1000+ snapshots/hour per pool
- **RPC Call Rate**: <100 requests/minute (avoid rate limits)
- **CSV Write Rate**: 300-500 records/second
- **Memory Usage**: <500MB steady state
- **Reconnection Rate**: <1/hour (normal)

## Deployment

### Prerequisites

```bash
# System requirements
- Rust 1.70+
- 2GB RAM minimum, 4GB recommended
- 10GB disk space for data
- Network access to Solana RPC and WebSocket endpoints
```

### Build for Production

```bash
# Clean build
cargo clean
cargo build --release

# Verify build
./target/release/datanalyzer --version

# Run tests
cargo test --release
cargo deny check
```

### Configuration

Create `config.toml` in working directory:

```toml
rpc_url = "https://api.mainnet-beta.solana.com"
ws_url = "wss://api.mainnet-beta.solana.com"
snapshot_interval_ms = 60000
csv_file_path = "./data/pools.csv"

[price_fetcher]
cache_ttl_secs = 300

[[token_mapping]]
mint = "So11111111111111111111111111111111111111112"
coingecko_id = "solana"
cache_ttl_secs = 600

[[pools]]
address = "YOUR_POOL_ADDRESS"
dex_type = "raydium"  # or "pumpfun"

[healthcheck]
host = "0.0.0.0"  # Bind to all interfaces for Docker
port = 8080

[metrics]
host = "0.0.0.0"
port = 9090

[throttle]
updates_per_second = 10.0
bucket_size = 10
```

### Start Service

```bash
# Foreground (for testing)
./target/release/datanalyzer --config config.toml

# Background with nohup
nohup ./target/release/datanalyzer --config config.toml > datanalyzer.log 2>&1 &

# With systemd (recommended)
sudo systemctl start datanalyzer
sudo systemctl enable datanalyzer  # Auto-start on boot
```

### Systemd Service (Recommended)

Create `/etc/systemd/system/datanalyzer.service`:

```ini
[Unit]
Description=DataAnalyzer - Solana Pool Monitor
After=network.target

[Service]
Type=simple
User=datanalyzer
WorkingDirectory=/opt/datanalyzer
ExecStart=/opt/datanalyzer/target/release/datanalyzer --config /opt/datanalyzer/config.toml
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal

# Resource limits
MemoryMax=1G
CPUQuota=50%

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable datanalyzer
sudo systemctl start datanalyzer
```

## Monitoring & Health

### Health Check

```bash
# Quick health check
curl http://localhost:8080/health

# Expected response
{
  "status": "healthy",
  "timestamp": 1730000000,
  "checks": {
    "websocket": "connected",
    "rpc": "available"
  }
}

# Automated monitoring (add to cron or monitoring system)
*/5 * * * * curl -f http://localhost:8080/health || alert "DataAnalyzer unhealthy"
```

### Prometheus Metrics

```bash
# View all metrics
curl http://localhost:9090/metrics

# Key metrics to monitor
curl http://localhost:9090/metrics | grep datanalyzer_websocket_subscriptions
curl http://localhost:9090/metrics | grep datanalyzer_price_fetcher_errors
```

### Important Metrics

| Metric | Type | Alert Threshold | Description |
|--------|------|-----------------|-------------|
| `datanalyzer_websocket_subscriptions` | Gauge | = 0 (critical) | Active subscriptions |
| `datanalyzer_websocket_notifications_total` | Counter | Rate < 1/min (warning) | Total notifications |
| `datanalyzer_websocket_reconnections_total` | Counter | > 10/hour (warning) | Reconnection attempts |
| `datanalyzer_price_fetcher_errors` | Counter | > 100/hour (warning) | Price fetch failures |
| `datanalyzer_price_fetcher_cache_hits` | Counter | Rate < 50% (info) | Cache efficiency |

### Logging

```bash
# View logs (systemd)
journalctl -u datanalyzer -f

# View logs (file)
tail -f datanalyzer.log

# Filter by level
journalctl -u datanalyzer -p err  # Errors only
journalctl -u datanalyzer -p warning  # Warnings and errors

# Set log level via environment
RUST_LOG=debug ./target/release/datanalyzer
```

### Log Levels

- **ERROR**: Critical failures (immediate attention required)
- **WARN**: Degraded performance or transient errors
- **INFO**: Normal operation events (connections, subscriptions)
- **DEBUG**: Detailed operation info (cache hits, RPC calls)
- **TRACE**: Very verbose (for development only)

## Common Operations

### Adding a New Pool

1. Edit `config.toml`:
```toml
[[pools]]
address = "NEW_POOL_ADDRESS"
dex_type = "raydium"  # or "pumpfun"
```

2. Restart service:
```bash
sudo systemctl restart datanalyzer
```

3. Verify subscription:
```bash
curl http://localhost:9090/metrics | grep datanalyzer_websocket_subscriptions
# Should increase by 1
```

### Removing a Pool

1. Remove from `config.toml`
2. Restart service
3. Verify subscription count decreased

### Rotating CSV Files

CSV files rotate automatically based on configuration. Manual rotation:

```bash
# Move current file
mv ./data/pools.csv ./data/pools_$(date +%Y%m%d_%H%M%S).csv

# Service will create new file automatically
```

### Updating Configuration

1. Edit `config.toml`
2. Validate syntax: `toml-cli check config.toml` (if available)
3. Restart service: `sudo systemctl restart datanalyzer`
4. Verify health: `curl http://localhost:8080/health`

## Troubleshooting

### Raydium Pool Configuration Issues

**Symptom**: Logs show "Failed to decode account data" or "Invalid Raydium account size"

**Diagnosis**:
```bash
# Check pool configuration in logs
journalctl -u datanalyzer | grep -i "pool.*raydium"

# Verify pool owner and data length
journalctl -u datanalyzer | grep "First update for pool"

# Expected output:
# First update for pool 58oQ...: owner=675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8, data_length=752 bytes
# ✓ Verified Raydium AMM v4 program for pool 58oQ...
```

**Common Issues & Solutions**:

1. **Wrong pool address or program type**:
   ```
   ⚠ Pool 58oQ... owner 5quBto... is not Raydium AMM v4 (expected 675kPX9...)
   ```
   **Solution**: Pool is Raydium CLMM (v5), not AMM v4. Use a different pool or update decoder.

2. **Invalid Raydium account size: expected 752 bytes, got 1232 bytes**:
   **Solution**: This is a Raydium CLMM pool. Use Raydium AMM v4 pools only.
   **Expected Raydium AMM v4 program**: `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`
   **Expected account size**: 752 bytes

3. **Pool not found in Raydium API**:
   ```
   ⚠ Pool address ABC... not found in Raydium API. Proceeding anyway (may be a new pool).
   ```
   **Solution**: Either the pool is very new or the address is incorrect. Verify via:
   ```bash
   # Manual verification using Solana CLI or RPC
   solana account <pool_address> --url https://api.mainnet-beta.solana.com
   
   # Check owner field should be: 675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8
   # Check data length should be: 752
   ```

4. **Resolver fetch failed**:
   ```
   Failed to fetch Raydium pool data: HTTP request failed. Continuing with original addresses.
   ```
   **Solution**: 
   - Network connectivity issue or Raydium API is down
   - Service continues with configured addresses
   - You can disable resolver: `[raydium_resolver] enabled = false`

**Manual Pool Verification**:
```bash
# Using curl and jq to verify a pool via RPC
POOL="58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"
curl -s -X POST https://api.mainnet-beta.solana.com \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getAccountInfo\",\"params\":[\"$POOL\",{\"encoding\":\"base64\"}]}" \
  | jq '.result.value | {owner: .owner, size: (.data[0] | length)}'

# Expected output:
# {
#   "owner": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
#   "size": 1004
# }
# Note: Base64 encoding adds ~33% overhead: 752 raw bytes = 1004 base64 characters (4:3 ratio)
```

**Using Raydium Resolver to find pools**:
```bash
# Fetch current Raydium pools
curl -s "https://api.raydium.io/v2/sdk/liquidity/mainnet.json" | \
  jq '.official[] | select(.baseMint == "So11111111111111111111111111111111111111112") | 
      select(.quoteMint == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v") |
      {id, baseMint, quoteMint, programId}'

# Find SOL/USDC AMM pools
```

### WebSocket Connection Issues

**Symptom**: Health check shows `"websocket": "disconnected"`

**Diagnosis**:
```bash
# Check connectivity
curl -I wss://api.mainnet-beta.solana.com

# Check logs
journalctl -u datanalyzer -n 100 | grep -i websocket
```

**Resolution**:
1. Verify WebSocket URL in config is correct
2. Check firewall rules: `sudo iptables -L | grep 443`
3. Test alternative endpoint
4. Check for rate limiting in logs
5. Restart service: `sudo systemctl restart datanalyzer`

### RPC Rate Limiting (429 Errors)

**Symptom**: Logs show "Rate limit exceeded" or "429" errors, circuit breaker opens

**Diagnosis**:
```bash
# Check error rate
curl http://localhost:9090/metrics | grep price_fetcher_errors

# Check circuit breaker state in logs
journalctl -u datanalyzer | grep -i "circuit\|rate limit"
```

**Resolution**:
1. **Immediate**: Circuit breaker will activate automatically (60s timeout)
2. **Short-term**: 
   - Reduce `snapshot_interval_ms` to query less frequently
   - Enable throttling in config
3. **Long-term**:
   - Use premium RPC endpoint with higher limits
   - Implement request batching
   - Add more cache TTL

### High Memory Usage

**Symptom**: Process using >1GB RAM

**Diagnosis**:
```bash
# Check memory
ps aux | grep datanalyzer
top -p $(pgrep datanalyzer)

# Check for memory leaks
valgrind --leak-check=full ./target/release/datanalyzer (development only)
```

**Resolution**:
1. Check CSV file rotation is working
2. Verify cache TTLs are set (not infinite)
3. Reduce number of monitored pools
4. Restart service to clear caches
5. Review pool count vs available RAM

### CSV Write Failures

**Symptom**: Logs show "Failed to write CSV" errors

**Diagnosis**:
```bash
# Check disk space
df -h ./data

# Check permissions
ls -la ./data/
```

**Resolution**:
1. Verify disk space available: `df -h`
2. Check write permissions: `chmod 755 ./data`
3. Ensure directory exists: `mkdir -p ./data`
4. Check for disk errors: `dmesg | grep -i error`
5. Verify CSV file isn't corrupted or locked

### Missing Price Data

**Symptom**: Prices not appearing in CSV or logs show "Price not found"

**Diagnosis**:
```bash
# Check if token is in mapping
grep "YOUR_MINT" config.toml

# Check price provider status
curl http://localhost:9090/metrics | grep price_fetcher
```

**Resolution**:
1. **For CoinGecko**: Add mint → token_id mapping in config
2. **For Jupiter**: Verify mint address is correct
3. Check circuit breaker status in logs
4. Test price fetch manually:
   ```bash
   curl "https://price.jup.ag/v4/price?ids=YOUR_MINT"
   ```
5. Fallback chain should use stale cache if available

### Service Won't Start

**Symptom**: `systemctl start datanalyzer` fails

**Diagnosis**:
```bash
# Check status
sudo systemctl status datanalyzer

# View logs
journalctl -u datanalyzer -n 50

# Try manual start to see errors
./target/release/datanalyzer --config config.toml
```

**Resolution**:
1. Verify config file exists and is valid TOML
2. Check file permissions
3. Ensure all directories exist (`./data`, etc.)
4. Verify port 8080 and 9090 aren't in use: `netstat -tulpn | grep :8080`
5. Check for binary corruption: `file ./target/release/datanalyzer`

## Performance Tuning

### Optimize for High-Frequency Updates

```toml
# config.toml
snapshot_interval_ms = 30000  # More frequent snapshots

[throttle]
updates_per_second = 20.0  # Higher throughput
bucket_size = 20

[price_fetcher]
cache_ttl_secs = 60  # Shorter cache for fresh data
```

### Optimize for Low Resource Usage

```toml
# config.toml
snapshot_interval_ms = 300000  # Less frequent (5 min)

[throttle]
updates_per_second = 5.0  # Lower throughput
bucket_size = 5

[price_fetcher]
cache_ttl_secs = 600  # Longer cache (10 min)
```

### CSV Writer Performance

```rust
// Use larger batch sizes for better throughput
CsvWriterConfig::builder()
    .batch_size(500)  // Default: 100
    .batch_time_ms(10000)  // 10 seconds
    .build()
```

## Recovery Procedures

### Complete System Failure

1. **Stop service**:
   ```bash
   sudo systemctl stop datanalyzer
   ```

2. **Backup current state**:
   ```bash
   cp -r ./data ./data.backup.$(date +%Y%m%d_%H%M%S)
   ```

3. **Check disk integrity**:
   ```bash
   df -h
   dmesg | grep -i error
   ```

4. **Restart from clean state**:
   ```bash
   sudo systemctl start datanalyzer
   ```

5. **Verify recovery**:
   ```bash
   curl http://localhost:8080/health
   journalctl -u datanalyzer -f
   ```

### Data Corruption

1. **Identify corrupted file**:
   ```bash
   # Check CSV file validity
   head -n 100 ./data/pools.csv
   tail -n 100 ./data/pools.csv
   ```

2. **Rotate corrupted file**:
   ```bash
   mv ./data/pools.csv ./data/pools.corrupted.$(date +%Y%m%d)
   ```

3. **Service creates new file automatically**

4. **Recover data if possible**:
   ```bash
   # Extract valid rows
   grep -v "^ERROR" ./data/pools.corrupted.* > ./data/pools.recovered.csv
   ```

### WebSocket Persistent Disconnection

1. **Check endpoint status**: Visit Solana status page
2. **Try alternative endpoint**:
   - `wss://api.mainnet-beta.solana.com`
   - `wss://solana-api.projectserum.com`
   - Premium provider endpoints

3. **Update config** and restart

4. **If all fail**: Issue likely on Solana network side, wait for recovery

## Maintenance

### Daily

- ✅ Check health endpoint responds
- ✅ Verify CSV files are being created
- ✅ Monitor disk space usage

### Weekly

- ✅ Review error logs for patterns
- ✅ Check metrics for anomalies
- ✅ Archive old CSV files
- ✅ Verify subscription count matches config

### Monthly

- ✅ Update dependencies: `cargo update`
- ✅ Run full test suite: `cargo test --release`
- ✅ Run security audit: `cargo deny check`
- ✅ Review and rotate logs
- ✅ Analyze performance metrics
- ✅ Update documentation if config changed

### Quarterly

- ✅ Review and optimize pool list
- ✅ Benchmark performance tests
- ✅ Update Rust toolchain
- ✅ Review backup strategy
- ✅ Capacity planning based on growth

### Backup Strategy

```bash
# Daily automated backup
0 2 * * * /opt/datanalyzer/scripts/backup.sh

# backup.sh
#!/bin/bash
DATE=$(date +%Y%m%d)
tar -czf /backups/datanalyzer_$DATE.tar.gz \
    /opt/datanalyzer/data/ \
    /opt/datanalyzer/config.toml
find /backups -name "datanalyzer_*.tar.gz" -mtime +30 -delete
```

### Log Rotation

```bash
# /etc/logrotate.d/datanalyzer
/var/log/datanalyzer/*.log {
    daily
    rotate 7
    compress
    delaycompress
    notifempty
    create 0640 datanalyzer datanalyzer
    sharedscripts
    postrotate
        systemctl reload datanalyzer
    endscript
}
```

## Emergency Contacts

### Escalation Path

1. **Level 1**: Check this runbook, review logs
2. **Level 2**: Restart service, verify health
3. **Level 3**: Review Solana network status
4. **Level 4**: Contact on-call engineer

### Useful Links

- Solana Status: https://status.solana.com/
- Solana Docs: https://docs.solana.com/
- Raydium Status: https://raydium.io/
- GitHub Issues: https://github.com/CryptoRomanescu/datanalyzer/issues

## Appendix

### Quick Command Reference

```bash
# Start/Stop
sudo systemctl start datanalyzer
sudo systemctl stop datanalyzer
sudo systemctl restart datanalyzer

# Status
sudo systemctl status datanalyzer
curl http://localhost:8080/health
curl http://localhost:9090/metrics

# Logs
journalctl -u datanalyzer -f
journalctl -u datanalyzer -p err

# Performance
ps aux | grep datanalyzer
top -p $(pgrep datanalyzer)
netstat -tulpn | grep datanalyzer

# Debugging
RUST_LOG=debug ./target/release/datanalyzer
RUST_BACKTRACE=1 ./target/release/datanalyzer
```

---

**Last Updated**: 2025-10-25
**Version**: 0.1.0
**Maintainer**: DataAnalyzer Team
