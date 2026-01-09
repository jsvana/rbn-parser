//! Ham2K PoLo callsign notes file support.
//!
//! Parses callsign notes files and manages URL fetching with background refresh.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::interval;
use tracing::{debug, info, warn};

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
            if let Some(ref url) = filter.polo_notes_url
                && !caches.contains_key(url)
            {
                let refresh_secs = filter
                    .polo_refresh_secs
                    .unwrap_or(DEFAULT_POLO_REFRESH_SECS);
                caches.insert(url.clone(), Arc::new(PoloNotesCache::new(refresh_secs)));
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
