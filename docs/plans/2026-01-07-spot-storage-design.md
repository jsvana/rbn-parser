# Spot Storage Design

Store recent spots matching configured filters in bounded per-filter queues with Prometheus metrics exposure and REST API access.

## Configuration

```toml
# Optional storage section - if omitted, no spot storage occurs
[storage]
default_max_kept_entries = 50      # default per-filter limit
global_max_size = "10MB"           # human-readable: "500KB", "10MB", etc.

[[filters]]
name = "w6_calls"                  # optional, for metrics labels
dx_call = "W6*"
max_kept_entries = 100             # overrides default

[[filters]]
name = "high_snr_20m"
bands = ["20m"]
min_snr = 20
# uses default_max_kept_entries
```

**Size parsing**: Support units KB, MB, GB (case-insensitive). "10MB" → 10,485,760 bytes.

**Behavior when `[storage]` absent**: Filters work for printing (current behavior), but no storage/metrics.

## Data Structures

```rust
// New: src/storage.rs

/// Configuration for spot storage
pub struct StorageConfig {
    pub default_max_kept_entries: usize,
    pub global_max_size: usize,  // parsed from human-readable string
}

/// Per-filter storage queue
pub struct FilterStorage {
    pub name: Option<String>,           // from filter config, for metrics labels
    pub max_kept_entries: usize,        // resolved (filter override or default)
    pub spots: VecDeque<CwSpot>,        // bounded queue
    pub overflow_count: AtomicU64,      // bumped on eviction
    pub current_size_bytes: AtomicUsize, // track memory usage
}

/// Central storage manager (thread-safe)
pub struct SpotStorage {
    pub config: StorageConfig,
    pub filters: Vec<(SpotFilter, RwLock<FilterStorage>)>,
    pub total_size_bytes: AtomicUsize,  // for global limit enforcement
}
```

**Why `VecDeque`**: Efficient push_back/pop_front for queue behavior. Standard library.

**Thread safety**: `RwLock` per `FilterStorage` allows concurrent reads (metrics) with occasional writes (new spots).

## Core Operations

**Storing a spot**:

```rust
impl SpotStorage {
    pub fn store_spot(&self, filter_index: usize, spot: CwSpot) {
        let spot_size = spot.json_size();

        // 1. Check global limit, evict if needed (proportional - from largest filter)
        while self.total_size_bytes.load(Relaxed) + spot_size > self.config.global_max_size {
            self.evict_from_largest_filter();
        }

        // 2. Get the filter's storage
        let (_, storage_lock) = &self.filters[filter_index];
        let mut storage = storage_lock.write();

        // 3. Check per-filter limit, evict oldest if needed
        while storage.spots.len() >= storage.max_kept_entries {
            if let Some(old) = storage.spots.pop_front() {
                storage.overflow_count.fetch_add(1, Relaxed);
                self.total_size_bytes.fetch_sub(old.json_size(), Relaxed);
            }
        }

        // 4. Add the new spot
        storage.spots.push_back(spot);
        self.total_size_bytes.fetch_add(spot_size, Relaxed);
    }

    fn evict_from_largest_filter(&self) {
        // Find filter with most entries, evict its oldest
        // Bump that filter's overflow_count
    }
}
```

**Eviction strategy**: Proportional - evict from filter with most entries to keep storage balanced.

Both per-filter and global evictions increment the filter's `overflow_count`.

## Prometheus Metrics

```prometheus
# Per-filter metrics (using filter name or index as label)
rbn_filter_stored_spots{filter="w6_calls"} 87
rbn_filter_stored_bytes{filter="w6_calls"} 24536
rbn_filter_overflow_total{filter="w6_calls"} 142
rbn_filter_max_kept_entries{filter="w6_calls"} 100

# Global metrics
rbn_storage_total_bytes 38736
rbn_storage_global_max_bytes 10485760
rbn_storage_global_evictions_total 12
```

**Filter labels**: Use `name` field if provided, otherwise `filter_0`, `filter_1`, etc.

**Metric types**:
- `stored_spots`, `stored_bytes`: Gauges (current value)
- `overflow_total`, `global_evictions_total`: Counters (monotonically increasing)
- `max_kept_entries`, `global_max_bytes`: Gauges (config values)

## Integration

**Files to modify**:
1. `config.rs`: Add `StorageConfig` parsing, `max_kept_entries` to `SpotFilter`, size parser
2. `filter.rs`: Add optional `name` and `max_kept_entries` fields
3. `metrics.rs`: Add storage metrics to existing endpoint
4. `main.rs`: Create `SpotStorage` if config exists, pass to `process_line`

**New file**: `src/storage.rs`

**Data flow**:

```
RBN Line → parse_spot() → CwSpot
                            ↓
              for each filter that matches:
                - print if verbose (existing)
                - store in SpotStorage (new)
                            ↓
              Prometheus scrape → read storage metrics
```

**No new dependencies**: Uses `VecDeque`, atomics, `RwLock` from std.

## REST API

### Endpoints

**List filters**: `GET /spots/filters`

Returns list of available filter names.

```json
["w6_calls", "high_snr_20m"]
```

**Get spots**: `GET /spots/filter/{filter_name}?since={seq}`

- `filter_name`: The filter's name (from config) or `filter_0`, `filter_1` if unnamed
- `since` (optional): Return spots with sequence > this value. Omit for all stored spots.

**Response** (200 OK):
```json
{
  "filter": "w6_calls",
  "spots": [
    {"seq": 124, "spot": {"spotter": "EA5WU-#", "frequency_khz": 14025.0, ...}},
    {"seq": 125, "spot": {"spotter": "K3LR-#", "frequency_khz": 7018.3, ...}}
  ],
  "latest_seq": 125,
  "overflow_count": 42
}
```

**Error responses**:
- `404 Not Found`: Filter name doesn't exist, or storage not configured
- `400 Bad Request`: Invalid `since` parameter

### Sequence Numbers

Each `FilterStorage` maintains its own sequence counter:

```rust
pub struct FilterStorage {
    // ... existing fields ...
    pub next_seq: AtomicU64,           // next sequence to assign (starts at 1)
    spots: VecDeque<StoredSpot>,       // stores seq + spot
}

pub struct StoredSpot {
    pub seq: u64,
    pub spot: CwSpot,
}
```

**Behavior**:
- New spot gets `seq = next_seq.fetch_add(1)`
- When spot evicted, its sequence is gone forever (gaps appear)
- `since=123` returns spots where `seq > 123`
- `latest_seq` in response = highest seq in storage (or 0 if empty)

**Client usage pattern**:
```
1. GET /spots/filter/w6_calls         → latest_seq: 50
2. GET /spots/filter/w6_calls?since=50 → latest_seq: 53 (3 new spots)
3. GET /spots/filter/w6_calls?since=53 → latest_seq: 53 (no new spots)
```

### Integration

REST API endpoints added to existing metrics server in `metrics.rs`:

```rust
.route("/spots/filters", get(list_filters_handler))
.route("/spots/filter/:name", get(get_spots_handler))
```

Endpoints only available when `[storage]` is configured and `metrics_enabled = true`.
