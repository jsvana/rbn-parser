//! Data structures representing RBN spots.
//!
//! This module defines the core types used throughout the application
//! to represent parsed CW spots from the Reverse Beacon Network.

use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The type of CQ or beacon activity detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpotType {
    /// Standard CQ call
    Cq,
    /// NCDXF/IARU beacon
    NcdxfBeacon,
    /// Generic beacon
    Beacon,
    /// Unknown or other type
    Other,
}

impl fmt::Display for SpotType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpotType::Cq => write!(f, "CQ"),
            SpotType::NcdxfBeacon => write!(f, "NCDXF B"),
            SpotType::Beacon => write!(f, "BEACON"),
            SpotType::Other => write!(f, "OTHER"),
        }
    }
}

/// The transmission mode of the spot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Mode {
    /// Continuous Wave (Morse code)
    Cw,
    /// Radio Teletype
    Rtty,
    /// FT8 digital mode
    Ft8,
    /// FT4 digital mode
    Ft4,
    /// PSK31 digital mode
    Psk31,
    /// Unknown mode
    Unknown,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Cw => write!(f, "CW"),
            Mode::Rtty => write!(f, "RTTY"),
            Mode::Ft8 => write!(f, "FT8"),
            Mode::Ft4 => write!(f, "FT4"),
            Mode::Psk31 => write!(f, "PSK31"),
            Mode::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

/// A parsed CW spot from the Reverse Beacon Network.
///
/// This represents a single decoded signal detected by a skimmer station.
///
/// # Example
///
/// A raw spot like:
/// ```text
/// DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z
/// ```
///
/// Would be parsed into a `CwSpot` with:
/// - `spotter`: "EA5WU-#"
/// - `frequency_khz`: 7018.3
/// - `dx_call`: "RW1M"
/// - `mode`: Mode::Cw
/// - `snr_db`: 19
/// - `wpm`: 18
/// - `spot_type`: SpotType::Cq
/// - `time`: 22:59 UTC
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CwSpot {
    /// The callsign of the skimmer station that detected this signal.
    /// Typically includes a `-#` suffix indicating it's an RBN skimmer.
    pub spotter: String,

    /// The frequency in kHz where the signal was detected.
    pub frequency_khz: f64,

    /// The callsign of the station being spotted (the DX station).
    pub dx_call: String,

    /// The transmission mode (CW, RTTY, etc.).
    pub mode: Mode,

    /// Signal-to-noise ratio in decibels.
    pub snr_db: i32,

    /// CW speed in words per minute.
    pub wpm: u16,

    /// The type of activity (CQ, BEACON, etc.).
    pub spot_type: SpotType,

    /// The UTC time when the spot was reported (time only, no date).
    pub time: NaiveTime,
}

impl CwSpot {
    /// Returns the amateur radio band for this spot's frequency.
    ///
    /// Returns `None` if the frequency doesn't fall within a recognized band.
    pub fn band(&self) -> Option<&'static str> {
        match self.frequency_khz as u32 {
            135..=138 => Some("2200m"),
            472..=479 => Some("630m"),
            1800..=2000 => Some("160m"),
            3500..=4000 => Some("80m"),
            5330..=5410 => Some("60m"),
            7000..=7300 => Some("40m"),
            10100..=10150 => Some("30m"),
            14000..=14350 => Some("20m"),
            18068..=18168 => Some("17m"),
            21000..=21450 => Some("15m"),
            24890..=24990 => Some("12m"),
            28000..=29700 => Some("10m"),
            50000..=54000 => Some("6m"),
            144000..=148000 => Some("2m"),
            _ => None,
        }
    }

    /// Returns the size of this spot in bytes when serialized as JSON.
    pub fn json_size(&self) -> usize {
        // This is approximate but consistent for statistics
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }
}

impl fmt::Display for CwSpot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DX de {}: {:>8.1} {} {} {} dB {} WPM {} {}",
            self.spotter,
            self.frequency_khz,
            self.dx_call,
            self.mode,
            self.snr_db,
            self.wpm,
            self.spot_type,
            self.time.format("%H%MZ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_band_detection() {
        let spot = CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: 14025.0,
            dx_call: "W1AW".to_string(),
            mode: Mode::Cw,
            snr_db: 10,
            wpm: 20,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        assert_eq!(spot.band(), Some("20m"));
    }

    #[test]
    fn test_band_detection_edge_cases() {
        let make_spot = |freq: f64| CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: freq,
            dx_call: "W1AW".to_string(),
            mode: Mode::Cw,
            snr_db: 10,
            wpm: 20,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        assert_eq!(make_spot(7000.0).band(), Some("40m"));
        assert_eq!(make_spot(7300.0).band(), Some("40m"));
        assert_eq!(make_spot(6999.0).band(), None);
    }
}
