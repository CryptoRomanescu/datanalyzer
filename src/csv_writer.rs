/// Minimal production-ready CSV writer module.
///
/// Provides a buffered CSV writer with headers, proper flushing, append mode,
/// directory creation, file rotation, and batching capabilities.
use crate::error::AppError;
use csv::Writer;
use std::fs::{self, File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Configuration for CSV writer behavior
#[derive(Debug, Clone)]
pub struct CsvWriterConfig {
    /// Enable append mode (default: false)
    pub append: bool,
    /// Maximum file size in bytes before rotation (0 = no rotation)
    pub max_file_size: u64,
    /// Maximum file age in seconds before rotation (0 = no rotation)
    pub max_file_age: u64,
    /// Batch size for flushing (0 = flush every write)
    pub batch_size: usize,
    /// Batch time in milliseconds for flushing (0 = no time-based flush)
    pub batch_time_ms: u64,
}

impl Default for CsvWriterConfig {
    fn default() -> Self {
        Self {
            append: false,
            max_file_size: 10 * 1024 * 1024, // 10MB
            max_file_age: 3600,              // 1 hour
            batch_size: 100,
            batch_time_ms: 5000, // 5 seconds
        }
    }
}

impl CsvWriterConfig {
    /// Create a new configuration builder
    pub fn builder() -> CsvWriterConfigBuilder {
        CsvWriterConfigBuilder::default()
    }
}

/// Builder for CsvWriterConfig
#[derive(Default)]
pub struct CsvWriterConfigBuilder {
    append: Option<bool>,
    max_file_size: Option<u64>,
    max_file_age: Option<u64>,
    batch_size: Option<usize>,
    batch_time_ms: Option<u64>,
}

impl CsvWriterConfigBuilder {
    pub fn append(mut self, append: bool) -> Self {
        self.append = Some(append);
        self
    }

    pub fn max_file_size(mut self, size: u64) -> Self {
        self.max_file_size = Some(size);
        self
    }

    pub fn max_file_age(mut self, age: u64) -> Self {
        self.max_file_age = Some(age);
        self
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size);
        self
    }

    pub fn batch_time_ms(mut self, time: u64) -> Self {
        self.batch_time_ms = Some(time);
        self
    }

    pub fn build(self) -> CsvWriterConfig {
        let default = CsvWriterConfig::default();
        CsvWriterConfig {
            append: self.append.unwrap_or(default.append),
            max_file_size: self.max_file_size.unwrap_or(default.max_file_size),
            max_file_age: self.max_file_age.unwrap_or(default.max_file_age),
            batch_size: self.batch_size.unwrap_or(default.batch_size),
            batch_time_ms: self.batch_time_ms.unwrap_or(default.batch_time_ms),
        }
    }
}

/// Buffered CSV writer with headers, rotation, and batching support
pub struct CsvWriter {
    writer: Writer<BufWriter<File>>,
    path: PathBuf,
    headers: Vec<String>,
    config: CsvWriterConfig,
    records_written: usize,
    file_created_at: SystemTime,
    last_flush: SystemTime,
}

impl CsvWriter {
    /// Create a new CSV writer with the specified file path and headers
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the CSV file
    /// * `headers` - Column headers for the CSV file
    ///
    /// # Returns
    ///
    /// * `Ok(CsvWriter)` - New CSV writer instance
    /// * `Err(AppError)` - If file creation or header writing fails
    pub fn new<P: AsRef<Path>>(path: P, headers: &[&str]) -> Result<Self, AppError> {
        Self::with_config(path, headers, CsvWriterConfig::default())
    }

    /// Create a new CSV writer with custom configuration
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the CSV file
    /// * `headers` - Column headers for the CSV file
    /// * `config` - Configuration for writer behavior
    ///
    /// # Returns
    ///
    /// * `Ok(CsvWriter)` - New CSV writer instance
    /// * `Err(AppError)` - If file creation or header writing fails
    pub fn with_config<P: AsRef<Path>>(
        path: P,
        headers: &[&str],
        config: CsvWriterConfig,
    ) -> Result<Self, AppError> {
        let path = path.as_ref();

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::IoError(format!("Failed to create directory {:?}: {}", parent, e))
            })?;
        }

        let file_exists = path.exists();
        let should_write_headers = !file_exists || !config.append;

        // Open file with appropriate mode
        let file = if config.append {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| AppError::IoError(format!("Failed to open CSV file: {}", e)))?
        } else {
            File::create(path)
                .map_err(|e| AppError::IoError(format!("Failed to create CSV file: {}", e)))?
        };

        let buf_writer = BufWriter::new(file);
        let mut writer = Writer::from_writer(buf_writer);

        // Write headers only if needed
        if should_write_headers {
            writer
                .write_record(headers)
                .map_err(|e| AppError::IoError(format!("Failed to write CSV headers: {}", e)))?;
        }

        let now = SystemTime::now();

        Ok(Self {
            writer,
            path: path.to_path_buf(),
            headers: headers.iter().map(|s| s.to_string()).collect(),
            config,
            records_written: 0,
            file_created_at: now,
            last_flush: now,
        })
    }

    /// Write a record to the CSV file
    ///
    /// # Arguments
    ///
    /// * `record` - Data to write as a row
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Record written successfully
    /// * `Err(AppError)` - If writing fails
    pub fn write_record<I, T>(&mut self, record: I) -> Result<(), AppError>
    where
        I: IntoIterator<Item = T>,
        T: AsRef<[u8]>,
    {
        // Check if rotation is needed
        if self.should_rotate()? {
            self.rotate()?;
        }

        self.writer
            .write_record(record)
            .map_err(|e| AppError::IoError(format!("Failed to write CSV record: {}", e)))?;

        self.records_written += 1;

        // Check if we should flush based on batch size or time
        if self.should_flush()? {
            self.flush()?;
        }

        Ok(())
    }

    /// Check if the file should be rotated
    fn should_rotate(&self) -> Result<bool, AppError> {
        // Check file size rotation
        if self.config.max_file_size > 0 {
            let metadata = fs::metadata(&self.path)
                .map_err(|e| AppError::IoError(format!("Failed to get file metadata: {}", e)))?;

            if metadata.len() >= self.config.max_file_size {
                return Ok(true);
            }
        }

        // Check file age rotation
        if self.config.max_file_age > 0 {
            let elapsed = self
                .file_created_at
                .elapsed()
                .map_err(|e| AppError::IoError(format!("Failed to calculate file age: {}", e)))?;

            if elapsed.as_secs() >= self.config.max_file_age {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Check if the writer should flush
    fn should_flush(&self) -> Result<bool, AppError> {
        // Check batch size
        if self.config.batch_size > 0 && self.records_written >= self.config.batch_size {
            return Ok(true);
        }

        // Check batch time
        if self.config.batch_time_ms > 0 {
            let elapsed = self
                .last_flush
                .elapsed()
                .map_err(|e| AppError::IoError(format!("Failed to calculate flush time: {}", e)))?;

            if elapsed.as_millis() >= self.config.batch_time_ms as u128 {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Rotate the CSV file by creating a new file with timestamp suffix
    fn rotate(&mut self) -> Result<(), AppError> {
        // Flush current data
        self.flush()?;

        // Generate rotated filename
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AppError::IoError(format!("Failed to get timestamp: {}", e)))?
            .as_secs();

        let rotated_path = self.generate_rotated_path(timestamp)?;

        // Rename current file
        fs::rename(&self.path, &rotated_path).map_err(|e| {
            AppError::IoError(format!(
                "Failed to rotate file from {:?} to {:?}: {}",
                self.path, rotated_path, e
            ))
        })?;

        // Create new file
        let file = File::create(&self.path)
            .map_err(|e| AppError::IoError(format!("Failed to create new CSV file: {}", e)))?;

        let buf_writer = BufWriter::new(file);
        let mut writer = Writer::from_writer(buf_writer);

        // Write headers to new file
        let header_refs: Vec<&str> = self.headers.iter().map(|s| s.as_str()).collect();
        writer.write_record(&header_refs).map_err(|e| {
            AppError::IoError(format!("Failed to write CSV headers after rotation: {}", e))
        })?;

        self.writer = writer;
        self.records_written = 0;
        self.file_created_at = SystemTime::now();

        Ok(())
    }

    /// Generate a rotated file path with timestamp
    fn generate_rotated_path(&self, timestamp: u64) -> Result<PathBuf, AppError> {
        let stem = self
            .path
            .file_stem()
            .ok_or_else(|| AppError::IoError("Invalid file path".to_string()))?
            .to_str()
            .ok_or_else(|| AppError::IoError("Invalid UTF-8 in file stem".to_string()))?;

        let extension = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("csv");

        let rotated_name = format!("{}_{}.{}", stem, timestamp, extension);
        let mut rotated_path = self.path.clone();
        rotated_path.set_file_name(rotated_name);

        Ok(rotated_path)
    }

    /// Flush the writer to ensure all data is written to disk
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Flush successful
    /// * `Err(AppError)` - If flushing fails
    pub fn flush(&mut self) -> Result<(), AppError> {
        self.writer
            .flush()
            .map_err(|e| AppError::IoError(format!("Failed to flush CSV writer: {}", e)))?;
        self.last_flush = SystemTime::now();
        self.records_written = 0;
        Ok(())
    }

    /// Get the current file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the number of records written since last flush
    pub fn records_written(&self) -> usize {
        self.records_written
    }
}

impl Drop for CsvWriter {
    fn drop(&mut self) {
        // Ensure data is flushed when writer is dropped
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_csv_writer_creates_file_with_headers() {
        let test_file = "/tmp/test_csv_writer.csv";

        // Clean up if file exists
        let _ = fs::remove_file(test_file);

        {
            let mut writer = CsvWriter::new(test_file, &["col1", "col2", "col3"])
                .expect("Failed to create CSV writer");
            writer.flush().expect("Failed to flush");
        }

        // Verify file exists and contains headers
        let mut file = File::open(test_file).expect("File should exist");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read file");

        assert!(contents.starts_with("col1,col2,col3"));

        // Clean up
        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_csv_writer_write_records() {
        let test_file = "/tmp/test_csv_writer_records.csv";

        // Clean up if file exists
        let _ = fs::remove_file(test_file);

        {
            let mut writer = CsvWriter::new(test_file, &["name", "age", "city"])
                .expect("Failed to create CSV writer");

            writer
                .write_record(&["Alice", "30", "NYC"])
                .expect("Failed to write record");
            writer
                .write_record(&["Bob", "25", "LA"])
                .expect("Failed to write record");

            writer.flush().expect("Failed to flush");
        }

        // Verify file contents
        let mut file = File::open(test_file).expect("File should exist");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read file");

        assert!(contents.contains("Alice,30,NYC"));
        assert!(contents.contains("Bob,25,LA"));

        // Clean up
        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_csv_writer_auto_flush_on_drop() {
        let test_file = "/tmp/test_csv_writer_drop.csv";

        // Clean up if file exists
        let _ = fs::remove_file(test_file);

        {
            let mut writer =
                CsvWriter::new(test_file, &["x", "y"]).expect("Failed to create CSV writer");

            writer
                .write_record(&["1", "2"])
                .expect("Failed to write record");

            // Don't explicitly flush - let Drop handle it
        }

        // Verify data was flushed
        let mut file = File::open(test_file).expect("File should exist");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read file");

        assert!(contents.contains("1,2"));

        // Clean up
        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_csv_writer_append_mode() {
        let test_file = "/tmp/test_csv_writer_append.csv";

        // Clean up if file exists
        let _ = fs::remove_file(test_file);

        // Create initial file
        {
            let mut writer =
                CsvWriter::new(test_file, &["col1", "col2"]).expect("Failed to create CSV writer");
            writer
                .write_record(&["val1", "val2"])
                .expect("Failed to write record");
            writer.flush().expect("Failed to flush");
        }

        // Append to existing file
        {
            let config = CsvWriterConfig::builder().append(true).build();
            let mut writer = CsvWriter::with_config(test_file, &["col1", "col2"], config)
                .expect("Failed to create CSV writer in append mode");
            writer
                .write_record(&["val3", "val4"])
                .expect("Failed to write record");
            writer.flush().expect("Failed to flush");
        }

        // Verify both records exist
        let contents = fs::read_to_string(test_file).expect("Failed to read file");
        assert!(contents.contains("val1,val2"));
        assert!(contents.contains("val3,val4"));

        // Should only have one header line
        let header_count = contents.matches("col1,col2").count();
        assert_eq!(header_count, 1);

        // Clean up
        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_csv_writer_create_dir_all() {
        let test_dir = "/tmp/test_csv_nested/subdir";
        let test_file = format!("{}/test.csv", test_dir);

        // Clean up if directory exists
        let _ = fs::remove_dir_all("/tmp/test_csv_nested");

        {
            let mut writer =
                CsvWriter::new(&test_file, &["a", "b"]).expect("Failed to create CSV writer");
            writer.flush().expect("Failed to flush");
        }

        // Verify directory and file were created
        assert!(Path::new(test_dir).exists());
        assert!(Path::new(&test_file).exists());

        // Clean up
        let _ = fs::remove_dir_all("/tmp/test_csv_nested");
    }

    #[test]
    fn test_csv_writer_rotation_by_size() {
        let test_file = "/tmp/test_csv_rotation_size.csv";

        // Clean up
        let _ = fs::remove_file(test_file);

        // Create writer with very small max file size (200 bytes)
        let config = CsvWriterConfig::builder()
            .max_file_size(200)
            .batch_size(1) // Flush after each record to check size
            .batch_time_ms(0) // Disable time-based flushing
            .max_file_age(0) // Disable age-based rotation
            .build();

        {
            let mut writer = CsvWriter::with_config(test_file, &["data"], config)
                .expect("Failed to create CSV writer");

            // Write enough data to trigger rotation
            for i in 0..30 {
                writer
                    .write_record(&[format!("data_value_with_long_text_{}", i)])
                    .expect("Failed to write record");
            }

            writer.flush().expect("Failed to flush");
        }

        // Check if rotated file was created
        let rotated_files: Vec<_> = fs::read_dir("/tmp")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("test_csv_rotation_size_")
                    && e.file_name().to_str().unwrap().ends_with(".csv")
            })
            .collect();

        assert!(
            !rotated_files.is_empty(),
            "Expected at least one rotated file"
        );

        // Clean up
        let _ = fs::remove_file(test_file);
        for entry in rotated_files {
            let _ = fs::remove_file(entry.path());
        }
    }

    #[test]
    fn test_csv_writer_rotation_by_age() {
        let test_file = "/tmp/test_csv_rotation_age.csv";

        // Clean up
        let _ = fs::remove_file(test_file);

        // Create writer with 1 second max age
        let config = CsvWriterConfig::builder()
            .max_file_age(1)
            .max_file_size(0) // Disable size rotation
            .batch_size(0) // Disable batch flushing
            .build();

        {
            let mut writer = CsvWriter::with_config(test_file, &["data"], config)
                .expect("Failed to create CSV writer");

            writer
                .write_record(&["initial"])
                .expect("Failed to write record");
            writer.flush().expect("Failed to flush");

            // Wait for file to age
            thread::sleep(Duration::from_secs(2));

            // This should trigger rotation
            writer
                .write_record(&["after_rotation"])
                .expect("Failed to write record");
            writer.flush().expect("Failed to flush");
        }

        // Check if rotated file was created
        let rotated_files: Vec<_> = fs::read_dir("/tmp")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("test_csv_rotation_age_")
                    && e.file_name().to_str().unwrap().ends_with(".csv")
            })
            .collect();

        assert!(
            !rotated_files.is_empty(),
            "Expected at least one rotated file"
        );

        // Clean up
        let _ = fs::remove_file(test_file);
        for entry in rotated_files {
            let _ = fs::remove_file(entry.path());
        }
    }

    #[test]
    fn test_csv_writer_batch_flush() {
        let test_file = "/tmp/test_csv_batch.csv";

        // Clean up
        let _ = fs::remove_file(test_file);

        let config = CsvWriterConfig::builder()
            .batch_size(3)
            .max_file_size(0) // Disable rotation
            .max_file_age(0) // Disable rotation
            .build();

        {
            let mut writer = CsvWriter::with_config(test_file, &["data"], config)
                .expect("Failed to create CSV writer");

            // Write 2 records - should not flush yet
            writer.write_record(&["1"]).expect("Failed to write");
            writer.write_record(&["2"]).expect("Failed to write");
            assert_eq!(writer.records_written(), 2);

            // Write 3rd record - should trigger flush
            writer.write_record(&["3"]).expect("Failed to write");
            assert_eq!(writer.records_written(), 0); // Reset after flush
        }

        // Clean up
        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_csv_writer_config_builder() {
        let config = CsvWriterConfig::builder()
            .append(true)
            .max_file_size(1024)
            .max_file_age(3600)
            .batch_size(50)
            .batch_time_ms(1000)
            .build();

        assert_eq!(config.append, true);
        assert_eq!(config.max_file_size, 1024);
        assert_eq!(config.max_file_age, 3600);
        assert_eq!(config.batch_size, 50);
        assert_eq!(config.batch_time_ms, 1000);
    }

    #[test]
    fn test_csv_writer_config_default() {
        let config = CsvWriterConfig::default();

        assert_eq!(config.append, false);
        assert_eq!(config.max_file_size, 10 * 1024 * 1024);
        assert_eq!(config.max_file_age, 3600);
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.batch_time_ms, 5000);
    }

    #[test]
    fn test_csv_writer_no_data_loss_on_rotation() {
        let test_file = "/tmp/test_csv_no_data_loss.csv";

        // Clean up
        let _ = fs::remove_file(test_file);

        let config = CsvWriterConfig::builder()
            .max_file_size(150)
            .batch_size(0)
            .build();

        {
            let mut writer = CsvWriter::with_config(test_file, &["id", "value"], config)
                .expect("Failed to create CSV writer");

            // Write records that will trigger rotation
            for i in 0..30 {
                writer
                    .write_record(&[format!("{}", i), format!("value_{}", i)])
                    .expect("Failed to write record");
            }

            writer.flush().expect("Failed to flush");
        }

        // Count total records across all files
        let mut total_records = 0;

        // Count in main file
        if let Ok(contents) = fs::read_to_string(test_file) {
            total_records += contents.lines().count() - 1; // -1 for header
        }

        // Count in rotated files
        let rotated_files: Vec<_> = fs::read_dir("/tmp")
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("test_csv_no_data_loss_")
                    && e.file_name().to_str().unwrap().ends_with(".csv")
            })
            .collect();

        for entry in &rotated_files {
            if let Ok(contents) = fs::read_to_string(entry.path()) {
                total_records += contents.lines().count() - 1; // -1 for header
            }
        }

        // We wrote 30 records, should have 30 records across all files
        assert_eq!(total_records, 30, "Data loss detected during rotation");

        // Clean up
        let _ = fs::remove_file(test_file);
        for entry in rotated_files {
            let _ = fs::remove_file(entry.path());
        }
    }

    #[test]
    fn test_csv_writer_performance_large_volume() {
        let test_file = "/tmp/test_csv_performance.csv";

        // Clean up
        let _ = fs::remove_file(test_file);

        let config = CsvWriterConfig::builder()
            .batch_size(1000)
            .max_file_size(0) // Disable rotation for this test
            .max_file_age(0)
            .build();

        let start = std::time::Instant::now();

        {
            let mut writer = CsvWriter::with_config(
                test_file,
                &[
                    "timestamp",
                    "pool",
                    "reserve_base",
                    "reserve_quote",
                    "price",
                ],
                config,
            )
            .expect("Failed to create CSV writer");

            // Write 10,000 records
            for i in 0..10_000 {
                writer
                    .write_record(&[
                        format!("{}", i),
                        format!("pool_{}", i % 100),
                        format!("{}", i * 1000),
                        format!("{}", i * 2000),
                        format!("{:.6}", i as f64 / 100.0),
                    ])
                    .expect("Failed to write record");
            }

            writer.flush().expect("Failed to flush");
        }

        let elapsed = start.elapsed();

        // Verify all records were written
        let contents = fs::read_to_string(test_file).expect("Failed to read file");
        let line_count = contents.lines().count();
        assert_eq!(line_count, 10_001); // 10,000 records + 1 header

        // Performance should be reasonable (less than 5 seconds for 10k records)
        assert!(
            elapsed.as_secs() < 5,
            "Performance test took too long: {:?}",
            elapsed
        );

        println!("Performance test: 10,000 records written in {:?}", elapsed);

        // Clean up
        let _ = fs::remove_file(test_file);
    }
}
