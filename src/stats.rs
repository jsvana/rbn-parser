//! Statistics tracking for RBN spots.
//!
//! This module provides structures for tracking various metrics about
//! parsed CW spots, including counts, size distributions, and breakdowns
//! by band, spotter, and other dimensions.

use hdrhistogram::Histogram;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::spot::{CwSpot, Mode, SpotType};

/// Thread-safe statistics collector for RBN spots.
#[derive(Debug)]
pub struct SpotStats {
    /// Total number of spots parsed successfully
    pub total_spots: AtomicU64,

    /// Total number of CW-only spots
    pub cw_spots: AtomicU64,

    /// Total number of lines that failed to parse
    pub parse_failures: AtomicU64,

    /// Total number of lines that were not spots (filtered early)
    pub non_spot_lines: AtomicU64,

    /// Total bytes of raw input processed
    pub bytes_processed: AtomicU64,

    /// Histogram of spot sizes (JSON serialized size)
    size_histogram: RwLock<Histogram<u64>>,

    /// Histogram of SNR values
    snr_histogram: RwLock<Histogram<u64>>,

    /// Histogram of WPM values
    wpm_histogram: RwLock<Histogram<u64>>,

    /// Spots per band
    spots_by_band: RwLock<HashMap<String, u64>>,

    /// Spots per mode
    spots_by_mode: RwLock<HashMap<Mode, u64>>,

    /// Spots per type (CQ, BEACON, etc.)
    spots_by_type: RwLock<HashMap<SpotType, u64>>,

    /// Top spotters (skimmers)
    top_spotters: RwLock<HashMap<String, u64>>,

    /// When stats collection started
    start_time: Instant,
}

impl SpotStats {
    /// Create a new statistics collector.
    pub fn new() -> Self {
        Self {
            total_spots: AtomicU64::new(0),
            cw_spots: AtomicU64::new(0),
            parse_failures: AtomicU64::new(0),
            non_spot_lines: AtomicU64::new(0),
            bytes_processed: AtomicU64::new(0),
            // Size histogram: 1 byte to 10KB, 3 significant figures
            size_histogram: RwLock::new(
                Histogram::new_with_bounds(1, 10_000, 3).expect("Failed to create size histogram"),
            ),
            // SNR histogram: 0 to 60 dB (we'll add 30 to handle negatives)
            snr_histogram: RwLock::new(
                Histogram::new_with_bounds(1, 100, 2).expect("Failed to create SNR histogram"),
            ),
            // WPM histogram: 1 to 60 WPM
            wpm_histogram: RwLock::new(
                Histogram::new_with_bounds(1, 100, 2).expect("Failed to create WPM histogram"),
            ),
            spots_by_band: RwLock::new(HashMap::new()),
            spots_by_mode: RwLock::new(HashMap::new()),
            spots_by_type: RwLock::new(HashMap::new()),
            top_spotters: RwLock::new(HashMap::new()),
            start_time: Instant::now(),
        }
    }

    /// Record a successfully parsed spot.
    pub fn record_spot(&self, spot: &CwSpot) {
        self.total_spots.fetch_add(1, Ordering::Relaxed);

        if matches!(spot.mode, Mode::Cw) {
            self.cw_spots.fetch_add(1, Ordering::Relaxed);
        }

        // Record size distribution
        let size = spot.json_size() as u64;
        if let Ok(mut hist) = self.size_histogram.write() {
            let _ = hist.record(size.max(1));
        }

        // Record SNR distribution (offset by 30 to handle negatives)
        let snr_offset = (spot.snr_db + 30).max(0) as u64;
        if let Ok(mut hist) = self.snr_histogram.write() {
            let _ = hist.record(snr_offset.clamp(1, 99));
        }

        // Record WPM distribution
        if let Ok(mut hist) = self.wpm_histogram.write() {
            let _ = hist.record((spot.wpm as u64).clamp(1, 99));
        }

        // Record by band
        if let Some(band) = spot.band()
            && let Ok(mut map) = self.spots_by_band.write()
        {
            *map.entry(band.to_string()).or_insert(0) += 1;
        }

        // Record by mode
        if let Ok(mut map) = self.spots_by_mode.write() {
            *map.entry(spot.mode).or_insert(0) += 1;
        }

        // Record by type
        if let Ok(mut map) = self.spots_by_type.write() {
            *map.entry(spot.spot_type).or_insert(0) += 1;
        }

        // Record spotter
        if let Ok(mut map) = self.top_spotters.write() {
            *map.entry(spot.spotter.clone()).or_insert(0) += 1;
        }
    }

    /// Record a parse failure.
    pub fn record_parse_failure(&self) {
        self.parse_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a non-spot line.
    pub fn record_non_spot(&self) {
        self.non_spot_lines.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bytes processed.
    pub fn record_bytes(&self, bytes: u64) {
        self.bytes_processed.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Get the elapsed time since stats collection started.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get the current spots per second rate.
    pub fn spots_per_second(&self) -> f64 {
        let elapsed = self.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.total_spots.load(Ordering::Relaxed) as f64 / elapsed
        } else {
            0.0
        }
    }

    /// Generate a summary report.
    pub fn summary(&self) -> StatsSummary {
        let elapsed = self.elapsed();
        let total = self.total_spots.load(Ordering::Relaxed);
        let cw = self.cw_spots.load(Ordering::Relaxed);
        let failures = self.parse_failures.load(Ordering::Relaxed);
        let non_spots = self.non_spot_lines.load(Ordering::Relaxed);
        let bytes = self.bytes_processed.load(Ordering::Relaxed);

        let size_percentiles = self
            .size_histogram
            .read()
            .map(|h| HistogramPercentiles {
                p50: h.value_at_quantile(0.50),
                p90: h.value_at_quantile(0.90),
                p99: h.value_at_quantile(0.99),
                min: h.min(),
                max: h.max(),
                mean: h.mean(),
            })
            .ok();

        let snr_percentiles = self
            .snr_histogram
            .read()
            .map(|h| HistogramPercentiles {
                // Subtract 30 to get back to real SNR values
                p50: h.value_at_quantile(0.50).saturating_sub(30),
                p90: h.value_at_quantile(0.90).saturating_sub(30),
                p99: h.value_at_quantile(0.99).saturating_sub(30),
                min: h.min().saturating_sub(30),
                max: h.max().saturating_sub(30),
                mean: h.mean() - 30.0,
            })
            .ok();

        let wpm_percentiles = self
            .wpm_histogram
            .read()
            .map(|h| HistogramPercentiles {
                p50: h.value_at_quantile(0.50),
                p90: h.value_at_quantile(0.90),
                p99: h.value_at_quantile(0.99),
                min: h.min(),
                max: h.max(),
                mean: h.mean(),
            })
            .ok();

        let spots_by_band = self
            .spots_by_band
            .read()
            .map(|m| m.clone())
            .unwrap_or_default();

        let spots_by_mode = self
            .spots_by_mode
            .read()
            .map(|m| m.iter().map(|(k, v)| (k.to_string(), *v)).collect())
            .unwrap_or_default();

        let spots_by_type = self
            .spots_by_type
            .read()
            .map(|m| m.iter().map(|(k, v)| (k.to_string(), *v)).collect())
            .unwrap_or_default();

        // Get top 10 spotters
        let top_spotters = self
            .top_spotters
            .read()
            .map(|m| {
                let mut vec: Vec<_> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
                vec.sort_by(|a, b| b.1.cmp(&a.1));
                vec.truncate(10);
                vec
            })
            .unwrap_or_default();

        StatsSummary {
            elapsed_secs: elapsed.as_secs_f64(),
            total_spots: total,
            cw_spots: cw,
            parse_failures: failures,
            non_spot_lines: non_spots,
            bytes_processed: bytes,
            spots_per_second: self.spots_per_second(),
            size_percentiles,
            snr_percentiles,
            wpm_percentiles,
            spots_by_band,
            spots_by_mode,
            spots_by_type,
            top_spotters,
        }
    }
}

impl Default for SpotStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Percentile values from a histogram.
#[derive(Debug, Clone, Serialize)]
pub struct HistogramPercentiles {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub min: u64,
    pub max: u64,
    pub mean: f64,
}

/// Summary of collected statistics.
#[derive(Debug, Clone, Serialize)]
pub struct StatsSummary {
    pub elapsed_secs: f64,
    pub total_spots: u64,
    pub cw_spots: u64,
    pub parse_failures: u64,
    pub non_spot_lines: u64,
    pub bytes_processed: u64,
    pub spots_per_second: f64,
    pub size_percentiles: Option<HistogramPercentiles>,
    pub snr_percentiles: Option<HistogramPercentiles>,
    pub wpm_percentiles: Option<HistogramPercentiles>,
    pub spots_by_band: HashMap<String, u64>,
    pub spots_by_mode: HashMap<String, u64>,
    pub spots_by_type: HashMap<String, u64>,
    pub top_spotters: Vec<(String, u64)>,
}

impl std::fmt::Display for StatsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "═══════════════════════════════════════════════════════")?;
        writeln!(f, "                  RBN SPOT STATISTICS")?;
        writeln!(f, "═══════════════════════════════════════════════════════")?;
        writeln!(f)?;
        writeln!(f, "Runtime: {:.1}s", self.elapsed_secs)?;
        writeln!(f, "Total spots: {}", self.total_spots)?;
        writeln!(
            f,
            "CW spots: {} ({:.1}%)",
            self.cw_spots,
            if self.total_spots > 0 {
                self.cw_spots as f64 / self.total_spots as f64 * 100.0
            } else {
                0.0
            }
        )?;
        writeln!(f, "Parse failures: {}", self.parse_failures)?;
        writeln!(f, "Non-spot lines: {}", self.non_spot_lines)?;
        writeln!(f, "Bytes processed: {} KB", self.bytes_processed / 1024)?;
        writeln!(f, "Rate: {:.1} spots/sec", self.spots_per_second)?;
        writeln!(f)?;

        if let Some(ref p) = self.size_percentiles {
            writeln!(f, "Size Distribution (bytes):")?;
            writeln!(f, "  Min: {}, Max: {}, Mean: {:.1}", p.min, p.max, p.mean)?;
            writeln!(f, "  P50: {}, P90: {}, P99: {}", p.p50, p.p90, p.p99)?;
            writeln!(f)?;
        }

        if let Some(ref p) = self.snr_percentiles {
            writeln!(f, "SNR Distribution (dB):")?;
            writeln!(
                f,
                "  Min: {}, Max: {}, Mean: {:.1}",
                p.min as i64, p.max as i64, p.mean
            )?;
            writeln!(
                f,
                "  P50: {}, P90: {}, P99: {}",
                p.p50 as i64, p.p90 as i64, p.p99 as i64
            )?;
            writeln!(f)?;
        }

        if let Some(ref p) = self.wpm_percentiles {
            writeln!(f, "WPM Distribution:")?;
            writeln!(f, "  Min: {}, Max: {}, Mean: {:.1}", p.min, p.max, p.mean)?;
            writeln!(f, "  P50: {}, P90: {}, P99: {}", p.p50, p.p90, p.p99)?;
            writeln!(f)?;
        }

        if !self.spots_by_band.is_empty() {
            writeln!(f, "Spots by Band:")?;
            let mut bands: Vec<_> = self.spots_by_band.iter().collect();
            bands.sort_by(|a, b| b.1.cmp(a.1));
            for (band, count) in bands {
                writeln!(f, "  {}: {}", band, count)?;
            }
            writeln!(f)?;
        }

        if !self.spots_by_type.is_empty() {
            writeln!(f, "Spots by Type:")?;
            let mut types: Vec<_> = self.spots_by_type.iter().collect();
            types.sort_by(|a, b| b.1.cmp(a.1));
            for (spot_type, count) in types {
                writeln!(f, "  {}: {}", spot_type, count)?;
            }
            writeln!(f)?;
        }

        if !self.top_spotters.is_empty() {
            writeln!(f, "Top 10 Spotters:")?;
            for (i, (spotter, count)) in self.top_spotters.iter().enumerate() {
                writeln!(f, "  {}. {}: {}", i + 1, spotter, count)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn make_test_spot() -> CwSpot {
        CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: 14025.0,
            dx_call: "W1AW".to_string(),
            mode: Mode::Cw,
            snr_db: 15,
            wpm: 22,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn test_record_spot() {
        let stats = SpotStats::new();
        let spot = make_test_spot();

        stats.record_spot(&spot);

        assert_eq!(stats.total_spots.load(Ordering::Relaxed), 1);
        assert_eq!(stats.cw_spots.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_spots_per_second() {
        let stats = SpotStats::new();

        for _ in 0..100 {
            stats.record_spot(&make_test_spot());
        }

        // Rate should be positive after recording spots
        assert!(stats.spots_per_second() > 0.0);
    }

    #[test]
    fn test_summary_generation() {
        let stats = SpotStats::new();

        for _ in 0..10 {
            stats.record_spot(&make_test_spot());
        }
        stats.record_parse_failure();
        stats.record_non_spot();
        stats.record_bytes(1000);

        let summary = stats.summary();

        assert_eq!(summary.total_spots, 10);
        assert_eq!(summary.cw_spots, 10);
        assert_eq!(summary.parse_failures, 1);
        assert_eq!(summary.non_spot_lines, 1);
        assert_eq!(summary.bytes_processed, 1000);
    }
}
