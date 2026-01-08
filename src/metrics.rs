//! Prometheus metrics HTTP server.
//!
//! Exposes RBN statistics in Prometheus text format via HTTP endpoint.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use tokio::net::TcpListener;
use tracing::info;

use crate::stats::SpotStats;

/// Start the Prometheus metrics HTTP server.
///
/// Runs in the background and serves metrics at `/metrics`.
/// Returns an error if the server fails to bind to the port.
pub async fn start_metrics_server(
    port: u16,
    stats: Arc<SpotStats>,
) -> Result<(), std::io::Error> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .with_state(stats);

    let listener = TcpListener::bind(addr).await?;
    info!("Prometheus metrics server listening on http://{}/metrics", addr);

    axum::serve(listener, app)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Health check endpoint.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Prometheus metrics endpoint.
async fn metrics_handler(State(stats): State<Arc<SpotStats>>) -> impl IntoResponse {
    let output = format_prometheus_metrics(&stats);
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}

/// Format statistics as Prometheus text format.
fn format_prometheus_metrics(stats: &SpotStats) -> String {
    let summary = stats.summary();
    let mut output = String::with_capacity(4096);

    // Help and type declarations + metrics

    // Uptime
    output.push_str("# HELP rbn_uptime_seconds Time since the parser started\n");
    output.push_str("# TYPE rbn_uptime_seconds gauge\n");
    output.push_str(&format!("rbn_uptime_seconds {:.3}\n", summary.elapsed_secs));

    // Total spots by mode
    output.push_str("# HELP rbn_spots_total Total number of spots parsed\n");
    output.push_str("# TYPE rbn_spots_total counter\n");
    for (mode, count) in &summary.spots_by_mode {
        output.push_str(&format!("rbn_spots_total{{mode=\"{}\"}} {}\n", mode, count));
    }
    // Also output total if modes map is empty
    if summary.spots_by_mode.is_empty() {
        output.push_str(&format!("rbn_spots_total {}\n", summary.total_spots));
    }

    // Parse failures
    output.push_str("# HELP rbn_parse_failures_total Number of lines that failed to parse\n");
    output.push_str("# TYPE rbn_parse_failures_total counter\n");
    output.push_str(&format!(
        "rbn_parse_failures_total {}\n",
        summary.parse_failures
    ));

    // Non-spot lines
    output.push_str("# HELP rbn_non_spot_lines_total Number of non-spot lines received\n");
    output.push_str("# TYPE rbn_non_spot_lines_total counter\n");
    output.push_str(&format!(
        "rbn_non_spot_lines_total {}\n",
        summary.non_spot_lines
    ));

    // Bytes processed
    output.push_str("# HELP rbn_bytes_processed_total Total bytes of raw input processed\n");
    output.push_str("# TYPE rbn_bytes_processed_total counter\n");
    output.push_str(&format!(
        "rbn_bytes_processed_total {}\n",
        summary.bytes_processed
    ));

    // Spots per second rate
    output.push_str("# HELP rbn_spots_per_second Current spot processing rate\n");
    output.push_str("# TYPE rbn_spots_per_second gauge\n");
    output.push_str(&format!(
        "rbn_spots_per_second {:.3}\n",
        summary.spots_per_second
    ));

    // Spots by band
    output.push_str("# HELP rbn_spots_by_band_total Spots broken down by amateur band\n");
    output.push_str("# TYPE rbn_spots_by_band_total counter\n");
    for (band, count) in &summary.spots_by_band {
        output.push_str(&format!(
            "rbn_spots_by_band_total{{band=\"{}\"}} {}\n",
            band, count
        ));
    }

    // Spots by type
    output.push_str("# HELP rbn_spots_by_type_total Spots broken down by spot type\n");
    output.push_str("# TYPE rbn_spots_by_type_total counter\n");
    for (spot_type, count) in &summary.spots_by_type {
        output.push_str(&format!(
            "rbn_spots_by_type_total{{type=\"{}\"}} {}\n",
            spot_type, count
        ));
    }

    // SNR histogram buckets
    if let Some(ref snr) = summary.snr_percentiles {
        output.push_str("# HELP rbn_snr_db SNR distribution in decibels\n");
        output.push_str("# TYPE rbn_snr_db summary\n");
        output.push_str(&format!(
            "rbn_snr_db{{quantile=\"0.5\"}} {}\n",
            snr.p50 as i64
        ));
        output.push_str(&format!(
            "rbn_snr_db{{quantile=\"0.9\"}} {}\n",
            snr.p90 as i64
        ));
        output.push_str(&format!(
            "rbn_snr_db{{quantile=\"0.99\"}} {}\n",
            snr.p99 as i64
        ));
        output.push_str(&format!("rbn_snr_db_count {}\n", summary.total_spots));
    }

    // WPM histogram buckets
    if let Some(ref wpm) = summary.wpm_percentiles {
        output.push_str("# HELP rbn_wpm WPM (words per minute) distribution\n");
        output.push_str("# TYPE rbn_wpm summary\n");
        output.push_str(&format!("rbn_wpm{{quantile=\"0.5\"}} {}\n", wpm.p50));
        output.push_str(&format!("rbn_wpm{{quantile=\"0.9\"}} {}\n", wpm.p90));
        output.push_str(&format!("rbn_wpm{{quantile=\"0.99\"}} {}\n", wpm.p99));
        output.push_str(&format!("rbn_wpm_count {}\n", summary.total_spots));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_prometheus_metrics_empty() {
        let stats = SpotStats::new();
        let output = format_prometheus_metrics(&stats);

        assert!(output.contains("rbn_uptime_seconds"));
        assert!(output.contains("rbn_parse_failures_total 0"));
        assert!(output.contains("rbn_non_spot_lines_total 0"));
        assert!(output.contains("rbn_bytes_processed_total 0"));
    }

    #[test]
    fn test_format_prometheus_metrics_with_data() {
        use crate::spot::{CwSpot, Mode, SpotType};
        use chrono::NaiveTime;

        let stats = SpotStats::new();

        let spot = CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: 14025.0,
            dx_call: "W1AW".to_string(),
            mode: Mode::Cw,
            snr_db: 15,
            wpm: 22,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        stats.record_spot(&spot);
        stats.record_bytes(100);

        let output = format_prometheus_metrics(&stats);

        assert!(output.contains("rbn_spots_total{mode=\"CW\"} 1"));
        assert!(output.contains("rbn_bytes_processed_total 100"));
        assert!(output.contains("rbn_spots_by_band_total{band=\"20m\"} 1"));
        assert!(output.contains("rbn_spots_by_type_total{type=\"CQ\"} 1"));
    }

    #[test]
    fn test_prometheus_format_validity() {
        let stats = SpotStats::new();
        let output = format_prometheus_metrics(&stats);

        // Check that each non-comment, non-empty line has proper format
        for line in output.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Should have metric name followed by value
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert!(
                parts.len() >= 2,
                "Invalid metric line: {}",
                line
            );
        }
    }
}
