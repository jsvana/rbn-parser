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
}
