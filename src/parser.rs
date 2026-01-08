//! Parser for RBN (Reverse Beacon Network) spot messages.
//!
//! This module uses the `nom` parsing library to parse DX cluster spot messages
//! from the RBN telnet feed. The parser is designed with correctness as the
//! primary goal, followed by performance.
//!
//! # Spot Format
//!
//! RBN spots follow this general format:
//! ```text
//! DX de SPOTTER:  FREQ  CALLSIGN  MODE  SNR dB  WPM WPM  TYPE  TIMEZ
//! ```
//!
//! Example:
//! ```text
//! DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z
//! ```

use chrono::NaiveTime;
use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{char, digit1, multispace1, space0, space1},
    combinator::{map_res, opt, recognize, value},
    sequence::terminated,
};
use thiserror::Error;

use crate::spot::{CwSpot, Mode, SpotType};

/// Errors that can occur during parsing.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Invalid spot format: {0}")]
    InvalidFormat(String),

    #[error("Invalid frequency: {0}")]
    InvalidFrequency(String),

    #[error("Invalid time: {0}")]
    InvalidTime(String),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Incomplete input")]
    Incomplete,
}

/// Result type for parsing operations.
pub type ParseResult<T> = Result<T, ParseError>;

/// Check if a character is valid in a callsign.
///
/// Valid callsign characters are alphanumeric plus `/` for portable designators
/// and `-` for suffixes like `-#` on RBN spotters.
fn is_callsign_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '#'
}

/// Parse the "DX de " prefix that starts every spot line.
fn parse_dx_de_prefix(input: &str) -> IResult<&str, ()> {
    value(
        (),
        (
            tag_no_case("DX"),
            multispace1,
            tag_no_case("de"),
            multispace1,
        ),
    )
    .parse(input)
}

/// Parse a callsign (spotter or DX station).
fn parse_callsign(input: &str) -> IResult<&str, &str> {
    take_while1(is_callsign_char).parse(input)
}

/// Parse the spotter callsign followed by a colon.
fn parse_spotter(input: &str) -> IResult<&str, &str> {
    terminated(parse_callsign, (char(':'), space0)).parse(input)
}

/// Parse a floating-point frequency in kHz.
fn parse_frequency(input: &str) -> IResult<&str, f64> {
    map_res(recognize((digit1, opt((char('.'), digit1)))), |s: &str| {
        s.parse::<f64>()
    })
    .parse(input)
}

/// Parse the transmission mode.
fn parse_mode(input: &str) -> IResult<&str, Mode> {
    alt((
        value(Mode::Cw, tag_no_case("CW")),
        value(Mode::Rtty, tag_no_case("RTTY")),
        value(Mode::Ft8, tag_no_case("FT8")),
        value(Mode::Ft4, tag_no_case("FT4")),
        value(Mode::Psk31, tag_no_case("PSK31")),
    ))
    .parse(input)
}

/// Parse the signal-to-noise ratio (e.g., "19 dB" or "-5 dB").
fn parse_snr(input: &str) -> IResult<&str, i32> {
    terminated(
        map_res(recognize((opt(char('-')), digit1)), |s: &str| {
            s.parse::<i32>()
        }),
        (space1, tag_no_case("dB")),
    )
    .parse(input)
}

/// Parse the CW speed in WPM (e.g., "18 WPM").
fn parse_wpm(input: &str) -> IResult<&str, u16> {
    terminated(
        map_res(digit1, |s: &str| s.parse::<u16>()),
        (space1, tag_no_case("WPM")),
    )
    .parse(input)
}

/// Parse the spot type (CQ, BEACON, NCDXF B, etc.).
fn parse_spot_type(input: &str) -> IResult<&str, SpotType> {
    alt((
        value(
            SpotType::NcdxfBeacon,
            (tag_no_case("NCDXF"), space1, tag_no_case("B")),
        ),
        value(SpotType::Beacon, tag_no_case("BEACON")),
        value(SpotType::Cq, tag_no_case("CQ")),
        // Catch-all for other types we might not recognize
        value(
            SpotType::Other,
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == ' '),
        ),
    ))
    .parse(input)
}

/// Parse the full UTC time from a 4-digit string like "2259Z".
fn parse_time_full(input: &str) -> IResult<&str, NaiveTime> {
    map_res(
        terminated(take_while1(|c: char| c.is_ascii_digit()), tag_no_case("Z")),
        |s: &str| {
            if s.len() != 4 {
                return Err("Time must be 4 digits");
            }
            let hour: u32 = s[0..2].parse().map_err(|_| "Invalid hour")?;
            let min: u32 = s[2..4].parse().map_err(|_| "Invalid minute")?;
            NaiveTime::from_hms_opt(hour, min, 0).ok_or("Invalid time values")
        },
    )
    .parse(input)
}

/// Parse a complete RBN CW spot line.
///
/// # Example
///
/// ```
/// use rbn_parser::parser::parse_spot;
///
/// let line = "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z";
/// let spot = parse_spot(line).unwrap();
/// assert_eq!(spot.spotter, "EA5WU-#");
/// assert_eq!(spot.dx_call, "RW1M");
/// ```
pub fn parse_spot(input: &str) -> ParseResult<CwSpot> {
    let input = input.trim();

    // Use a parser that handles variable whitespace between fields
    let result: IResult<&str, CwSpot> = (|input| {
        let (input, _) = parse_dx_de_prefix(input)?;
        let (input, spotter) = parse_spotter(input)?;
        let (input, _) = space0(input)?;
        let (input, frequency_khz) = parse_frequency(input)?;
        let (input, _) = space1(input)?;
        let (input, dx_call) = parse_callsign(input)?;
        let (input, _) = space1(input)?;
        let (input, mode) = parse_mode(input)?;
        let (input, _) = space1(input)?;
        let (input, snr_db) = parse_snr(input)?;
        let (input, _) = space1(input)?;
        let (input, wpm) = parse_wpm(input)?;
        let (input, _) = space1(input)?;
        let (input, spot_type) = parse_spot_type(input)?;
        let (input, _) = space0(input)?;
        let (input, time) = parse_time_full(input)?;

        Ok((
            input,
            CwSpot {
                spotter: spotter.to_string(),
                frequency_khz,
                dx_call: dx_call.to_string(),
                mode,
                snr_db,
                wpm,
                spot_type,
                time,
            },
        ))
    })(input);

    match result {
        Ok((_, spot)) => Ok(spot),
        Err(e) => Err(ParseError::InvalidFormat(format!("{:?}", e))),
    }
}

/// Check if a line looks like a spot (quick pre-filter).
///
/// This is a fast check to avoid running the full parser on non-spot lines.
#[inline]
pub fn looks_like_spot(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() > 20
        && (trimmed.starts_with("DX de ")
            || trimmed.starts_with("DX DE ")
            || trimmed.starts_with("dx de "))
}

/// Check if a spot is a CW spot (not RTTY or digital).
#[inline]
pub fn is_cw_spot(spot: &CwSpot) -> bool {
    matches!(spot.mode, Mode::Cw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_cq_spot() {
        let line = "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z";
        let spot = parse_spot(line).expect("Should parse successfully");

        assert_eq!(spot.spotter, "EA5WU-#");
        assert!((spot.frequency_khz - 7018.3).abs() < 0.01);
        assert_eq!(spot.dx_call, "RW1M");
        assert_eq!(spot.mode, Mode::Cw);
        assert_eq!(spot.snr_db, 19);
        assert_eq!(spot.wpm, 18);
        assert_eq!(spot.spot_type, SpotType::Cq);
        assert_eq!(spot.time, NaiveTime::from_hms_opt(22, 59, 0).unwrap());
    }

    #[test]
    fn test_parse_beacon_spot() {
        let line = "DX de KM3T-2-#:  14100.0  CS3B           CW    24 dB  22 WPM  NCDXF B 2259Z";
        let spot = parse_spot(line).expect("Should parse successfully");

        assert_eq!(spot.spotter, "KM3T-2-#");
        assert!((spot.frequency_khz - 14100.0).abs() < 0.01);
        assert_eq!(spot.dx_call, "CS3B");
        assert_eq!(spot.spot_type, SpotType::NcdxfBeacon);
    }

    #[test]
    fn test_parse_regular_beacon_spot() {
        let line = "DX de K9LC-#:    28169.9  VA3XCD/B       CW     9 dB  10 WPM  BEACON  2259Z";
        let spot = parse_spot(line).expect("Should parse successfully");

        assert_eq!(spot.dx_call, "VA3XCD/B");
        assert_eq!(spot.spot_type, SpotType::Beacon);
    }

    #[test]
    fn test_parse_portable_callsign() {
        let line = "DX de W1NT-6-#:  28222.9  N1NSP/B        CW     5 dB  15 WPM  BEACON  2259Z";
        let spot = parse_spot(line).expect("Should parse successfully");

        assert_eq!(spot.spotter, "W1NT-6-#");
        assert_eq!(spot.dx_call, "N1NSP/B");
    }

    #[test]
    fn test_looks_like_spot() {
        assert!(looks_like_spot(
            "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z"
        ));
        assert!(looks_like_spot(
            "  DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z  "
        ));
        assert!(!looks_like_spot("Hello world"));
        assert!(!looks_like_spot(""));
        assert!(!looks_like_spot("DX de ")); // Too short
    }

    #[test]
    fn test_is_cw_spot() {
        let cw_spot = CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: 7018.3,
            dx_call: "W1AW".to_string(),
            mode: Mode::Cw,
            snr_db: 10,
            wpm: 20,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        };

        assert!(is_cw_spot(&cw_spot));

        let rtty_spot = CwSpot {
            mode: Mode::Rtty,
            ..cw_spot
        };

        assert!(!is_cw_spot(&rtty_spot));
    }

    #[test]
    fn test_case_insensitive_parsing() {
        let line = "dx de ea5wu-#:    7018.3  rw1m           cw    19 db  18 wpm  cq      2259z";
        let spot = parse_spot(line).expect("Should parse case-insensitively");
        assert_eq!(spot.mode, Mode::Cw);
    }

    #[test]
    fn test_negative_snr() {
        // Some weak signals have negative SNR
        let line = "DX de TEST-#:    7018.3  W1AW           CW    -5 dB  20 WPM  CQ      1234Z";
        let spot = parse_spot(line).expect("Should parse negative SNR");
        assert_eq!(spot.snr_db, -5);
    }

    #[test]
    fn test_midnight_time() {
        let line = "DX de TEST-#:    7018.3  W1AW           CW    10 dB  20 WPM  CQ      0000Z";
        let spot = parse_spot(line).expect("Should parse midnight time");
        assert_eq!(spot.time, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    }

    #[test]
    fn test_various_bands() {
        let test_cases = [
            (
                "DX de T-#: 1820.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                1820.0,
                Some("160m"),
            ),
            (
                "DX de T-#: 3525.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                3525.0,
                Some("80m"),
            ),
            (
                "DX de T-#: 7030.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                7030.0,
                Some("40m"),
            ),
            (
                "DX de T-#: 14025.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                14025.0,
                Some("20m"),
            ),
            (
                "DX de T-#: 21025.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                21025.0,
                Some("15m"),
            ),
            (
                "DX de T-#: 28025.0 W1 CW 10 dB 20 WPM CQ 0000Z",
                28025.0,
                Some("10m"),
            ),
        ];

        for (line, expected_freq, expected_band) in test_cases {
            let spot = parse_spot(line).unwrap_or_else(|_| panic!("Should parse: {}", line));
            assert!((spot.frequency_khz - expected_freq).abs() < 0.1);
            assert_eq!(spot.band(), expected_band);
        }
    }
}
