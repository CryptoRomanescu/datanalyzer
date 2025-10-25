use std::fmt;
use std::error::Error;
use std::io;

#[derive(Debug)]
pub enum AppError {
    ConfigError(String),
    RpcError(String),
    DecodingError(String),
    CsvError(String),
    PriceError(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            AppError::RpcError(msg) => write!(f, "RPC error: {}", msg),
            AppError::DecodingError(msg) => write!(f, "Decoding error: {}", msg),
            AppError::CsvError(msg) => write!(f, "CSV error: {}", msg),
            AppError::PriceError(msg) => write!(f, "Price error: {}", msg),
        }
    }
}

impl Error for AppError {}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        AppError::ConfigError(error.to_string())
    }
}

impl From<csv::Error> for AppError {
    fn from(error: csv::Error) -> Self {
        AppError::CsvError(error.to_string())
    }
}

impl From<std::num::ParseIntError> for AppError {
    fn from(error: std::num::ParseIntError) -> Self {
        AppError::DecodingError(error.to_string())
    }
}

impl From<std::string::ParseError> for AppError {
    fn from(error: std::string::ParseError) -> Self {
        AppError::ConfigError(error.to_string())
    }
}
