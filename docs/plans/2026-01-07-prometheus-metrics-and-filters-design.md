# Prometheus Metrics and Spot Filters Design

## Overview

Add two features to rbn-parser:
1. Prometheus-style metrics HTTP endpoint for monitoring
2. Configurable spot filters to print matching spots

## Configuration

Extend `~/.config/rbn-parser/config.toml`:

```toml
# Existing settings remain unchanged
callsign = "W6JSV"
host = "telnet.reversebeacon.net"
port = 7000
connect_timeout = 30
read_timeout = 120
reconnect = true
cw_only = true
stats_interval = 30

# NEW: Prometheus metrics
metrics_enabled = true
metrics_port = 9090

# NEW: Spot filters (spots matching ANY filter are printed)
[[filters]]
dx_call = "W6*"           # Wildcard: prefix match

[[filters]]
bands = ["20m", "40m"]    # Must be on one of these bands
min_snr = 15              # AND have SNR >= 15

[[filters]]
spotter = "*-#"           # All RBN skimmers
spot_types = ["CQ"]       # Only CQ calls
```

### Filter Fields

All fields are optional within a filter:

| Field | Type | Description |
|-------|------|-------------|
| `dx_call` | String | DX callsign with optional `*` wildcard (prefix or suffix) |
| `spotter` | String | Spotter callsign with optional `*` wildcard |
| `bands` | Array | Band names: `["160m", "80m", "40m", "30m", "20m", "17m", "15m", "12m", "10m", "6m"]` |
| `modes` | Array | Modes: `["CW", "RTTY", "FT8", "FT4", "PSK31"]` |
| `spot_types` | Array | Types: `["CQ", "BEACON", "NCDXF_BEACON"]` |
| `min_snr` | Integer | Minimum SNR in dB |
| `max_snr` | Integer | Maximum SNR in dB |
| `min_wpm` | Integer | Minimum WPM |
| `max_wpm` | Integer | Maximum WPM |

### Matching Logic

- All specified fields within a filter must match (AND)
- A spot prints if it matches any filter (OR)
- Empty `[[filters]]` array = no filtered output
- `--verbose` flag remains independent (prints ALL spots)

## Prometheus Metrics

HTTP server on configured port exposes `/metrics`:

```prometheus
# Counters
rbn_spots_total{mode="CW"} 12345
rbn_spots_total{mode="RTTY"} 234
rbn_parse_failures_total 12
rbn_non_spot_lines_total 89
rbn_bytes_processed_total 1048576

# Gauges
rbn_spots_per_second 4.2
rbn_uptime_seconds 3600

# Histograms
rbn_snr_db_bucket{le="0"} 100
rbn_snr_db_bucket{le="10"} 500
rbn_snr_db_bucket{le="20"} 1200
rbn_snr_db_bucket{le="+Inf"} 1500
rbn_snr_db_count 1500
rbn_snr_db_sum 18000

rbn_wpm_bucket{le="15"} 200
rbn_wpm_bucket{le="25"} 800
rbn_wpm_bucket{le="35"} 1400
rbn_wpm_bucket{le="+Inf"} 1500
rbn_wpm_count 1500
rbn_wpm_sum 33000

# Labeled breakdowns
rbn_spots_by_band{band="20m"} 5000
rbn_spots_by_band{band="40m"} 3000
rbn_spots_by_type{type="CQ"} 7500
rbn_spots_by_type{type="BEACON"} 500
```

## Implementation

### File Structure

```
src/
├── config.rs      # Extended: metrics_* fields, Filter struct
├── filter.rs      # NEW: SpotFilter struct, matching logic, wildcard support
├── metrics.rs     # NEW: Prometheus HTTP server, format_metrics()
├── main.rs        # Modified: start metrics server, apply filters
├── lib.rs         # Extended: re-export new types
└── ...existing...
```

### New Structs

```rust
// config.rs
pub struct Config {
    // ...existing fields...
    pub metrics_enabled: bool,
    pub metrics_port: u16,
    pub filters: Vec<SpotFilter>,
}

// filter.rs
pub struct SpotFilter {
    pub dx_call: Option<String>,
    pub spotter: Option<String>,
    pub bands: Option<Vec<String>>,
    pub modes: Option<Vec<Mode>>,
    pub spot_types: Option<Vec<SpotType>>,
    pub min_snr: Option<i32>,
    pub max_snr: Option<i32>,
    pub min_wpm: Option<u16>,
    pub max_wpm: Option<u16>,
}

impl SpotFilter {
    pub fn matches(&self, spot: &CwSpot) -> bool { ... }
}
```

### Data Flow

```
RbnClient → process_line() → SpotStats.record_spot()
                ↓
         filter.matches(&spot)?
                ↓
         println!("{}", spot)   ← filtered output

SpotStats ←──── metrics server (reads on /metrics request)
```

### Dependencies

- `axum` or `tiny_http` for minimal HTTP server

### Defaults

| Setting | Default | Rationale |
|---------|---------|-----------|
| `metrics_enabled` | `false` | Opt-in, doesn't break existing setups |
| `metrics_port` | `9090` | Standard Prometheus exporter port |
| `filters` | `[]` | Empty = no filtered output |

### Error Handling

- Metrics port in use: log error, continue without metrics server
- Malformed filter config: fail fast at startup with clear error
- Invalid wildcard (e.g., `"*W6*"`): reject at config load

## Interaction with Existing Features

- `--verbose`: prints ALL spots (unchanged)
- Filters: print MATCHING spots (independent of --verbose)
- Both can be active simultaneously
