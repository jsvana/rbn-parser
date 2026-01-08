//! Configuration file support for RBN Parser.
//!
//! Loads settings from `~/.config/rbn-parser/config.toml` on Linux
//! (or platform-appropriate location on other OSes).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

use crate::client::{RBN_HOST, RBN_PORT_CW};
use crate::filter::SpotFilter;

/// Configuration for spot storage.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Default maximum entries per filter (used when filter doesn't specify).
    pub default_max_kept_entries: usize,

    /// Global maximum size for all stored spots (human-readable, e.g., "10MB").
    #[serde(deserialize_with = "deserialize_size")]
    pub global_max_size: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            default_max_kept_entries: 50,
            global_max_size: 10 * 1024 * 1024, // 10MB
        }
    }
}

/// Deserialize a human-readable size string like "10MB" into bytes.
fn deserialize_size<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    parse_size(&s).map_err(serde::de::Error::custom)
}

/// Parse a human-readable size string into bytes.
///
/// Supports: B, KB, MB, GB (case-insensitive).
/// Examples: "100", "500KB", "10MB", "1GB"
pub fn parse_size(s: &str) -> Result<usize, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size string".to_string());
    }

    let s_upper = s.to_ascii_uppercase();

    // Find where the numeric part ends
    let num_end = s_upper
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s_upper.len());

    let (num_str, unit) = s_upper.split_at(num_end);
    let num: f64 = num_str
        .parse()
        .map_err(|_| format!("invalid number in size: {}", s))?;

    let multiplier: usize = match unit.trim() {
        "" | "B" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        _ => return Err(format!("unknown size unit: {}", unit)),
    };

    Ok((num * multiplier as f64) as usize)
}

/// Application configuration loaded from TOML file.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Callsign to use for RBN login.
    pub callsign: String,

    /// RBN server hostname.
    pub host: String,

    /// RBN server port.
    pub port: u16,

    /// Connection timeout in seconds.
    pub connect_timeout: u64,

    /// Read timeout in seconds.
    pub read_timeout: u64,

    /// Whether to automatically reconnect on disconnect.
    pub reconnect: bool,

    /// Only count CW spots (ignore RTTY/digital).
    pub cw_only: bool,

    /// Print statistics every N seconds.
    pub stats_interval: u64,

    /// Enable Prometheus metrics HTTP endpoint.
    pub metrics_enabled: bool,

    /// Port for Prometheus metrics HTTP endpoint.
    pub metrics_port: u16,

    /// Spot filters for selective output.
    pub filters: Vec<SpotFilter>,

    /// Optional storage configuration for keeping recent matched spots.
    pub storage: Option<StorageConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            callsign: "N0CALL".to_string(),
            host: RBN_HOST.to_string(),
            port: RBN_PORT_CW,
            connect_timeout: 30,
            read_timeout: 120,
            reconnect: true,
            cw_only: true,
            stats_interval: 30,
            metrics_enabled: false,
            metrics_port: 9090,
            filters: Vec::new(),
            storage: None,
        }
    }
}

impl Config {
    /// Load configuration from the default config file location.
    ///
    /// Returns default config if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load() -> Result<Self> {
        match Self::config_path() {
            Some(path) if path.exists() => {
                let content = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config file: {}", path.display()))?;
                toml::from_str(&content)
                    .with_context(|| format!("Invalid TOML in config file: {}", path.display()))
            }
            _ => Ok(Config::default()),
        }
    }

    /// Returns the path to the config file.
    pub fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("rbn-parser/config.toml"))
    }

    /// Validate all configuration settings.
    ///
    /// Returns an error if any filters have invalid patterns.
    pub fn validate(&self) -> Result<()> {
        for (i, filter) in self.filters.iter().enumerate() {
            filter
                .validate()
                .map_err(|e| anyhow::anyhow!("Invalid filter [{}]: {}", i, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.callsign, "N0CALL");
        assert_eq!(config.host, RBN_HOST);
        assert_eq!(config.port, RBN_PORT_CW);
        assert!(config.reconnect);
        assert!(config.cw_only);
    }

    #[test]
    fn test_parse_minimal_toml() {
        let toml = r#"
            callsign = "W6JSV"
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.callsign, "W6JSV");
        // Other fields should use defaults
        assert_eq!(config.host, RBN_HOST);
        assert_eq!(config.port, RBN_PORT_CW);
    }

    #[test]
    fn test_parse_full_toml() {
        let toml = r#"
            callsign = "W6JSV"
            host = "custom.server.net"
            port = 7001
            connect_timeout = 60
            read_timeout = 180
            reconnect = false
            cw_only = false
            stats_interval = 60
            metrics_enabled = true
            metrics_port = 9091
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.callsign, "W6JSV");
        assert_eq!(config.host, "custom.server.net");
        assert_eq!(config.port, 7001);
        assert_eq!(config.connect_timeout, 60);
        assert_eq!(config.read_timeout, 180);
        assert!(!config.reconnect);
        assert!(!config.cw_only);
        assert_eq!(config.stats_interval, 60);
        assert!(config.metrics_enabled);
        assert_eq!(config.metrics_port, 9091);
    }

    #[test]
    fn test_parse_filters() {
        let toml = r#"
            callsign = "W6JSV"

            [[filters]]
            dx_call = "W6*"

            [[filters]]
            bands = ["20m", "40m"]
            min_snr = 15
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.filters.len(), 2);
        assert_eq!(config.filters[0].dx_call, Some("W6*".to_string()));
        assert_eq!(
            config.filters[1].bands,
            Some(vec!["20m".to_string(), "40m".to_string()])
        );
        assert_eq!(config.filters[1].min_snr, Some(15));
    }

    #[test]
    fn test_default_metrics_disabled() {
        let config = Config::default();
        assert!(!config.metrics_enabled);
        assert_eq!(config.metrics_port, 9090);
        assert!(config.filters.is_empty());
    }

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
        assert_eq!(parse_size("1K").unwrap(), 1024);
        assert_eq!(parse_size("10MB").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("10M").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        // Case insensitive
        assert_eq!(parse_size("10mb").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("10Mb").unwrap(), 10 * 1024 * 1024);
        // With whitespace
        assert_eq!(parse_size("  10MB  ").unwrap(), 10 * 1024 * 1024);
        // Decimal
        assert_eq!(parse_size("1.5MB").unwrap(), (1.5 * 1024.0 * 1024.0) as usize);
    }

    #[test]
    fn test_parse_size_errors() {
        assert!(parse_size("").is_err());
        assert!(parse_size("abc").is_err());
        assert!(parse_size("10TB").is_err()); // TB not supported
    }

    #[test]
    fn test_parse_storage_config() {
        let toml = r#"
            callsign = "W6JSV"

            [storage]
            default_max_kept_entries = 100
            global_max_size = "50MB"

            [[filters]]
            name = "w6_calls"
            dx_call = "W6*"
            max_kept_entries = 200
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        let storage = config.storage.unwrap();
        assert_eq!(storage.default_max_kept_entries, 100);
        assert_eq!(storage.global_max_size, 50 * 1024 * 1024);
        assert_eq!(config.filters[0].name, Some("w6_calls".to_string()));
        assert_eq!(config.filters[0].max_kept_entries, Some(200));
    }

    #[test]
    fn test_no_storage_config() {
        let config = Config::default();
        assert!(config.storage.is_none());
    }
}
