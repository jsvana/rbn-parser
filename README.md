# RBN Parser

A Rust server for parsing CW spots from the [Reverse Beacon Network](https://www.reversebeacon.net/) (RBN).

## Features

- **Robust nom-based parser** - Correctness-first parsing with comprehensive error handling
- **Statistics tracking** - HDR histograms for size, SNR, and WPM distributions
- **Async telnet client** - Non-blocking connection with auto-reconnect
- **CW-focused filtering** - Built for CW operators, filters out RTTY/digital modes
- **Band detection** - Automatic amateur band identification from frequency

## Installation

```bash
cargo install --path .
```

Or build from source:

```bash
cargo build --release
```

## Usage

### Basic Usage

Connect to RBN and start collecting statistics:

```bash
rbn-parser --callsign W6JSV
```

### Command Line Options

```
Usage: rbn-parser [OPTIONS]

Options:
  -c, --callsign <CALLSIGN>      Callsign to use for RBN login [env: RBN_CALLSIGN=] [default: N0CALL]
      --host <HOST>              RBN server hostname [env: RBN_HOST=] [default: telnet.reversebeacon.net]
      --port <PORT>              RBN server port [env: RBN_PORT=] [default: 7000]
      --cw-only                  Only count CW spots (ignore RTTY) [default: true]
  -s, --stats-interval <SECS>    Print statistics every N seconds [default: 30]
  -v, --verbose                  Print each parsed spot
      --log-level <LEVEL>        Log level (trace, debug, info, warn, error) [default: info]
      --no-reconnect             Disable auto-reconnect
      --connect-timeout <SECS>   Connection timeout in seconds [default: 30]
      --max-runtime <SECS>       Maximum runtime in seconds (0 = unlimited) [default: 0]
  -h, --help                     Print help
  -V, --version                  Print version
```

### Examples

Verbose mode showing each spot:

```bash
rbn-parser -c W6JSV -v
```

Run for 5 minutes and exit:

```bash
rbn-parser -c W6JSV --max-runtime 300
```

Custom stats interval:

```bash
rbn-parser -c W6JSV --stats-interval 60
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
├── spot.rs       # CwSpot data structure
├── parser.rs     # nom-based parser
├── stats.rs      # Statistics collection
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

- [ ] HTTP API for recent spots
- [ ] WebSocket streaming
- [ ] Prometheus metrics export
- [ ] Spot storage (SQLite/PostgreSQL)
- [ ] Filtering by band/region
- [ ] Real-time dashboard

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.

## See Also

- [Reverse Beacon Network](https://www.reversebeacon.net/)
- [CW Skimmer](https://www.dxatlas.com/cwskimmer/)
- [AR-Cluster](http://www.ab5k.net/)
