//! RBN Parser - A Rust library and server for parsing CW spots from the Reverse Beacon Network.
//!
//! This crate provides:
//! - A robust nom-based parser for RBN spot messages
//! - Statistics tracking with HDR histograms
//! - An async telnet client for streaming spots
//!
//! # Example
//!
//! ```rust,no_run
//! use rbn_parser::{parser::parse_spot, stats::SpotStats};
//!
//! let line = "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z";
//! let spot = parse_spot(line).expect("Failed to parse spot");
//!
//! let stats = SpotStats::new();
//! stats.record_spot(&spot);
//!
//! println!("{}", stats.summary());
//! ```

pub mod client;
pub mod config;
pub mod filter;
pub mod metrics;
pub mod parser;
pub mod spot;
pub mod stats;
pub mod storage;

pub use client::{RbnClient, RbnClientConfig, RbnEvent};
pub use config::{Config, StorageConfig};
pub use filter::{SpotFilter, any_filter_matches};
pub use parser::{ParseError, is_cw_spot, looks_like_spot, parse_spot};
pub use spot::{CwSpot, Mode, SpotType};
pub use stats::{SpotStats, StatsSummary};
pub use storage::SpotStorage;
