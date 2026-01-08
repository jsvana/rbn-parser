# Multi-Callsign Filters and PoLo Notes Integration

## Summary

Enhance filter capabilities with two features:
1. Generalize `dx_call` and `spotter` fields to accept arrays (OR logic)
2. Load callsigns from Ham2K PoLo notes files via URL

## Generalized Pattern Fields

The `dx_call` and `spotter` fields accept either a single string or an array:

```toml
[[filters]]
name = "california_calls"
dx_call = ["W6*", "K6*", "N6*"]  # Array - matches ANY pattern
spotter = "EA5*"                 # Single string still works
min_snr = 10

[[filters]]
name = "specific_call"
dx_call = "W6JSV"  # Single string still works
```

Implementation uses a custom deserializer that accepts `String` or `Vec<String>`, normalizing to `Vec<String>` internally. Matching iterates the array with OR semantics.

## PoLo Callsign Notes Integration

New filter fields for loading callsigns from a URL:

```toml
[[filters]]
name = "polo_watchlist"
polo_notes_url = "https://example.com/my-notes.txt"
polo_refresh_secs = 1800  # 30 minutes, default
min_snr = 15
bands = ["20m", "40m"]
```

### PoLo File Format

Ham2K PoLo uses a simple text format:
```
# Comment lines start with #
VK1AO Alan
VK4KC Marty
KI2D Sebastián
```

Format: `CALLSIGN space notes-text`. We extract only the callsign (first token).

### Behavior

- On startup, fetch the URL and parse callsigns
- Re-fetch every `polo_refresh_secs` (default 1800 = 30 min, 0 = no refresh)
- Callsigns matched against `spot.dx_call` with OR logic
- Can combine with other filter fields (bands, min_snr) using AND logic
- If fetch fails, log warning and keep previous callsigns (empty on first failure)

### Mutual Exclusion

A filter cannot specify both `dx_call` and `polo_notes_url` - config validation rejects this.

## Implementation Architecture

### New Types

```rust
/// Deserializes from string or array, stored as Vec<String>
pub struct PatternList(Vec<String>);

/// Runtime state for PoLo notes (shared across filter instances)
pub struct PoloNotesCache {
    url: String,
    refresh_secs: u64,
    callsigns: RwLock<Vec<String>>,
    last_fetch: AtomicU64,  // Unix timestamp
}
```

### SpotFilter Changes

- `dx_call: Option<PatternList>` (was `Option<String>`)
- `spotter: Option<PatternList>` (was `Option<String>`)
- `polo_notes_url: Option<String>` (new)
- `polo_refresh_secs: Option<u64>` (new, default 1800)

### Background Refresh

- Tokio task spawned at startup for each unique `polo_notes_url`
- Uses `reqwest` for HTTP fetching
- Shared `Arc<PoloNotesCache>` passed to filters referencing same URL

### Matching Logic

1. Check `dx_call` patterns (OR within array, existing wildcard matching)
2. Check `polo_callsigns` if configured (OR against cached list, exact match)
3. Other fields (bands, min_snr, etc.) use AND logic as before

## Error Handling

### Startup

- Fetch failure: log error, filter starts with empty callsign list
- Invalid URL: validation error at config load time

### Runtime Refresh

- Fetch failure: log warning, retain previous callsigns, retry on next interval

### Edge Cases

- Empty file = empty callsign list (matches nothing)
- Lines without valid callsigns = skipped
- `polo_refresh_secs = 0` = one-time load, no refresh

## Testing Strategy

### Unit Tests

- `PatternList` deserialization: string, array, empty
- `matches()` with multiple patterns
- PoLo parsing: basic format, comments, empty lines, malformed

### Integration Tests

- Mock HTTP server for URL fetching
- Refresh behavior verification
- Graceful degradation on failures

### Config Validation Tests

- `dx_call` + `polo_notes_url` = error
- Invalid URL = error
- Valid combinations pass
