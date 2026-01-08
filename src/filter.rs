//! Spot filtering for selective output.
//!
//! Allows configuring which spots to print based on various criteria
//! like callsign patterns, bands, SNR thresholds, etc.

use serde::Deserialize;

use crate::spot::{CwSpot, Mode, SpotType};

/// A filter for matching spots.
///
/// All specified fields must match (AND logic).
/// Use multiple filters for OR logic.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SpotFilter {
    /// Optional name for this filter (used in metrics labels).
    pub name: Option<String>,

    /// DX callsign pattern (supports `*` wildcard for prefix/suffix).
    pub dx_call: Option<String>,

    /// Spotter callsign pattern (supports `*` wildcard for prefix/suffix).
    pub spotter: Option<String>,

    /// Bands to match (e.g., "20m", "40m").
    pub bands: Option<Vec<String>>,

    /// Modes to match.
    pub modes: Option<Vec<Mode>>,

    /// Spot types to match.
    pub spot_types: Option<Vec<SpotType>>,

    /// Minimum SNR in dB.
    pub min_snr: Option<i32>,

    /// Maximum SNR in dB.
    pub max_snr: Option<i32>,

    /// Minimum WPM.
    pub min_wpm: Option<u16>,

    /// Maximum WPM.
    pub max_wpm: Option<u16>,

    /// Maximum number of spots to keep in storage for this filter.
    /// Overrides `default_max_kept_entries` from `[storage]` config.
    pub max_kept_entries: Option<usize>,
}

impl SpotFilter {
    /// Check if a spot matches this filter.
    ///
    /// All specified fields must match (AND logic).
    pub fn matches(&self, spot: &CwSpot) -> bool {
        // Check dx_call pattern
        if let Some(ref pattern) = self.dx_call
            && !matches_wildcard(pattern, &spot.dx_call)
        {
            return false;
        }

        // Check spotter pattern
        if let Some(ref pattern) = self.spotter
            && !matches_wildcard(pattern, &spot.spotter)
        {
            return false;
        }

        // Check bands
        if let Some(ref bands) = self.bands {
            match spot.band() {
                Some(band) if bands.iter().any(|b| b.eq_ignore_ascii_case(band)) => {}
                _ => return false,
            }
        }

        // Check modes
        if let Some(ref modes) = self.modes
            && !modes.contains(&spot.mode)
        {
            return false;
        }

        // Check spot types
        if let Some(ref spot_types) = self.spot_types
            && !spot_types.contains(&spot.spot_type)
        {
            return false;
        }

        // Check SNR range
        if let Some(min_snr) = self.min_snr
            && spot.snr_db < min_snr
        {
            return false;
        }
        if let Some(max_snr) = self.max_snr
            && spot.snr_db > max_snr
        {
            return false;
        }

        // Check WPM range
        if let Some(min_wpm) = self.min_wpm
            && spot.wpm < min_wpm
        {
            return false;
        }
        if let Some(max_wpm) = self.max_wpm
            && spot.wpm > max_wpm
        {
            return false;
        }

        true
    }

    /// Validate the filter configuration.
    ///
    /// Returns an error if any patterns are invalid.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ref pattern) = self.dx_call {
            validate_wildcard_pattern(pattern)?;
        }
        if let Some(ref pattern) = self.spotter {
            validate_wildcard_pattern(pattern)?;
        }
        Ok(())
    }
}

/// Check if any filter in the list matches the spot.
///
/// Returns `true` if at least one filter matches (OR logic).
/// Returns `false` if the list is empty.
pub fn any_filter_matches(filters: &[SpotFilter], spot: &CwSpot) -> bool {
    filters.iter().any(|f| f.matches(spot))
}

/// Match a string against a wildcard pattern.
///
/// Supports `*` as prefix or suffix wildcard (not both).
/// Matching is case-insensitive.
fn matches_wildcard(pattern: &str, value: &str) -> bool {
    let pattern_upper = pattern.to_ascii_uppercase();
    let value_upper = value.to_ascii_uppercase();

    if let Some(suffix) = pattern_upper.strip_prefix('*') {
        // Suffix match: "*JSV" matches "W6JSV"
        value_upper.ends_with(suffix)
    } else if let Some(prefix) = pattern_upper.strip_suffix('*') {
        // Prefix match: "W6*" matches "W6JSV"
        value_upper.starts_with(prefix)
    } else {
        // Exact match
        pattern_upper == value_upper
    }
}

/// Validate a wildcard pattern.
///
/// Returns an error if the pattern has wildcards in invalid positions.
fn validate_wildcard_pattern(pattern: &str) -> Result<(), String> {
    let wildcard_count = pattern.chars().filter(|&c| c == '*').count();

    if wildcard_count > 1 {
        return Err(format!(
            "Pattern '{}' has multiple wildcards; only one is allowed",
            pattern
        ));
    }

    if wildcard_count == 1 && !pattern.starts_with('*') && !pattern.ends_with('*') {
        return Err(format!(
            "Pattern '{}' has wildcard in middle; only prefix (*ABC) or suffix (ABC*) allowed",
            pattern
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn make_spot(dx_call: &str, spotter: &str, freq: f64, snr: i32, wpm: u16) -> CwSpot {
        CwSpot {
            spotter: spotter.to_string(),
            frequency_khz: freq,
            dx_call: dx_call.to_string(),
            mode: Mode::Cw,
            snr_db: snr,
            wpm,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn test_matches_wildcard_prefix() {
        assert!(matches_wildcard("W6*", "W6JSV"));
        assert!(matches_wildcard("W6*", "w6abc"));
        assert!(!matches_wildcard("W6*", "K6JSV"));
    }

    #[test]
    fn test_matches_wildcard_suffix() {
        assert!(matches_wildcard("*JSV", "W6JSV"));
        assert!(matches_wildcard("*jsv", "K1JSV"));
        assert!(!matches_wildcard("*JSV", "W6ABC"));
    }

    #[test]
    fn test_matches_wildcard_exact() {
        assert!(matches_wildcard("W6JSV", "W6JSV"));
        assert!(matches_wildcard("w6jsv", "W6JSV"));
        assert!(!matches_wildcard("W6JSV", "W6ABC"));
    }

    #[test]
    fn test_validate_wildcard_pattern() {
        assert!(validate_wildcard_pattern("W6*").is_ok());
        assert!(validate_wildcard_pattern("*JSV").is_ok());
        assert!(validate_wildcard_pattern("W6JSV").is_ok());
        assert!(validate_wildcard_pattern("*W6*").is_err());
        assert!(validate_wildcard_pattern("W*6").is_err());
    }

    #[test]
    fn test_filter_dx_call() {
        let filter = SpotFilter {
            dx_call: Some("W6*".to_string()),
            ..Default::default()
        };

        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)));
        assert!(!filter.matches(&make_spot("K1ABC", "EA5WU-#", 14025.0, 15, 20)));
    }

    #[test]
    fn test_filter_band() {
        let filter = SpotFilter {
            bands: Some(vec!["20m".to_string(), "40m".to_string()]),
            ..Default::default()
        };

        // 14025 kHz is 20m
        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)));
        // 7025 kHz is 40m
        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 7025.0, 15, 20)));
        // 21025 kHz is 15m
        assert!(!filter.matches(&make_spot("W6JSV", "EA5WU-#", 21025.0, 15, 20)));
    }

    #[test]
    fn test_filter_snr_range() {
        let filter = SpotFilter {
            min_snr: Some(10),
            max_snr: Some(20),
            ..Default::default()
        };

        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)));
        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 10, 20)));
        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 20, 20)));
        assert!(!filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 5, 20)));
        assert!(!filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 25, 20)));
    }

    #[test]
    fn test_filter_combined_and_logic() {
        let filter = SpotFilter {
            bands: Some(vec!["20m".to_string()]),
            min_snr: Some(15),
            ..Default::default()
        };

        // Both conditions met
        assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 20, 20)));
        // Band ok, SNR too low
        assert!(!filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 10, 20)));
        // SNR ok, wrong band
        assert!(!filter.matches(&make_spot("W6JSV", "EA5WU-#", 7025.0, 20, 20)));
    }

    #[test]
    fn test_any_filter_matches_or_logic() {
        let filters = vec![
            SpotFilter {
                dx_call: Some("W6JSV".to_string()),
                ..Default::default()
            },
            SpotFilter {
                bands: Some(vec!["40m".to_string()]),
                ..Default::default()
            },
        ];

        // Matches first filter
        assert!(any_filter_matches(
            &filters,
            &make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)
        ));
        // Matches second filter
        assert!(any_filter_matches(
            &filters,
            &make_spot("K1ABC", "EA5WU-#", 7025.0, 15, 20)
        ));
        // Matches neither
        assert!(!any_filter_matches(
            &filters,
            &make_spot("K1ABC", "EA5WU-#", 14025.0, 15, 20)
        ));
    }

    #[test]
    fn test_empty_filters() {
        let filters: Vec<SpotFilter> = vec![];
        assert!(!any_filter_matches(
            &filters,
            &make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)
        ));
    }
}
