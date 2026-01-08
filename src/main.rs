//! RBN Parser CLI - Stream and analyze CW spots from the Reverse Beacon Network.

use anyhow::Result;
use clap::Parser;
use rbn_parser::{
    Config,
    client::{RbnClient, RbnClientConfig, RbnEvent},
    filter::{SpotFilter, any_filter_matches},
    metrics::start_metrics_server,
    parser::{is_cw_spot, looks_like_spot, parse_spot},
    stats::SpotStats,
    storage::SpotStorage,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

/// RBN Parser - Stream and analyze CW spots from the Reverse Beacon Network
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Print each parsed spot (verbose)
    #[arg(short, long)]
    verbose: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Maximum runtime in seconds (0 = unlimited)
    #[arg(long, default_value_t = 0)]
    max_runtime: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load()?;
    config.validate()?;

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&args.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    info!("RBN Parser starting...");
    if let Some(path) = Config::config_path() {
        info!("Config file: {}", path.display());
    }
    info!("Callsign: {}", config.callsign);
    info!("Server: {}:{}", config.host, config.port);
    if !config.filters.is_empty() {
        info!("Filters: {} configured", config.filters.len());
    }

    // Create shared statistics
    let stats = Arc::new(SpotStats::new());

    // Create spot storage if configured
    let storage = config.storage.as_ref().map(|storage_config| {
        Arc::new(SpotStorage::new(storage_config, config.filters.clone()))
    });

    if storage.is_some() {
        info!("Spot storage enabled with {} filter(s)", config.filters.len());
    }

    // Start metrics server if enabled
    if config.metrics_enabled {
        let stats_for_metrics = Arc::clone(&stats);
        let storage_for_metrics = storage.clone();
        let metrics_port = config.metrics_port;
        tokio::spawn(async move {
            if let Err(e) = start_metrics_server(metrics_port, stats_for_metrics, storage_for_metrics).await {
                error!("Failed to start metrics server: {}", e);
            }
        });
    }

    // Create shutdown signal
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    // Handle Ctrl+C
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        info!("Shutdown signal received");
        let _ = shutdown_tx_clone.send(true);
    });

    // Optional max runtime
    if args.max_runtime > 0 {
        let shutdown_tx_clone = shutdown_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(args.max_runtime)).await;
            info!("Max runtime reached");
            let _ = shutdown_tx_clone.send(true);
        });
    }

    // Start stats printer
    let stats_clone = Arc::clone(&stats);
    let stats_interval = config.stats_interval;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(stats_interval));
        loop {
            interval.tick().await;
            println!("\n{}", stats_clone.summary());
        }
    });

    // Configure and start RBN client
    let cw_only = config.cw_only;
    let filters = config.filters;
    let client_config = RbnClientConfig {
        host: config.host,
        port: config.port,
        callsign: config.callsign,
        connect_timeout: Duration::from_secs(config.connect_timeout),
        read_timeout: Duration::from_secs(config.read_timeout),
        auto_reconnect: config.reconnect,
        ..Default::default()
    };

    let client = RbnClient::new(client_config);
    let mut events = client.connect().await?;

    // Main event loop
    loop {
        tokio::select! {
            // Check for shutdown
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }

            // Process RBN events
            event = events.recv() => {
                match event {
                    Some(RbnEvent::Line(line)) => {
                        process_line(&line, &stats, cw_only, args.verbose, &filters, storage.as_deref());
                    }
                    Some(RbnEvent::Connected) => {
                        info!("Connected to RBN");
                    }
                    Some(RbnEvent::Disconnected(reason)) => {
                        warn!("Disconnected: {}", reason);
                    }
                    Some(RbnEvent::Error(e)) => {
                        error!("Error: {}", e);
                    }
                    None => {
                        // Channel closed
                        break;
                    }
                }
            }
        }
    }

    // Print final statistics
    println!("\n\nFINAL STATISTICS");
    println!("{}", stats.summary());

    Ok(())
}

/// Process a single line from the RBN feed.
fn process_line(
    line: &str,
    stats: &SpotStats,
    cw_only: bool,
    verbose: bool,
    filters: &[SpotFilter],
    storage: Option<&SpotStorage>,
) {
    stats.record_bytes(line.len() as u64);

    // Quick filter for non-spot lines
    if !looks_like_spot(line) {
        stats.record_non_spot();
        debug!("Non-spot line: {}", line);
        return;
    }

    // Try to parse the spot
    match parse_spot(line) {
        Ok(spot) => {
            // Filter for CW-only if requested
            if cw_only && !is_cw_spot(&spot) {
                debug!("Filtered non-CW spot: {:?}", spot.mode);
                return;
            }

            stats.record_spot(&spot);

            // Store in spot storage if configured (storage has its own filters)
            if let Some(storage) = storage {
                storage.try_store(&spot);
            }

            // Print if verbose or if spot matches any filter
            if verbose || any_filter_matches(filters, &spot) {
                println!("{}", spot);
            }
        }
        Err(e) => {
            stats.record_parse_failure();
            debug!("Parse error for '{}': {}", line, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_line_valid_spot() {
        let stats = SpotStats::new();
        let line = "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z";
        let filters: Vec<SpotFilter> = vec![];

        process_line(line, &stats, true, false, &filters, None);

        assert_eq!(
            stats.total_spots.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn test_process_line_non_spot() {
        let stats = SpotStats::new();
        let line = "Welcome to the Reverse Beacon Network";
        let filters: Vec<SpotFilter> = vec![];

        process_line(line, &stats, true, false, &filters, None);

        assert_eq!(
            stats
                .non_spot_lines
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
