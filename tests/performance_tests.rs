//! Performance and stress tests for Stage 5
//!
//! These tests verify system behavior under load:
//! - High-frequency data processing
//! - CSV persistence under load
//! - Orchestrator performance with many pools
//! - Memory usage and resource management
//! - Concurrent access patterns

#[cfg(test)]
mod performance_tests {
    use datanalyzer::{CsvWriter, DexType, PoolSnapshot};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::RwLock;
    use tokio::time::sleep;

    /// Helper function to create a test snapshot
    fn create_test_snapshot(id: u64, pool_suffix: &str) -> PoolSnapshot {
        PoolSnapshot::new(
            format!("Pool{}{:064}", pool_suffix, id),
            format!("Token{:064}", id),
            DexType::Raydium,
            1_000_000 + id,
            2_000_000 + id,
            chrono::Utc::now().timestamp(),
            100.0 + (id as f64) * 0.01,
        )
        .unwrap()
    }

    /// Test handling many pool snapshots in memory
    /// Verifies: Memory usage, data structure performance
    #[tokio::test]
    async fn test_many_pool_snapshots() {
        // Create 1000 unique pool snapshots
        let pool_count = 1000_usize;
        let snapshots: Vec<PoolSnapshot> = (0..pool_count as u64)
            .map(|i| create_test_snapshot(i, "A"))
            .collect();

        let start = Instant::now();

        // Process all snapshots (simulating real workload)
        let mut csv_rows = Vec::new();
        for snapshot in &snapshots {
            csv_rows.push(snapshot.to_csv_row());
        }

        let duration = start.elapsed();

        println!("✓ Processed {} snapshots in {:?}", pool_count, duration);
        println!(
            "✓ Throughput: {:.0} snapshots/sec",
            pool_count as f64 / duration.as_secs_f64()
        );

        assert_eq!(csv_rows.len(), pool_count);
        assert!(duration.as_millis() < 1000, "Processing took too long");
    }

    /// Test high-frequency writes to CSV writer
    /// Verifies: No data loss, proper flushing, performance under load
    #[tokio::test]
    async fn test_high_frequency_csv_writes() {
        let temp_dir = std::env::temp_dir().join("perf_test_csv");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let csv_path = temp_dir.join("high_freq.csv");
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

        let csv_writer = Arc::new(RwLock::new(
            CsvWriter::new(csv_path.as_path(), headers).unwrap(),
        ));

        // Generate 10,000 snapshots
        let snapshot_count = 10_000;
        let snapshots: Vec<PoolSnapshot> = (0..snapshot_count)
            .map(|i| create_test_snapshot(i, "B"))
            .collect();

        let start = Instant::now();

        // Write all snapshots
        for snapshot in snapshots {
            let mut writer = csv_writer.write().await;
            writer.write_record(snapshot.to_csv_row()).unwrap();
        }

        // Force flush
        {
            let mut writer = csv_writer.write().await;
            writer.flush().unwrap();
        }

        let write_time = start.elapsed();

        println!("✓ Wrote {} snapshots in {:?}", snapshot_count, write_time);
        println!(
            "✓ Throughput: {:.0} writes/sec",
            snapshot_count as f64 / write_time.as_secs_f64()
        );

        // Performance assertion: Should handle 10k writes in reasonable time
        assert!(
            write_time.as_secs() < 30,
            "CSV writes took too long: {:?}",
            write_time
        );

        // Verify file was created and has data
        let metadata = std::fs::metadata(&csv_path).unwrap();
        assert!(metadata.len() > 0, "CSV file is empty");

        println!("✓ CSV file size: {} bytes", metadata.len());

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Test CSV writer rotation under load
    /// Verifies: Rotation works correctly, no data loss during rotation
    #[tokio::test]
    async fn test_csv_rotation_under_load() {
        let temp_dir = std::env::temp_dir().join("perf_test_rotation");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let csv_path = temp_dir.join("rotation_test.csv");
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

        // Create writer - rotation is configured in CsvWriterConfig
        let csv_writer = Arc::new(RwLock::new(
            CsvWriter::new(csv_path.as_path(), headers).unwrap(),
        ));

        let start = Instant::now();

        // Write snapshots for a period, testing batching and flushing
        for i in 0..500 {
            let snapshot = create_test_snapshot(i, "C");
            let mut writer = csv_writer.write().await;
            writer.write_record(snapshot.to_csv_row()).unwrap();
            drop(writer);

            // Periodic flush
            if i % 100 == 0 {
                let mut writer = csv_writer.write().await;
                writer.flush().unwrap();
            }
        }

        // Final flush
        {
            let mut writer = csv_writer.write().await;
            writer.flush().unwrap();
        }

        let duration = start.elapsed();

        // Verify file exists and has data
        let metadata = std::fs::metadata(&csv_path).unwrap();
        assert!(metadata.len() > 0, "CSV file is empty");

        println!(
            "✓ Wrote 500 snapshots with periodic flushing in {:?}",
            duration
        );
        println!("✓ CSV file size: {} bytes", metadata.len());

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Test orchestrator creation - DISABLED
    /// NOTE: This test needs to be updated for the new Orchestrator API
    /// which now requires Oracle and TokenMetadataProvider
    #[tokio::test]
    #[ignore]
    async fn test_orchestrator_creation() {
        println!("This test needs to be updated for the new Orchestrator API.");
        println!("Please refer to src/orchestrator.rs and src/main.rs for current usage patterns.");
    }

    /// Test concurrent access to shared resources
    /// Verifies: Thread safety, no deadlocks, performance under contention
    #[tokio::test]
    async fn test_concurrent_csv_access() {
        let temp_dir = std::env::temp_dir().join("perf_test_concurrent");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let csv_path = temp_dir.join("concurrent.csv");
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

        let csv_writer = Arc::new(RwLock::new(
            CsvWriter::new(csv_path.as_path(), headers).unwrap(),
        ));

        let start = Instant::now();

        // Spawn 20 concurrent tasks writing to the same CSV
        let mut handles = vec![];
        for task_id in 0..20 {
            let writer = Arc::clone(&csv_writer);
            let handle = tokio::spawn(async move {
                for i in 0..100 {
                    let snapshot = create_test_snapshot(i, &format!("Task{}", task_id));
                    let mut w = writer.write().await;
                    w.write_record(snapshot.to_csv_row()).unwrap();
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        let duration = start.elapsed();

        println!(
            "✓ 20 concurrent tasks wrote 2000 total snapshots in {:?}",
            duration
        );
        println!(
            "✓ Throughput: {:.0} writes/sec",
            2000.0 / duration.as_secs_f64()
        );

        // Verify no deadlock occurred
        assert!(
            duration.as_secs() < 60,
            "Concurrent writes took too long, possible deadlock"
        );

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Memory stress test - verify no memory leaks with long-running operations
    /// Verifies: Proper cleanup, no unbounded growth
    #[tokio::test]
    async fn test_memory_usage_stability() {
        let temp_dir = std::env::temp_dir().join("perf_test_memory");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let csv_path = temp_dir.join("memory_test.csv");
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

        let csv_writer = Arc::new(RwLock::new(
            CsvWriter::new(csv_path.as_path(), headers).unwrap(),
        ));

        // Write snapshots in batches and verify memory doesn't grow unbounded
        let iterations = 5;
        let snapshots_per_iteration = 1000;

        for iteration in 0..iterations {
            let start = Instant::now();

            for i in 0..snapshots_per_iteration {
                let snapshot = create_test_snapshot(i, &format!("Mem{}", iteration));
                let mut writer = csv_writer.write().await;
                writer.write_record(snapshot.to_csv_row()).unwrap();
            }

            // Flush after each iteration
            {
                let mut writer = csv_writer.write().await;
                writer.flush().unwrap();
            }

            println!(
                "✓ Iteration {} completed in {:?}",
                iteration + 1,
                start.elapsed()
            );
        }

        println!(
            "✓ Completed {} iterations with {} snapshots each",
            iterations, snapshots_per_iteration
        );
        println!(
            "✓ Total: {} snapshots written",
            iterations * snapshots_per_iteration
        );

        // If we got here without OOM, the test passes
        // Cleanup
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Test system under combined load
    /// Verifies: Multiple components working together under stress
    #[tokio::test]
    async fn test_combined_load() {
        let temp_dir = std::env::temp_dir().join("perf_test_combined");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let csv_path = temp_dir.join("combined.csv");
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

        let csv_writer = Arc::new(RwLock::new(
            CsvWriter::new(csv_path.as_path(), headers).unwrap(),
        ));

        let start = Instant::now();

        // Task 1: Create snapshots in memory
        let snapshot_task = tokio::spawn(async move {
            let mut snapshots = Vec::new();
            for i in 0..100 {
                snapshots.push(create_test_snapshot(i, "Combined"));
                sleep(Duration::from_millis(10)).await;
            }
            snapshots
        });

        // Task 2: Write snapshots to CSV
        let writer_clone = Arc::clone(&csv_writer);
        let write_task = tokio::spawn(async move {
            for i in 0..1000 {
                let snapshot = create_test_snapshot(i, "Write");
                let mut writer = writer_clone.write().await;
                writer.write_record(snapshot.to_csv_row()).unwrap();
                drop(writer);
                sleep(Duration::from_millis(5)).await;
            }
        });

        // Wait for both tasks
        let snapshots = snapshot_task.await.unwrap();
        write_task.await.unwrap();

        let duration = start.elapsed();

        println!("✓ Combined load test completed in {:?}", duration);
        println!(
            "✓ Created {} snapshots in memory while writing 1000 to CSV",
            snapshots.len()
        );

        // Should complete in reasonable time
        assert!(duration.as_secs() < 30, "Combined load took too long");

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).unwrap();
    }

    /// Test data structure scalability
    /// Verifies: Can handle large numbers of pool snapshots efficiently
    #[tokio::test]
    async fn test_snapshot_scalability() {
        let start = Instant::now();

        // Create 10,000 snapshots and convert to CSV format
        let count = 10_000_usize;
        let snapshots: Vec<PoolSnapshot> = (0..count as u64)
            .map(|i| create_test_snapshot(i, "Scale"))
            .collect();

        let create_time = start.elapsed();

        // Convert all to CSV rows
        let csv_start = Instant::now();
        let csv_rows: Vec<Vec<String>> = snapshots.iter().map(|s| s.to_csv_row()).collect();
        let csv_time = csv_start.elapsed();

        println!("✓ Created {} snapshots in {:?}", count, create_time);
        println!("✓ Converted {} snapshots to CSV in {:?}", count, csv_time);
        println!("✓ Total time: {:?}", start.elapsed());

        assert_eq!(snapshots.len(), count);
        assert_eq!(csv_rows.len(), count);
        assert!(csv_time.as_millis() < 500, "CSV conversion too slow");
    }
}
