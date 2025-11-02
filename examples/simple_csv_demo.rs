/// Simple CSV Demo - Minimal demonstration of core CSV pipeline
///
/// This example demonstrates:
/// - Loading configuration from TOML
/// - Creating synthetic PoolSnapshot data
/// - Writing to CSV with proper headers
/// - Clean exit after writing a few rows
///
/// Usage: cargo run --example simple_csv_demo -- --config ./config.example.toml
use datanalyzer::config::AppConfig;
use datanalyzer::csv_writer::CsvWriter;
use datanalyzer::models::create_demo_snapshots;
use std::env;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::init();
    log::info!("Simple CSV Demo - Starting");

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let config_path = parse_config_path(&args);
    
    log::info!("Loading configuration from: {}", config_path);

    // Load configuration
    let app_config = AppConfig::load(&config_path)?;
    let csv_config = app_config.csv;

    log::info!("Configuration loaded successfully");
    log::info!("Output directory: {}", app_config.output_dir);
    log::info!("CSV batch size: {}", csv_config.batch_size);

    // Create output directory if it doesn't exist
    std::fs::create_dir_all(&app_config.output_dir)?;

    // Prepare CSV file path
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
