# Config File Design

## Overview

Add a TOML config file at `~/.config/rbn-parser/config.toml` for persistent connection and preference settings.

## Settings Split

**Config file (persistent preferences):**
- callsign, host, port, connect_timeout, read_timeout, reconnect
- cw_only, stats_interval

**CLI flags (runtime behavior):**
- verbose, log_level, max_runtime

## Config Structure

```rust
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub callsign: String,      // default: "N0CALL"
    pub host: String,          // default: "telnet.reversebeacon.net"
    pub port: u16,             // default: 7000
    pub connect_timeout: u64,  // default: 30
    pub read_timeout: u64,     // default: 120
    pub reconnect: bool,       // default: true
    pub cw_only: bool,         // default: true
    pub stats_interval: u64,   // default: 30
}
```

## Loading Behavior

- Config file is optional - missing file uses defaults
- Invalid TOML exits with clear error including file path
- Missing fields use defaults via `#[serde(default)]`
- Location: `~/.config/rbn-parser/config.toml` (via `dirs` crate)

## Example Config

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

## Dependencies

- `toml` - TOML parsing
- `dirs` - XDG config directory lookup

## Changes Required

1. Add `toml` and `dirs` to Cargo.toml
2. Create `src/config.rs` module
3. Update `src/lib.rs` to export config module
4. Simplify `src/main.rs` - remove config-related clap args, load from Config
