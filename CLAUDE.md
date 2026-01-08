# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Debug build
cargo build --release    # Release build (LTO enabled)
cargo test               # Run all tests
cargo test parser        # Run tests matching "parser"
cargo bench              # Run benchmarks (requires criterion)
cargo run                # Run (uses config file for settings)
cargo run -- --verbose   # Run with verbose output
```

## Architecture

This is a Rust library and CLI for parsing CW spots from the Reverse Beacon Network (RBN). Uses Rust 2024 edition.

### Module Structure

- **lib.rs** - Library entry point, re-exports public API (`parse_spot`, `CwSpot`, `SpotStats`, `RbnClient`, `Config`)
- **config.rs** - TOML config file support. Loads from `~/.config/rbn-parser/config.toml` (via `dirs` crate)
- **parser.rs** - nom-based parser for RBN spot messages. Key functions: `parse_spot()`, `looks_like_spot()` (fast pre-filter), `is_cw_spot()`
- **spot.rs** - Data structures: `CwSpot` (parsed spot), `Mode` enum (CW/RTTY/FT8/etc), `SpotType` enum (CQ/Beacon/etc). Contains `band()` method for frequency-to-band mapping
- **stats.rs** - Thread-safe statistics collector using atomics and RwLock. Uses hdrhistogram for percentile tracking (size, SNR, WPM)
- **client.rs** - Async telnet client using tokio. Handles connection, login, and auto-reconnect. Sends `RbnEvent`s via mpsc channel
- **main.rs** - CLI application using clap. Connection settings from config file, runtime flags from CLI

### Configuration

Config file: `~/.config/rbn-parser/config.toml` (optional, uses defaults if missing)

```toml
callsign = "W6JSV"
host = "telnet.reversebeacon.net"
port = 7000
connect_timeout = 30
read_timeout = 120
reconnect = true
cw_only = true
stats_interval = 30
```

CLI flags (runtime only): `--verbose`, `--log-level`, `--max-runtime`

### Data Flow

1. `RbnClient` connects to RBN telnet server, sends callsign login, streams lines via `RbnEvent::Line`
2. `looks_like_spot()` does fast pre-filtering (checks "DX de " prefix)
3. `parse_spot()` parses valid spots into `CwSpot` structs
4. `SpotStats` collects metrics with thread-safe counters and histograms

### Key Design Decisions

- Parser prioritizes correctness over performance (nom combinators for robust parsing)
- Statistics use atomics for counters, RwLock for histograms/maps
- SNR histogram stores values offset by +30 to handle negative dB values
- Client runs in background tokio task, communicates via channel
