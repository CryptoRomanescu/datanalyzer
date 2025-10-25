use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::fs;
use std::path::Path;
use serde::Deserialize;
use crate::error::AppError;
use crate::models::DexType;

/// TOML configuration structure for a pool
#[derive(Debug, Clone, Deserialize)]
pub struct PoolConfigToml {
    pub pool_address: String,
    pub dex_type: String,
    pub token_mint: String,
}

impl PoolConfigToml {
    /// Convert PoolConfigToml to runtime PoolConfig
    pub fn into_pool_config(self) -> Result<PoolConfig, AppError> {
        let pool_address = Pubkey::from_str(&self.pool_address)
            .map_err(|e| AppError::ConfigError(format!("Invalid pool address '{}': {}", self.pool_address, e)))?;
        
        let dex_type = self.dex_type.parse::<DexType>()?;
        
        let token_mint = Pubkey::from_str(&self.token_mint)
            .map_err(|e| AppError::ConfigError(format!("Invalid token mint '{}': {}", self.token_mint, e)))?;
        
        PoolConfig::new(pool_address, dex_type, token_mint)
    }
}

/// Application configuration loaded from TOML
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub rpc_url: String,
    pub rpc_ws_url: String,
    pub output_dir: String,
    pub snapshot_interval_ms: u64,
    pub pools: Vec<PoolConfigToml>,
}

impl AppConfig {
    /// Load configuration from a TOML file
    pub fn load(path: &str) -> Result<Self, AppError> {
        // Check if file exists
        if !Path::new(path).exists() {
            return Err(AppError::ConfigError(format!("Configuration file not found: {}", path)));
        }
        
        // Read file contents
        let contents = fs::read_to_string(path)
            .map_err(|e| AppError::ConfigError(format!("Failed to read config file '{}': {}", path, e)))?;
        
        // Parse TOML
        let config: AppConfig = toml::from_str(&contents)
            .map_err(|e| AppError::ConfigError(format!("Failed to parse TOML: {}", e)))?;
        
        // Validate configuration
        config.validate()?;
        
        Ok(config)
    }
    
    /// Validate the configuration
    fn validate(&self) -> Result<(), AppError> {
        // Validate RPC URL
        if self.rpc_url.is_empty() {
            return Err(AppError::ConfigError("RPC URL cannot be empty".to_string()));
        }
        
        if !self.rpc_url.starts_with("http://") && !self.rpc_url.starts_with("https://") {
            return Err(AppError::ConfigError(
                format!("RPC URL must start with http:// or https://, got: {}", self.rpc_url)
            ));
        }
        
        // Validate RPC WebSocket URL
        if self.rpc_ws_url.is_empty() {
            return Err(AppError::ConfigError("RPC WebSocket URL cannot be empty".to_string()));
        }
        
        if !self.rpc_ws_url.starts_with("ws://") && !self.rpc_ws_url.starts_with("wss://") {
            return Err(AppError::ConfigError(
                format!("RPC WebSocket URL must start with ws:// or wss://, got: {}", self.rpc_ws_url)
            ));
        }
        
        // Validate output directory
        if self.output_dir.is_empty() {
            return Err(AppError::ConfigError("Output directory cannot be empty".to_string()));
        }
        
        // Validate snapshot interval
        if self.snapshot_interval_ms == 0 {
            return Err(AppError::ConfigError("Snapshot interval must be greater than 0".to_string()));
        }
        
        if self.snapshot_interval_ms < 100 {
            return Err(AppError::ConfigError(
                format!("Snapshot interval too small ({}ms), minimum is 100ms", self.snapshot_interval_ms)
            ));
        }
        
        // Validate pools list
        if self.pools.is_empty() {
            return Err(AppError::ConfigError("At least one pool must be configured".to_string()));
        }
        
        Ok(())
    }
    
    /// Convert AppConfig to runtime configuration with validated PoolConfig instances
    pub fn into_runtime_config(self) -> Result<RuntimeConfig, AppError> {
        let pools: Result<Vec<PoolConfig>, AppError> = self.pools
            .into_iter()
            .map(|pool_toml| pool_toml.into_pool_config())
            .collect();
        
        Ok(RuntimeConfig {
            rpc_url: self.rpc_url,
            rpc_ws_url: self.rpc_ws_url,
            output_dir: self.output_dir,
            snapshot_interval_ms: self.snapshot_interval_ms,
            pools: pools?,
        })
    }
}

/// Runtime configuration with validated and parsed values
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub rpc_url: String,
    pub rpc_ws_url: String,
    pub output_dir: String,
    pub snapshot_interval_ms: u64,
    pub pools: Vec<PoolConfig>,
}

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pool_address: Pubkey,
    dex_type: DexType,
    token_mint: Pubkey,
}

impl PoolConfig {
    pub fn new(pool_address: Pubkey, dex_type: DexType, token_mint: Pubkey) -> Result<Self, AppError> {
        let config = PoolConfig {
            pool_address,
            dex_type,
            token_mint,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.pool_address == Pubkey::default() {
            return Err(AppError::ConfigError(
                "Pool address cannot be default/zero pubkey".to_string(),
            ));
        }
        
        if self.token_mint == Pubkey::default() {
            return Err(AppError::ConfigError(
                "Token mint cannot be default/zero pubkey".to_string(),
            ));
        }
        
        Ok(())
    }

    pub fn get_csv_filename(&self) -> String {
        format!(
            "{}_{}.csv",
            self.dex_type.to_string().to_lowercase(),
            &self.pool_address.to_string()[..8]
        )
    }

    pub fn pool_address(&self) -> &Pubkey {
        &self.pool_address
    }

    pub fn dex_type(&self) -> DexType {
        self.dex_type
    }

    pub fn token_mint(&self) -> &Pubkey {
        &self.token_mint
    }

    pub fn builder() -> PoolConfigBuilder {
        PoolConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct PoolConfigBuilder {
    pool_address: Option<Pubkey>,
    dex_type: Option<DexType>,
    token_mint: Option<Pubkey>,
}

impl PoolConfigBuilder {
    pub fn pool_address(mut self, address: &str) -> Result<Self, AppError> {
        self.pool_address = Some(
            Pubkey::from_str(address)
                .map_err(|e| AppError::ConfigError(format!("Invalid pool address: {}", e)))?,
        );
        Ok(self)
    }

    pub fn pool_address_pubkey(mut self, pubkey: Pubkey) -> Self {
        self.pool_address = Some(pubkey);
        self
    }

    pub fn dex_type(mut self, dex_type: DexType) -> Self {
        self.dex_type = Some(dex_type);
        self
    }

    pub fn dex_type_str(mut self, dex_type: &str) -> Result<Self, AppError> {
        self.dex_type = Some(dex_type.parse()?);
        Ok(self)
    }

    pub fn token_mint(mut self, mint: &str) -> Result<Self, AppError> {
        self.token_mint = Some(
            Pubkey::from_str(mint)
                .map_err(|e| AppError::ConfigError(format!("Invalid token mint: {}", e)))?,
        );
        Ok(self)
    }

    pub fn token_mint_pubkey(mut self, pubkey: Pubkey) -> Self {
        self.token_mint = Some(pubkey);
        self
    }

    pub fn build(self) -> Result<PoolConfig, AppError> {
        let pool_address = self
            .pool_address
            .ok_or_else(|| AppError::ConfigError("Pool address is required".to_string()))?;
        let dex_type = self
            .dex_type
            .ok_or_else(|| AppError::ConfigError("DEX type is required".to_string()))?;
        let token_mint = self
            .token_mint
            .ok_or_else(|| AppError::ConfigError("Token mint is required".to_string()))?;

        PoolConfig::new(pool_address, dex_type, token_mint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_config_validation() {
        let result = PoolConfig::new(
            Pubkey::default(),
            DexType::Raydium,
            Pubkey::new_unique(),
        );
        assert!(result.is_err());

        let result = PoolConfig::new(
            Pubkey::new_unique(),
            DexType::Raydium,
            Pubkey::default(),
        );
        assert!(result.is_err());

        let result = PoolConfig::new(
            Pubkey::new_unique(),
            DexType::Raydium,
            Pubkey::new_unique(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_pool_config_csv_filename() {
        let pool_addr = Pubkey::new_unique();
        let config = PoolConfig::new(
            pool_addr,
            DexType::PumpFun,
            Pubkey::new_unique(),
        )
        .unwrap();

        let filename = config.get_csv_filename();
        assert!(filename.starts_with("pumpfun_"));
        assert!(filename.ends_with(".csv"));
    }

    #[test]
    fn test_pool_config_builder() {
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();
        
        let config = PoolConfig::builder()
            .pool_address_pubkey(pool_addr)
            .dex_type(DexType::Raydium)
            .token_mint_pubkey(token_mint)
            .build()
            .unwrap();

        assert_eq!(config.pool_address(), &pool_addr);
        assert_eq!(config.dex_type(), DexType::Raydium);
        assert_eq!(config.token_mint(), &token_mint);
    }

    #[test]
    fn test_pool_config_builder_missing_field() {
        let result = PoolConfig::builder()
            .pool_address_pubkey(Pubkey::new_unique())
            .dex_type(DexType::Raydium)
            .build();
        
        assert!(result.is_err());
    }

    #[test]
    fn test_pool_config_builder_string_input() {
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();
        
        let result = PoolConfig::builder()
            .pool_address(&pool_addr.to_string())
            .unwrap()
            .dex_type_str("raydium")
            .unwrap()
            .token_mint(&token_mint.to_string())
            .unwrap()
            .build();

        assert!(result.is_ok());
    }

    #[test]
    fn test_pool_config_toml_conversion() {
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();
        
        let toml_config = PoolConfigToml {
            pool_address: pool_addr.to_string(),
            dex_type: "raydium".to_string(),
            token_mint: token_mint.to_string(),
        };
        
        let result = toml_config.into_pool_config();
        assert!(result.is_ok());
        
        let config = result.unwrap();
        assert_eq!(config.pool_address(), &pool_addr);
        assert_eq!(config.dex_type(), DexType::Raydium);
        assert_eq!(config.token_mint(), &token_mint);
    }

    #[test]
    fn test_pool_config_toml_invalid_address() {
        let toml_config = PoolConfigToml {
            pool_address: "invalid_address".to_string(),
            dex_type: "raydium".to_string(),
            token_mint: Pubkey::new_unique().to_string(),
        };
        
        let result = toml_config.into_pool_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_pool_config_toml_invalid_dex_type() {
        let toml_config = PoolConfigToml {
            pool_address: Pubkey::new_unique().to_string(),
            dex_type: "unknown_dex".to_string(),
            token_mint: Pubkey::new_unique().to_string(),
        };
        
        let result = toml_config.into_pool_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_app_config_load_missing_file() {
        let result = AppConfig::load("/nonexistent/config.toml");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }

    #[test]
    fn test_app_config_validation_empty_rpc_url() {
        let config = AppConfig {
            rpc_url: "".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 1000,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("RPC URL cannot be empty"));
    }

    #[test]
    fn test_app_config_validation_invalid_rpc_url() {
        let config = AppConfig {
            rpc_url: "ftp://example.com".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 1000,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("must start with http://"));
    }

    #[test]
    fn test_app_config_validation_invalid_ws_url() {
        let config = AppConfig {
            rpc_url: "https://example.com".to_string(),
            rpc_ws_url: "http://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 1000,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("must start with ws://"));
    }

    #[test]
    fn test_app_config_validation_empty_output_dir() {
        let config = AppConfig {
            rpc_url: "https://example.com".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "".to_string(),
            snapshot_interval_ms: 1000,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Output directory cannot be empty"));
    }

    #[test]
    fn test_app_config_validation_zero_interval() {
        let config = AppConfig {
            rpc_url: "https://example.com".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 0,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("must be greater than 0"));
    }

    #[test]
    fn test_app_config_validation_too_small_interval() {
        let config = AppConfig {
            rpc_url: "https://example.com".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 50,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("too small"));
    }

    #[test]
    fn test_app_config_validation_no_pools() {
        let config = AppConfig {
            rpc_url: "https://example.com".to_string(),
            rpc_ws_url: "wss://example.com".to_string(),
            output_dir: "./output".to_string(),
            snapshot_interval_ms: 1000,
            pools: vec![],
        };
        
        let result = config.validate();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("At least one pool"));
    }

    #[test]
    fn test_app_config_into_runtime_config() {
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();
        
        let config = AppConfig {
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            rpc_ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
            output_dir: "./snapshots".to_string(),
            snapshot_interval_ms: 5000,
            pools: vec![
                PoolConfigToml {
                    pool_address: pool_addr.to_string(),
                    dex_type: "raydium".to_string(),
                    token_mint: token_mint.to_string(),
                },
            ],
        };
        
        let result = config.into_runtime_config();
        assert!(result.is_ok());
        
        let runtime = result.unwrap();
        assert_eq!(runtime.rpc_url, "https://api.mainnet-beta.solana.com");
        assert_eq!(runtime.pools.len(), 1);
        assert_eq!(runtime.pools[0].dex_type(), DexType::Raydium);
    }

    #[test]
    fn test_app_config_runtime_invalid_pool() {
        let config = AppConfig {
            rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            rpc_ws_url: "wss://api.mainnet-beta.solana.com".to_string(),
            output_dir: "./snapshots".to_string(),
            snapshot_interval_ms: 5000,
            pools: vec![
                PoolConfigToml {
                    pool_address: "invalid".to_string(),
                    dex_type: "raydium".to_string(),
                    token_mint: Pubkey::new_unique().to_string(),
                },
            ],
        };
        
        let result = config.into_runtime_config();
        assert!(result.is_err());
    }

    #[test]
    fn test_app_config_load_from_toml() {
        use std::io::Write;
        
        let pool_addr = Pubkey::new_unique();
        let token_mint = Pubkey::new_unique();
        
        let toml_content = format!(r#"
rpc_url = "https://api.mainnet-beta.solana.com"
rpc_ws_url = "wss://api.mainnet-beta.solana.com"
output_dir = "./snapshots"
snapshot_interval_ms = 5000

[[pools]]
pool_address = "{}"
dex_type = "raydium"
token_mint = "{}"

[[pools]]
pool_address = "{}"
dex_type = "pumpfun"
token_mint = "{}"
"#, pool_addr, token_mint, Pubkey::new_unique(), Pubkey::new_unique());
        
        // Create a temporary file
        let temp_path = "/tmp/test_config.toml";
        let mut file = std::fs::File::create(temp_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();
        
        // Load config
        let result = AppConfig::load(temp_path);
        assert!(result.is_ok());
        
        let config = result.unwrap();
        assert_eq!(config.rpc_url, "https://api.mainnet-beta.solana.com");
        assert_eq!(config.pools.len(), 2);
        
        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }

    #[test]
    fn test_app_config_load_invalid_toml() {
        use std::io::Write;
        
        let toml_content = r#"
rpc_url = "https://api.mainnet-beta.solana.com"
# Invalid TOML - missing closing quote
rpc_ws_url = "wss://api.mainnet-beta.solana.com
"#;
        
        // Create a temporary file
        let temp_path = "/tmp/test_invalid_config.toml";
        let mut file = std::fs::File::create(temp_path).unwrap();
        file.write_all(toml_content.as_bytes()).unwrap();
        
        // Load config should fail
        let result = AppConfig::load(temp_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to parse TOML"));
        
        // Cleanup
        std::fs::remove_file(temp_path).ok();
    }


}
