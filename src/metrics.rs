//! Prometheus metrics HTTP server and REST API for spot retrieval.
//!
//! Exposes RBN statistics in Prometheus text format via HTTP endpoint,
//! plus REST API endpoints for retrieving stored spots.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::info;

use crate::stats::SpotStats;
use crate::storage::{SpotStorage, StoredSpot};

/// Shared state for the metrics server.
#[derive(Clone)]
pub struct MetricsState {
    stats: Arc<SpotStats>,
    storage: Option<Arc<SpotStorage>>,
}

/// Start the Prometheus metrics HTTP server.
///
/// Runs in the background and serves metrics at `/metrics`.
/// Returns an error if the server fails to bind to the port.
pub async fn start_metrics_server(
    port: u16,
    stats: Arc<SpotStats>,
    storage: Option<Arc<SpotStorage>>,
) -> Result<(), std::io::Error> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let state = MetricsState { stats, storage };

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(health_handler))
        .route("/spots/filters", get(list_filters_handler))
        .route("/spots/filter/{name}", get(get_spots_handler))
        .with_state(state);

    let listener = TcpListener::bind(addr).await?;
    info!(
        "Prometheus metrics server listening on http://{}/metrics",
        addr
    );

    axum::serve(listener, app)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Health check endpoint.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Prometheus metrics endpoint.
async fn metrics_handler(State(state): State<MetricsState>) -> impl IntoResponse {
    let output = format_prometheus_metrics(&state.stats, state.storage.as_deref());
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        output,
    )
}

/// Query parameters for the get spots endpoint.
#[derive(Deserialize)]
struct GetSpotsQuery {
    /// Return spots with sequence > this value.
    since: Option<u64>,
}

/// Response for the get spots endpoint.
#[derive(Serialize)]
struct GetSpotsResponse {
    /// Filter name.
    filter: String,
    /// List of spots with sequence numbers.
    spots: Vec<StoredSpot>,
    /// Latest sequence number in storage (0 if empty).
    latest_seq: u64,
    /// Count of spots evicted from this filter.
    overflow_count: u64,
}

/// List available filter names.
async fn list_filters_handler(State(state): State<MetricsState>) -> impl IntoResponse {
    match &state.storage {
        Some(storage) => {
            let names = storage.filter_names();
            (StatusCode::OK, Json(names)).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Storage not configured"})),
        )
            .into_response(),
    }
}

/// Get spots for a specific filter.
async fn get_spots_handler(
    State(state): State<MetricsState>,
    Path(name): Path<String>,
    Query(query): Query<GetSpotsQuery>,
) -> impl IntoResponse {
    let Some(storage) = &state.storage else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Storage not configured"})),
        )
            .into_response();
    };

    let Some(filter_storage_lock) = storage.get_filter_by_name(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Filter '{}' not found", name)})),
        )
            .into_response();
    };

    let filter_storage = filter_storage_lock.read().unwrap();
    let since = query.since.unwrap_or(0);
    let spots = filter_storage.get_spots_since(since);
    let latest_seq = filter_storage.latest_seq();
    let overflow_count = filter_storage.overflow_count.load(Relaxed);

    let response = GetSpotsResponse {
        filter: name,
        spots,
        latest_seq,
        overflow_count,
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Format statistics as Prometheus text format.
fn format_prometheus_metrics(stats: &SpotStats, storage: Option<&SpotStorage>) -> String {
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

    // Storage metrics (if storage is configured)
    if let Some(storage) = storage {
        format_storage_metrics(&mut output, storage);
    }

    output
}

/// Format storage metrics in Prometheus text format.
fn format_storage_metrics(output: &mut String, storage: &SpotStorage) {
    // Per-filter metrics
    output.push_str("# HELP rbn_filter_stored_spots Number of spots currently stored per filter\n");
    output.push_str("# TYPE rbn_filter_stored_spots gauge\n");

    output.push_str("# HELP rbn_filter_stored_bytes Bytes of stored spots per filter\n");
    output.push_str("# TYPE rbn_filter_stored_bytes gauge\n");

    output.push_str("# HELP rbn_filter_overflow_total Count of evicted spots per filter\n");
    output.push_str("# TYPE rbn_filter_overflow_total counter\n");

    output.push_str("# HELP rbn_filter_max_kept_entries Configured max entries per filter\n");
    output.push_str("# TYPE rbn_filter_max_kept_entries gauge\n");

    for (_, storage_lock) in storage.iter_storages() {
        let fs = storage_lock.read().unwrap();
        let name = &fs.name;

        output.push_str(&format!(
            "rbn_filter_stored_spots{{filter=\"{}\"}} {}\n",
            name,
            fs.len()
        ));
        output.push_str(&format!(
            "rbn_filter_stored_bytes{{filter=\"{}\"}} {}\n",
            name,
            fs.current_size_bytes.load(Relaxed)
        ));
        output.push_str(&format!(
            "rbn_filter_overflow_total{{filter=\"{}\"}} {}\n",
            name,
            fs.overflow_count.load(Relaxed)
        ));
        output.push_str(&format!(
            "rbn_filter_max_kept_entries{{filter=\"{}\"}} {}\n",
            name, fs.max_kept_entries
        ));
    }

    // Global storage metrics
    output.push_str("# HELP rbn_storage_total_bytes Total bytes across all filter storages\n");
    output.push_str("# TYPE rbn_storage_total_bytes gauge\n");
    output.push_str(&format!(
        "rbn_storage_total_bytes {}\n",
        storage.total_size_bytes.load(Relaxed)
    ));

    output.push_str("# HELP rbn_storage_global_max_bytes Configured global max storage size\n");
    output.push_str("# TYPE rbn_storage_global_max_bytes gauge\n");
    output.push_str(&format!(
        "rbn_storage_global_max_bytes {}\n",
        storage.global_max_size()
    ));

    output.push_str(
        "# HELP rbn_storage_global_evictions_total Count of evictions due to global limit\n",
    );
    output.push_str("# TYPE rbn_storage_global_evictions_total counter\n");
    output.push_str(&format!(
        "rbn_storage_global_evictions_total {}\n",
        storage.global_evictions.load(Relaxed)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_prometheus_metrics_empty() {
        let stats = SpotStats::new();
        let output = format_prometheus_metrics(&stats, None);

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

        let output = format_prometheus_metrics(&stats, None);

        assert!(output.contains("rbn_spots_total{mode=\"CW\"} 1"));
        assert!(output.contains("rbn_bytes_processed_total 100"));
        assert!(output.contains("rbn_spots_by_band_total{band=\"20m\"} 1"));
        assert!(output.contains("rbn_spots_by_type_total{type=\"CQ\"} 1"));
    }

    #[test]
    fn test_prometheus_format_validity() {
        let stats = SpotStats::new();
        let output = format_prometheus_metrics(&stats, None);

        // Check that each non-comment, non-empty line has proper format
        for line in output.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Should have metric name followed by value
            let parts: Vec<&str> = line.split_whitespace().collect();
            assert!(parts.len() >= 2, "Invalid metric line: {}", line);
        }
    }
}
