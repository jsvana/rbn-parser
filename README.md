# RBN Parser

A Rust server for parsing CW spots from the [Reverse Beacon Network](https://www.reversebeacon.net/) (RBN).

## Features

- **Robust nom-based parser** - Correctness-first parsing with comprehensive error handling
- **Statistics tracking** - HDR histograms for size, SNR, and WPM distributions
- **Async telnet client** - Non-blocking connection with auto-reconnect
- **CW-focused filtering** - Built for CW operators, filters out RTTY/digital modes
- **Band detection** - Automatic amateur band identification from frequency
- **Configurable spot filters** - Match spots by callsign patterns, bands, SNR, WPM
- **Prometheus metrics** - Export statistics for monitoring and alerting
- **Spot storage** - Bounded per-filter queues with configurable limits
- **REST API** - Cursor-based retrieval of stored spots

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

## Usage

### Configuration

Copy the example config to your config directory:

```bash
mkdir -p ~/.config/rbn-parser
cp config.example.toml ~/.config/rbn-parser/config.toml
```

Then edit `~/.config/rbn-parser/config.toml` to set your callsign:

```toml
callsign = "W6JSV"

# Enable HTTP server (metrics, health, spot API)
server_enabled = true
server_port = 9090

# Spot filters - print spots matching any filter
[[filters]]
name = "my_calls"
dx_call = "W6*"        # Wildcard prefix match

[[filters]]
name = "high_snr_20m"
bands = ["20m"]
min_snr = 20

# Spot storage - keep recent matched spots in memory
[storage]
default_max_kept_entries = 50
global_max_size = "10MB"
```

All fields are optional - defaults are used for any missing fields.

### Basic Usage

Connect to RBN and start collecting statistics:

```bash
rbn-parser
```

### Command Line Options

```
Usage: rbn-parser [OPTIONS]

Options:
  -v, --verbose              Print each parsed spot
      --log-level <LEVEL>    Log level (trace, debug, info, warn, error) [default: info]
      --max-runtime <SECS>   Maximum runtime in seconds (0 = unlimited) [default: 0]
  -h, --help                 Print help
  -V, --version              Print version
```

### Examples

Verbose mode showing each spot:

```bash
rbn-parser -v
```

Run for 5 minutes and exit:

```bash
rbn-parser --max-runtime 300
```

Debug logging:

```bash
rbn-parser --log-level debug
```

## Prometheus Metrics

When `server_enabled = true`, an HTTP server exposes metrics at `http://localhost:9090/metrics`:

```bash
curl http://localhost:9090/metrics
```

Available metrics:
- `rbn_uptime_seconds` - Time since parser started
- `rbn_spots_total{mode="CW"}` - Total spots by mode
- `rbn_spots_per_second` - Current processing rate
- `rbn_spots_by_band_total{band="20m"}` - Spots by band
- `rbn_snr_db{quantile="0.5"}` - SNR distribution
- `rbn_wpm{quantile="0.5"}` - WPM distribution
- `rbn_filter_stored_spots{filter="..."}` - Stored spots per filter
- `rbn_filter_overflow_total{filter="..."}` - Evicted spots per filter
- `rbn_storage_total_bytes` - Total storage usage

## REST API

When storage is configured, REST endpoints are available for retrieving stored spots:

### List Filters

```bash
curl http://localhost:9090/spots/filters
# ["my_calls", "high_snr_20m"]
```

### Get Spots

```bash
# Get all stored spots for a filter
curl http://localhost:9090/spots/filter/my_calls

# Response:
{
  "filter": "my_calls",
  "spots": [
    {"seq": 1, "spot": {"spotter": "EA5WU-#", "frequency_khz": 14025.0, ...}},
    {"seq": 2, "spot": {"spotter": "K3LR-#", "frequency_khz": 7018.3, ...}}
  ],
  "latest_seq": 2,
  "overflow_count": 0
}
```

### Cursor-Based Polling

Use the `since` parameter for efficient polling:

```bash
# First request - get all spots
curl http://localhost:9090/spots/filter/my_calls
# Returns latest_seq: 50

# Subsequent requests - only get new spots
curl "http://localhost:9090/spots/filter/my_calls?since=50"
# Returns spots with seq > 50
```

## Spot Format

The parser handles RBN spot messages in this format:

```
DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z
      │           │       │              │     │      │       │       │
      │           │       │              │     │      │       │       └─ UTC time
      │           │       │              │     │      │       └─ Spot type (CQ/BEACON)
      │           │       │              │     │      └─ CW speed in WPM
      │           │       │              │     └─ Signal-to-noise ratio
      │           │       │              └─ Mode
      │           │       └─ DX station callsign
      │           └─ Frequency in kHz
      └─ Skimmer station callsign
```

## Statistics Output

The server tracks and reports:

- Total spots processed
- CW vs. other mode breakdown
- Parse success/failure rates
- Size distribution (P50, P90, P99)
- SNR distribution
- WPM distribution
- Spots by band
- Spots by type (CQ, BEACON, etc.)
- Top 10 spotters (skimmers)

Example output:

```
═══════════════════════════════════════════════════════
                  RBN SPOT STATISTICS
═══════════════════════════════════════════════════════

Runtime: 300.0s
Total spots: 15234
CW spots: 14892 (97.8%)
Parse failures: 12
Non-spot lines: 45
Bytes processed: 1523 KB
Rate: 50.8 spots/sec

Size Distribution (bytes):
  Min: 89, Max: 156, Mean: 112.3
  P50: 110, P90: 125, P99: 142

SNR Distribution (dB):
  Min: -8, Max: 55, Mean: 18.4
  P50: 17, P90: 32, P99: 45

WPM Distribution:
  Min: 5, Max: 45, Mean: 22.1
  P50: 22, P90: 30, P99: 38

Spots by Band:
  20m: 4521
  40m: 3892
  15m: 2341
  ...

Top 10 Spotters:
  1. KM3T-2-#: 892
  2. W3OA-#: 756
  ...
```

## Library Usage

The parser can also be used as a library:

```rust
use rbn_parser::{parse_spot, SpotStats, CwSpot};

// Parse a single spot
let line = "DX de EA5WU-#:    7018.3  RW1M           CW    19 dB  18 WPM  CQ      2259Z";
let spot: CwSpot = parse_spot(line)?;

println!("Spotter: {}", spot.spotter);
println!("DX: {}", spot.dx_call);
println!("Frequency: {} kHz", spot.frequency_khz);
println!("Band: {:?}", spot.band());
println!("SNR: {} dB", spot.snr_db);
println!("Speed: {} WPM", spot.wpm);

// Collect statistics
let stats = SpotStats::new();
stats.record_spot(&spot);
println!("{}", stats.summary());
```

## Architecture

```
src/
├── lib.rs        # Library entry point
├── main.rs       # CLI application
├── config.rs     # TOML configuration
├── spot.rs       # CwSpot data structure
├── parser.rs     # nom-based parser
├── filter.rs     # Spot filtering
├── stats.rs      # Statistics collection
├── storage.rs    # Spot storage queues
├── metrics.rs    # Prometheus metrics & REST API
└── client.rs     # Async telnet client
```

## Testing

Run the test suite:

```bash
cargo test
```

Run benchmarks:

```bash
cargo bench
```

## Future Plans

- [ ] WebSocket streaming for real-time spot updates
- [ ] Persistent storage (SQLite/PostgreSQL)
- [ ] Real-time dashboard
- [ ] Geographic/region-based filtering

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.

## See Also

- [Reverse Beacon Network](https://www.reversebeacon.net/)
- [CW Skimmer](https://www.dxatlas.com/cwskimmer/)
- [AR-Cluster](http://www.ab5k.net/)
