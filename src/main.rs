//! RBN Parser CLI - Stream and analyze CW spots from the Reverse Beacon Network.

use anyhow::Result;
use clap::Parser;
use rbn_parser::{
    Config,
    client::{RbnClient, RbnClientConfig, RbnEvent},
    metrics::start_metrics_server,
    parser::{is_cw_spot, looks_like_spot, parse_spot},
    polo::PoloNotesManager,
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

    // Initialize PoLo notes manager if any filters use polo_notes_url
    let polo_manager = Arc::new(PoloNotesManager::from_filters(&config.filters));
    if !polo_manager.is_empty() {
        info!("PoLo notes: fetching initial callsigns...");
        polo_manager.refresh_all().await;

        // Start background refresh
        let pm = Arc::clone(&polo_manager);
        tokio::spawn(async move {
            pm.start_background_refresh();
        });
    }

    // Create shared statistics
    let stats = Arc::new(SpotStats::new());

    // Create spot storage if configured
    let storage = config.storage.as_ref().map(|storage_config| {
        let pm = if polo_manager.is_empty() {
            None
        } else {
            Some(Arc::clone(&polo_manager))
        };
        Arc::new(SpotStorage::new(storage_config, config.filters.clone(), pm))
    });

    if storage.is_some() {
        info!(
            "Spot storage enabled with {} filter(s)",
            config.filters.len()
        );
    }

    // Start HTTP server if enabled
    if config.server_enabled {
        info!("HTTP server listening on port {}", config.server_port);
        let stats_for_server = Arc::clone(&stats);
        let storage_for_server = storage.clone();
        let server_port = config.server_port;
        tokio::spawn(async move {
            if let Err(e) =
                start_metrics_server(server_port, stats_for_server, storage_for_server).await
            {
                error!("Failed to start HTTP server: {}", e);
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

    // Start stats printer (disabled if stats_interval is 0)
    let stats_interval = config.stats_interval;
    if stats_interval > 0 {
        let stats_clone = Arc::clone(&stats);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(stats_interval));
            loop {
                interval.tick().await;
                println!("\n{}", stats_clone.summary());
            }
        });
    }

    // Configure and start RBN client
    let cw_only = config.cw_only;
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
                        process_line(&line, &stats, cw_only, args.verbose, storage.as_deref());
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

            // Print if verbose mode is enabled
            if verbose {
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

        process_line(line, &stats, true, false, None);

        assert_eq!(
            stats.total_spots.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn test_process_line_non_spot() {
        let stats = SpotStats::new();
        let line = "Welcome to the Reverse Beacon Network";

        process_line(line, &stats, true, false, None);

        assert_eq!(
            stats
                .non_spot_lines
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
