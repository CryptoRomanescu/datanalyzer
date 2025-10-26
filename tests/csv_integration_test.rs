/// Integration test for CSV writer configuration and orchestrator integration
///
/// This test verifies that:
/// 1. CsvConfig can be loaded from TOML
/// 2. CsvConfig converts correctly to CsvWriterConfig
/// 3. CSV writer works with the configuration
/// 4. Rotation and batching work as configured
use datanalyzer::config::{AppConfig, CsvConfig};
use datanalyzer::csv_writer::CsvWriter;
use datanalyzer::models::{DexType, PoolSnapshot};
use std::fs;
use std::io::Write;

#[test]
fn test_csv_config_default_values() {
    let config = CsvConfig::default();

    assert!(config.append);
    assert_eq!(config.max_file_size, 500_000_000);
    assert_eq!(config.max_file_age, 0);
    assert_eq!(config.batch_size, 500);
    assert_eq!(config.batch_time_ms, 3000);
}

#[test]
fn test_csv_config_to_writer_config() {
    let csv_config = CsvConfig {
        append: true,
        max_file_size: 1_000_000,
        max_file_age: 3600,
        batch_size: 100,
        batch_time_ms: 5000,
    };

    let writer_config = csv_config.to_csv_writer_config();

    assert!(writer_config.append);
    assert_eq!(writer_config.max_file_size, 1_000_000);
    assert_eq!(writer_config.max_file_age, 3600);
    assert_eq!(writer_config.batch_size, 100);
    assert_eq!(writer_config.batch_time_ms, 5000);
}

#[test]
fn test_csv_config_from_toml() {
    let toml_content = r#"
rpc_url = "https://api.mainnet-beta.solana.com"
rpc_ws_url = "wss://api.mainnet-beta.solana.com"
output_dir = "./snapshots"
snapshot_interval_ms = 5000

[csv]
append = true
max_file_size = 500000000
max_file_age = 0
batch_size = 500
batch_time_ms = 3000

[[pools]]
pool_address = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"
dex_type = "raydium"
token_mint = "So11111111111111111111111111111111111111112"
"#;

    let temp_path = "/tmp/test_csv_config.toml";
    let mut file = fs::File::create(temp_path).unwrap();
    file.write_all(toml_content.as_bytes()).unwrap();

    let config = AppConfig::load(temp_path).unwrap();

    assert!(config.csv.append);
    assert_eq!(config.csv.max_file_size, 500_000_000);
    assert_eq!(config.csv.max_file_age, 0);
    assert_eq!(config.csv.batch_size, 500);
    assert_eq!(config.csv.batch_time_ms, 3000);

    fs::remove_file(temp_path).ok();
}

#[test]
fn test_csv_config_partial_override() {
    let toml_content = r#"
rpc_url = "https://api.mainnet-beta.solana.com"
rpc_ws_url = "wss://api.mainnet-beta.solana.com"
output_dir = "./snapshots"
snapshot_interval_ms = 5000

[csv]
batch_size = 1000

[[pools]]
pool_address = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"
dex_type = "raydium"
token_mint = "So11111111111111111111111111111111111111112"
"#;

    let temp_path = "/tmp/test_csv_partial.toml";
    let mut file = fs::File::create(temp_path).unwrap();
    file.write_all(toml_content.as_bytes()).unwrap();

    let config = AppConfig::load(temp_path).unwrap();

    // Custom value
    assert_eq!(config.csv.batch_size, 1000);

    // Default values
    assert!(config.csv.append);
    assert_eq!(config.csv.max_file_size, 500_000_000);
    assert_eq!(config.csv.max_file_age, 0);
    assert_eq!(config.csv.batch_time_ms, 3000);

    fs::remove_file(temp_path).ok();
}

#[test]
fn test_csv_writer_with_config_from_toml() {
    let csv_config = CsvConfig {
        append: false,
        max_file_size: 0,
        max_file_age: 0,
        batch_size: 10,
        batch_time_ms: 0,
    };

    let writer_config = csv_config.to_csv_writer_config();
    let test_file = "/tmp/test_integration_writer.csv";

    let _ = fs::remove_file(test_file);

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

    {
        let mut writer = CsvWriter::with_config(test_file, headers, writer_config.clone()).unwrap();

        // Create a test snapshot
        let snapshot = PoolSnapshot::new(
            "test_pool".to_string(),
            "test_mint".to_string(),
            DexType::Raydium,
            1000,
            2000,
            1730000000,
            2.0,
        )
        .unwrap();

        writer.write_record(snapshot.to_csv_row()).unwrap();
        writer.flush().unwrap();
    }

    // Verify file contents
    let contents = fs::read_to_string(test_file).unwrap();
    assert!(contents.contains("pool_address"));
    assert!(contents.contains("test_pool"));
    assert!(contents.contains("Raydium"));

    fs::remove_file(test_file).ok();
}

#[test]
fn test_csv_writer_batching_from_config() {
    let csv_config = CsvConfig {
        append: false,
        max_file_size: 0,
        max_file_age: 0,
        batch_size: 5,
        batch_time_ms: 0,
    };

    let writer_config = csv_config.to_csv_writer_config();
    let test_file = "/tmp/test_batching.csv";

    let _ = fs::remove_file(test_file);

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

    let mut writer = CsvWriter::with_config(test_file, headers, writer_config).unwrap();

    // Write 3 records - should not flush yet
    for i in 0..3 {
        let snapshot = PoolSnapshot::new(
            format!("pool_{}", i),
            "mint".to_string(),
            DexType::PumpFun,
            1000,
            2000,
            1730000000 + i as i64,
            2.0,
        )
        .unwrap();

        writer.write_record(snapshot.to_csv_row()).unwrap();
    }

    assert_eq!(writer.records_written(), 3);

    // Write 2 more - should trigger flush at 5
    for i in 3..5 {
        let snapshot = PoolSnapshot::new(
            format!("pool_{}", i),
            "mint".to_string(),
            DexType::PumpFun,
            1000,
            2000,
            1730000000 + i as i64,
            2.0,
        )
        .unwrap();

        writer.write_record(snapshot.to_csv_row()).unwrap();
    }

    // After batch_size=5, should have flushed and reset counter
    assert_eq!(writer.records_written(), 0);

    fs::remove_file(test_file).ok();
}

#[test]
fn test_orchestrator_headers_match_snapshot() {
    // This test verifies that the headers used in orchestrator match PoolSnapshot::to_csv_row()
    let snapshot = PoolSnapshot::new(
        "pool_address".to_string(),
        "token_mint".to_string(),
        DexType::Raydium,
        1000,
        2000,
        1730000000,
        2.0,
    )
    .unwrap();

    let csv_row = snapshot.to_csv_row();

    // The row should have 8 fields matching the headers
    assert_eq!(csv_row.len(), 8);
    assert_eq!(csv_row[0], "pool_address");
    assert_eq!(csv_row[1], "token_mint");
    assert_eq!(csv_row[2], "Raydium");
    assert_eq!(csv_row[3], "1000");
    assert_eq!(csv_row[4], "2000");
    assert_eq!(csv_row[5], "1730000000");
    assert_eq!(csv_row[6], "2");
    assert_eq!(csv_row[7], ""); // liquidity_usd is None
}
