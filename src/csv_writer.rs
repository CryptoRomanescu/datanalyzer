/// Minimal production-ready CSV writer module.
///
/// Provides a buffered CSV writer with headers and proper flushing.

use crate::error::AppError;
use csv::Writer;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// Buffered CSV writer with headers
pub struct CsvWriter {
    writer: Writer<BufWriter<File>>,
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
        let file = File::create(path)
            .map_err(|e| AppError::IoError(format!("Failed to create CSV file: {}", e)))?;
        
        let buf_writer = BufWriter::new(file);
        let mut writer = Writer::from_writer(buf_writer);
        
        // Write headers
        writer
            .write_record(headers)
            .map_err(|e| AppError::IoError(format!("Failed to write CSV headers: {}", e)))?;
        
        Ok(Self { writer })
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
        self.writer
            .write_record(record)
            .map_err(|e| AppError::IoError(format!("Failed to write CSV record: {}", e)))
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
            .map_err(|e| AppError::IoError(format!("Failed to flush CSV writer: {}", e)))
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
        file.read_to_string(&mut contents).expect("Failed to read file");
        
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
            
            writer.write_record(&["Alice", "30", "NYC"])
                .expect("Failed to write record");
            writer.write_record(&["Bob", "25", "LA"])
                .expect("Failed to write record");
            
            writer.flush().expect("Failed to flush");
        }
        
        // Verify file contents
        let mut file = File::open(test_file).expect("File should exist");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("Failed to read file");
        
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
            let mut writer = CsvWriter::new(test_file, &["x", "y"])
                .expect("Failed to create CSV writer");
            
            writer.write_record(&["1", "2"])
                .expect("Failed to write record");
            
            // Don't explicitly flush - let Drop handle it
        }
        
        // Verify data was flushed
        let mut file = File::open(test_file).expect("File should exist");
        let mut contents = String::new();
        file.read_to_string(&mut contents).expect("Failed to read file");
        
        assert!(contents.contains("1,2"));
        
        // Clean up
        let _ = fs::remove_file(test_file);
    }
}
