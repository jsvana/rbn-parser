# Multi-Callsign Filters and PoLo Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow dx_call/spotter fields to accept arrays of patterns, and load callsigns from Ham2K PoLo notes URLs.

**Architecture:** Custom serde deserializer for string-or-array pattern fields. Separate PoloNotesManager handles URL fetching and caching with background refresh via tokio tasks.

**Tech Stack:** Rust, serde (custom deserializer), reqwest (HTTP client), tokio (async runtime)

---

### Task 1: Add reqwest dependency

**Files:**
- Modify: `Cargo.toml:9-17`

**Step 1: Add reqwest to dependencies**

Add after the axum line in Cargo.toml:

```toml
# HTTP client for PoLo notes fetching
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compiles successfully (may download new crates)

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "Add reqwest dependency for PoLo URL fetching"
```

---

### Task 2: Create PatternList type with custom deserializer

**Files:**
- Modify: `src/filter.rs:1-15`
- Test: `src/filter.rs` (tests module)

**Step 1: Write the failing test for PatternList deserialization**

Add to the tests module in `src/filter.rs`:

```rust
#[test]
fn test_pattern_list_from_string() {
    let json = r#""W6*""#;
    let list: PatternList = serde_json::from_str(json).unwrap();
    assert_eq!(list.patterns(), &["W6*"]);
}

#[test]
fn test_pattern_list_from_array() {
    let json = r#"["W6*", "K6*", "N6*"]"#;
    let list: PatternList = serde_json::from_str(json).unwrap();
    assert_eq!(list.patterns(), &["W6*", "K6*", "N6*"]);
}

#[test]
fn test_pattern_list_empty_array() {
    let json = r#"[]"#;
    let list: PatternList = serde_json::from_str(json).unwrap();
    assert!(list.patterns().is_empty());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_pattern_list`
Expected: FAIL - PatternList type doesn't exist

**Step 3: Implement PatternList type**

Add after the imports in `src/filter.rs`:

```rust
use serde::de::{self, Deserializer, Visitor};
use std::fmt;

/// A list of patterns that deserializes from either a string or array.
///
/// Used for dx_call and spotter fields to allow both:
/// - `dx_call = "W6*"` (single pattern)
/// - `dx_call = ["W6*", "K6*"]` (multiple patterns with OR logic)
#[derive(Debug, Clone, Default)]
pub struct PatternList(Vec<String>);

impl PatternList {
    /// Get the patterns as a slice.
    pub fn patterns(&self) -> &[String] {
        &self.0
    }

    /// Check if any pattern matches the value.
    pub fn matches_any(&self, value: &str) -> bool {
        self.0.iter().any(|p| matches_wildcard(p, value))
    }

    /// Check if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for PatternList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatternListVisitor;

        impl<'de> Visitor<'de> for PatternListVisitor {
            type Value = PatternList;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string or array of strings")
            }

            fn visit_str<E>(self, value: &str) -> Result<PatternList, E>
            where
                E: de::Error,
            {
                Ok(PatternList(vec![value.to_string()]))
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<PatternList, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut patterns = Vec::new();
                while let Some(value) = seq.next_element::<String>()? {
                    patterns.push(value);
                }
                Ok(PatternList(patterns))
            }
        }

        deserializer.deserialize_any(PatternListVisitor)
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test test_pattern_list`
Expected: PASS

**Step 5: Commit**

```bash
git add src/filter.rs
git commit -m "Add PatternList type with string-or-array deserialization"
```

---

### Task 3: Update SpotFilter to use PatternList

**Files:**
- Modify: `src/filter.rs:16-50` (SpotFilter struct)
- Modify: `src/filter.rs:52-118` (matches method)
- Modify: `src/filter.rs:120-131` (validate method)

**Step 1: Write failing test for array pattern matching**

Add to tests module:

```rust
#[test]
fn test_filter_dx_call_array() {
    let toml = r#"
        dx_call = ["W6*", "K6*"]
    "#;
    let filter: SpotFilter = toml::from_str(toml).unwrap();

    assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)));
    assert!(filter.matches(&make_spot("K6ABC", "EA5WU-#", 14025.0, 15, 20)));
    assert!(!filter.matches(&make_spot("N1ABC", "EA5WU-#", 14025.0, 15, 20)));
}

#[test]
fn test_filter_spotter_array() {
    let toml = r#"
        spotter = ["EA5*", "VE7*"]
    "#;
    let filter: SpotFilter = toml::from_str(toml).unwrap();

    assert!(filter.matches(&make_spot("W6JSV", "EA5WU-#", 14025.0, 15, 20)));
    assert!(filter.matches(&make_spot("W6JSV", "VE7ABC-#", 14025.0, 15, 20)));
    assert!(!filter.matches(&make_spot("W6JSV", "K1ABC-#", 14025.0, 15, 20)));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_filter_dx_call_array test_filter_spotter_array`
Expected: FAIL - SpotFilter still uses Option<String>

**Step 3: Update SpotFilter struct**

Change the dx_call and spotter fields in SpotFilter:

```rust
/// DX callsign patterns (supports `*` wildcard for prefix/suffix).
/// Accepts a single string or array of strings (OR logic within array).
pub dx_call: Option<PatternList>,

/// Spotter callsign patterns (supports `*` wildcard for prefix/suffix).
/// Accepts a single string or array of strings (OR logic within array).
pub spotter: Option<PatternList>,
```

**Step 4: Update matches() method**

Replace the dx_call and spotter checks in `matches()`:

```rust
// Check dx_call patterns (OR logic within array)
if let Some(ref patterns) = self.dx_call {
    if !patterns.is_empty() && !patterns.matches_any(&spot.dx_call) {
        return false;
    }
}

// Check spotter patterns (OR logic within array)
if let Some(ref patterns) = self.spotter {
    if !patterns.is_empty() && !patterns.matches_any(&spot.spotter) {
        return false;
    }
}
```

**Step 5: Update validate() method**

Replace the validation logic:

```rust
pub fn validate(&self) -> Result<(), String> {
    if let Some(ref patterns) = self.dx_call {
        for pattern in patterns.patterns() {
            validate_wildcard_pattern(pattern)?;
        }
    }
    if let Some(ref patterns) = self.spotter {
        for pattern in patterns.patterns() {
            validate_wildcard_pattern(pattern)?;
        }
    }
    Ok(())
}
```

**Step 6: Run all tests**

Run: `cargo test`
Expected: All tests pass (existing tests should still work due to backward-compatible deserialization)

**Step 7: Commit**

```bash
git add src/filter.rs
git commit -m "Update SpotFilter to use PatternList for dx_call and spotter"
```

---

### Task 4: Add PoLo notes fields to SpotFilter

**Files:**
- Modify: `src/filter.rs` (SpotFilter struct)
- Modify: `src/filter.rs` (validate method)

**Step 1: Write failing test for polo fields validation**

Add to tests module:

```rust
#[test]
fn test_filter_polo_notes_url() {
    let toml = r#"
        polo_notes_url = "https://example.com/notes.txt"
        polo_refresh_secs = 600
        min_snr = 10
    "#;
    let filter: SpotFilter = toml::from_str(toml).unwrap();

    assert_eq!(filter.polo_notes_url, Some("https://example.com/notes.txt".to_string()));
    assert_eq!(filter.polo_refresh_secs, Some(600));
}

#[test]
fn test_filter_polo_and_dx_call_exclusive() {
    let filter = SpotFilter {
        dx_call: Some(PatternList(vec!["W6*".to_string()])),
        polo_notes_url: Some("https://example.com/notes.txt".to_string()),
        ..Default::default()
    };

    assert!(filter.validate().is_err());
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_filter_polo`
Expected: FAIL - polo_notes_url field doesn't exist

**Step 3: Add polo fields to SpotFilter**

Add after the spotter field:

```rust
/// URL to Ham2K PoLo notes file for loading callsigns.
/// Mutually exclusive with dx_call.
pub polo_notes_url: Option<String>,

/// Refresh interval for PoLo notes in seconds (default 1800 = 30 min, 0 = no refresh).
pub polo_refresh_secs: Option<u64>,
```

**Step 4: Update validate() for mutual exclusion**

Add to the beginning of validate():

```rust
// Check mutual exclusion of dx_call and polo_notes_url
if self.dx_call.is_some() && self.polo_notes_url.is_some() {
    return Err("Cannot specify both 'dx_call' and 'polo_notes_url' on the same filter".to_string());
}

// Validate polo_notes_url format
if let Some(ref url) = self.polo_notes_url {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("polo_notes_url must be an HTTP(S) URL: {}", url));
    }
}
```

**Step 5: Run tests**

Run: `cargo test test_filter_polo`
Expected: PASS

**Step 6: Commit**

```bash
git add src/filter.rs
git commit -m "Add polo_notes_url and polo_refresh_secs fields to SpotFilter"
```

---

### Task 5: Create PoLo notes parser

**Files:**
- Create: `src/polo.rs`
- Modify: `src/lib.rs` (add module)

**Step 1: Write failing tests for PoLo parsing**

Create `src/polo.rs` with tests first:

```rust
//! Ham2K PoLo callsign notes file support.
//!
//! Parses callsign notes files and manages URL fetching with background refresh.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_polo_notes_basic() {
        let content = "VK1AO Alan\nVK4KC Marty\nKI2D Sebastián";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["VK1AO", "VK4KC", "KI2D"]);
    }

    #[test]
    fn test_parse_polo_notes_with_comments() {
        let content = "# My watchlist\nW6JSV Jay\n# Another comment\nK6ABC Bob";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_empty_lines() {
        let content = "W6JSV Jay\n\n\nK6ABC Bob\n";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_whitespace() {
        let content = "  W6JSV Jay\n\t\nK6ABC Bob  ";
        let callsigns = parse_polo_notes(content);
        assert_eq!(callsigns, vec!["W6JSV", "K6ABC"]);
    }

    #[test]
    fn test_parse_polo_notes_empty() {
        let content = "";
        let callsigns = parse_polo_notes(content);
        assert!(callsigns.is_empty());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test test_parse_polo`
Expected: FAIL - module doesn't exist yet

**Step 3: Add module to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod polo;
```

**Step 4: Implement parse_polo_notes function**

Add to `src/polo.rs` before the tests module:

```rust
/// Parse Ham2K PoLo notes file content into a list of callsigns.
///
/// File format:
/// - One callsign per line, followed by optional notes
/// - Lines starting with # are comments
/// - Empty lines are ignored
pub fn parse_polo_notes(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            // Extract callsign (first whitespace-delimited token)
            trimmed.split_whitespace().next().map(|s| s.to_uppercase())
        })
        .collect()
}
```

**Step 5: Run tests**

Run: `cargo test test_parse_polo`
Expected: PASS

**Step 6: Commit**

```bash
git add src/polo.rs src/lib.rs
git commit -m "Add PoLo notes file parser"
```

---

### Task 6: Create PoloNotesManager for URL fetching and caching

**Files:**
- Modify: `src/polo.rs`

**Step 1: Add imports and PoloNotesManager struct**

Add to the top of `src/polo.rs`:

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Default refresh interval for PoLo notes (30 minutes).
pub const DEFAULT_POLO_REFRESH_SECS: u64 = 1800;

/// Cached callsigns from a PoLo notes URL.
#[derive(Debug)]
struct PoloNotesCache {
    /// Cached callsigns (uppercase).
    callsigns: RwLock<Vec<String>>,
    /// Unix timestamp of last successful fetch.
    last_fetch: AtomicU64,
    /// Refresh interval in seconds (0 = no refresh).
    refresh_secs: u64,
}

impl PoloNotesCache {
    fn new(refresh_secs: u64) -> Self {
        Self {
            callsigns: RwLock::new(Vec::new()),
            last_fetch: AtomicU64::new(0),
            refresh_secs,
        }
    }

    fn get_callsigns(&self) -> Vec<String> {
        self.callsigns.read().unwrap().clone()
    }

    fn set_callsigns(&self, callsigns: Vec<String>) {
        *self.callsigns.write().unwrap() = callsigns;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_fetch.store(now, Ordering::Relaxed);
    }

    fn needs_refresh(&self) -> bool {
        if self.refresh_secs == 0 {
            // No refresh, but need initial fetch if never fetched
            return self.last_fetch.load(Ordering::Relaxed) == 0;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let last = self.last_fetch.load(Ordering::Relaxed);
        now >= last + self.refresh_secs
    }
}

/// Manager for PoLo notes URL fetching and caching.
///
/// Handles multiple URLs, deduplicating requests for the same URL.
pub struct PoloNotesManager {
    /// HTTP client for fetching URLs.
    client: reqwest::Client,
    /// Cache per URL.
    caches: HashMap<String, Arc<PoloNotesCache>>,
}

impl PoloNotesManager {
    /// Create a new manager from filter configurations.
    ///
    /// Extracts unique polo_notes_url values and their refresh intervals.
    pub fn from_filters(filters: &[crate::filter::SpotFilter]) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let mut caches = HashMap::new();
        for filter in filters {
            if let Some(ref url) = filter.polo_notes_url {
                if !caches.contains_key(url) {
                    let refresh_secs = filter.polo_refresh_secs.unwrap_or(DEFAULT_POLO_REFRESH_SECS);
                    caches.insert(url.clone(), Arc::new(PoloNotesCache::new(refresh_secs)));
                }
            }
        }

        Self { client, caches }
    }

    /// Check if there are any PoLo URLs configured.
    pub fn is_empty(&self) -> bool {
        self.caches.is_empty()
    }

    /// Get cached callsigns for a URL.
    ///
    /// Returns empty vec if URL not configured or not yet fetched.
    pub fn get_callsigns(&self, url: &str) -> Vec<String> {
        self.caches
            .get(url)
            .map(|c| c.get_callsigns())
            .unwrap_or_default()
    }

    /// Fetch all URLs that need refreshing.
    ///
    /// Call this periodically or on startup.
    pub async fn refresh_all(&self) {
        for (url, cache) in &self.caches {
            if cache.needs_refresh() {
                self.fetch_and_update(url, cache).await;
            }
        }
    }

    /// Fetch a single URL and update its cache.
    async fn fetch_and_update(&self, url: &str, cache: &PoloNotesCache) {
        debug!("Fetching PoLo notes from {}", url);
        match self.client.get(url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.text().await {
                        Ok(content) => {
                            let callsigns = parse_polo_notes(&content);
                            info!(
                                "Loaded {} callsigns from PoLo notes: {}",
                                callsigns.len(),
                                url
                            );
                            cache.set_callsigns(callsigns);
                        }
                        Err(e) => {
                            warn!("Failed to read PoLo notes body from {}: {}", url, e);
                        }
                    }
                } else {
                    warn!(
                        "PoLo notes fetch failed with status {}: {}",
                        response.status(),
                        url
                    );
                }
            }
            Err(e) => {
                warn!("Failed to fetch PoLo notes from {}: {}", url, e);
            }
        }
    }

    /// Start background refresh task.
    ///
    /// Returns a handle that keeps the task running. Drop to stop.
    pub fn start_background_refresh(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Find minimum refresh interval (excluding 0)
            let min_interval = self
                .caches
                .values()
                .map(|c| c.refresh_secs)
                .filter(|&s| s > 0)
                .min()
                .unwrap_or(DEFAULT_POLO_REFRESH_SECS);

            let mut ticker = interval(Duration::from_secs(min_interval));
            loop {
                ticker.tick().await;
                self.refresh_all().await;
            }
        })
    }
}
```

**Step 2: Run tests and cargo check**

Run: `cargo check && cargo test`
Expected: PASS

**Step 3: Commit**

```bash
git add src/polo.rs
git commit -m "Add PoloNotesManager for URL fetching and caching"
```

---

### Task 7: Integrate PoLo matching into SpotFilter

**Files:**
- Modify: `src/filter.rs`

**Step 1: Add polo_manager field and matching logic**

Add a new method to SpotFilter for checking PoLo callsigns:

```rust
/// Check if a spot's DX call matches the PoLo callsigns (if configured).
///
/// Returns true if:
/// - No polo_notes_url configured, OR
/// - The DX call matches any cached PoLo callsign (exact, case-insensitive)
pub fn matches_polo(&self, spot: &CwSpot, polo_manager: Option<&crate::polo::PoloNotesManager>) -> bool {
    let Some(ref url) = self.polo_notes_url else {
        return true; // No PoLo URL configured, doesn't affect matching
    };

    let Some(manager) = polo_manager else {
        return true; // No manager provided, can't check
    };

    let callsigns = manager.get_callsigns(url);
    if callsigns.is_empty() {
        return false; // No callsigns loaded yet
    }

    let dx_upper = spot.dx_call.to_uppercase();
    callsigns.iter().any(|c| c == &dx_upper)
}

/// Check if a spot matches this filter, including PoLo callsigns.
///
/// All specified fields must match (AND logic).
pub fn matches_with_polo(&self, spot: &CwSpot, polo_manager: Option<&crate::polo::PoloNotesManager>) -> bool {
    self.matches(spot) && self.matches_polo(spot, polo_manager)
}
```

**Step 2: Run tests**

Run: `cargo test`
Expected: PASS

**Step 3: Commit**

```bash
git add src/filter.rs
git commit -m "Add PoLo callsign matching to SpotFilter"
```

---

### Task 8: Integrate PoloNotesManager into main.rs

**Files:**
- Modify: `src/main.rs`

**Step 1: Add imports**

Add to imports:

```rust
use rbn_parser::polo::PoloNotesManager;
```

**Step 2: Initialize PoloNotesManager after config loading**

After config validation, add:

```rust
// Initialize PoLo notes manager if any filters use polo_notes_url
let polo_manager = Arc::new(PoloNotesManager::from_filters(&config.filters));
if !polo_manager.is_empty() {
    info!("PoLo notes: fetching initial callsigns...");
    polo_manager.refresh_all().await;

    // Start background refresh
    let pm = polo_manager.clone();
    tokio::spawn(async move {
        pm.start_background_refresh();
    });
}
```

**Step 3: Pass polo_manager to spot processing**

This depends on current main.rs structure. The manager needs to be accessible where `filter.matches()` is called. Update the spot processing to use `matches_with_polo()` instead.

**Step 4: Run and test manually**

Run: `cargo run`
Expected: Starts without errors, logs PoLo fetch if configured

**Step 5: Commit**

```bash
git add src/main.rs
git commit -m "Integrate PoloNotesManager into main application"
```

---

### Task 9: Update config parsing tests

**Files:**
- Modify: `src/config.rs` (tests)

**Step 1: Add test for array patterns in config**

Add to tests:

```rust
#[test]
fn test_parse_filters_with_arrays() {
    let toml = r#"
        callsign = "W6JSV"

        [[filters]]
        dx_call = ["W6*", "K6*", "N6*"]
        min_snr = 10

        [[filters]]
        spotter = ["EA5*", "VE7*"]
        bands = ["20m"]
    "#;
    let config: Config = toml::from_str(toml).unwrap();
    assert_eq!(config.filters.len(), 2);

    let dx_patterns = config.filters[0].dx_call.as_ref().unwrap().patterns();
    assert_eq!(dx_patterns, &["W6*", "K6*", "N6*"]);

    let spotter_patterns = config.filters[1].spotter.as_ref().unwrap().patterns();
    assert_eq!(spotter_patterns, &["EA5*", "VE7*"]);
}

#[test]
fn test_parse_filters_with_polo() {
    let toml = r#"
        callsign = "W6JSV"

        [[filters]]
        polo_notes_url = "https://example.com/notes.txt"
        polo_refresh_secs = 600
        bands = ["20m", "40m"]
    "#;
    let config: Config = toml::from_str(toml).unwrap();
    assert_eq!(config.filters[0].polo_notes_url, Some("https://example.com/notes.txt".to_string()));
    assert_eq!(config.filters[0].polo_refresh_secs, Some(600));
}
```

**Step 2: Run tests**

Run: `cargo test test_parse_filters`
Expected: PASS

**Step 3: Commit**

```bash
git add src/config.rs
git commit -m "Add config parsing tests for array patterns and PoLo fields"
```

---

### Task 10: Update beads issue and sync

**Step 1: Close the beads issue**

```bash
bd update rbn-parser-azi --status=in_progress
```

After all implementation is done:

```bash
bd close rbn-parser-azi --reason="Implemented multi-callsign filters with array support for dx_call/spotter, and PoLo notes URL integration with background refresh"
bd sync
```

**Step 2: Final commit and push**

```bash
git push
```
