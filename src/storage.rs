//! Spot storage for keeping recent matched spots in bounded per-filter queues.
//!
//! Each filter maintains its own queue with configurable maximum entries.
//! A global size limit enforces proportional eviction across all filters.
//! Each spot is assigned a per-filter sequence number for cursor-based retrieval.

use std::collections::VecDeque;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

use serde::Serialize;

use crate::config::StorageConfig;
use crate::filter::SpotFilter;
use crate::spot::CwSpot;

/// A spot with its sequence number for storage.
#[derive(Debug, Clone, Serialize)]
pub struct StoredSpot {
    /// Per-filter sequence number (monotonically increasing, may have gaps).
    pub seq: u64,
    /// The actual spot data.
    pub spot: CwSpot,
}

/// Per-filter storage queue.
pub struct FilterStorage {
    /// Filter name (from config, or generated like "filter_0").
    pub name: String,

    /// Maximum entries for this filter.
    pub max_kept_entries: usize,

    /// The bounded queue of spots with sequence numbers.
    spots: VecDeque<StoredSpot>,

    /// Next sequence number to assign (starts at 1).
    next_seq: AtomicU64,

    /// Count of spots evicted due to limits (per-filter or global).
    pub overflow_count: AtomicU64,

    /// Current size in bytes of stored spots.
    pub current_size_bytes: AtomicUsize,
}

impl FilterStorage {
    /// Create a new filter storage.
    pub fn new(name: String, max_kept_entries: usize) -> Self {
        Self {
            name,
            max_kept_entries,
            spots: VecDeque::new(),
            next_seq: AtomicU64::new(1),
            overflow_count: AtomicU64::new(0),
            current_size_bytes: AtomicUsize::new(0),
        }
    }

    /// Number of spots currently stored.
    pub fn len(&self) -> usize {
        self.spots.len()
    }

    /// Whether the storage is empty.
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }

    /// Get the latest sequence number in storage (0 if empty).
    pub fn latest_seq(&self) -> u64 {
        self.spots.back().map(|s| s.seq).unwrap_or(0)
    }

    /// Get spots with sequence number greater than `since`.
    pub fn get_spots_since(&self, since: u64) -> Vec<StoredSpot> {
        self.spots
            .iter()
            .filter(|s| s.seq > since)
            .cloned()
            .collect()
    }

    /// Push a spot, returning its size in bytes.
    fn push(&mut self, spot: CwSpot) -> usize {
        let size = spot.json_size();
        let seq = self.next_seq.fetch_add(1, Relaxed);
        self.spots.push_back(StoredSpot { seq, spot });
        self.current_size_bytes.fetch_add(size, Relaxed);
        size
    }

    /// Pop the oldest spot, returning its size in bytes if any was removed.
    fn pop_oldest(&mut self) -> Option<usize> {
        self.spots.pop_front().map(|stored| {
            let size = stored.spot.json_size();
            self.current_size_bytes.fetch_sub(size, Relaxed);
            self.overflow_count.fetch_add(1, Relaxed);
            size
        })
    }
}

/// Central storage manager for all filters.
pub struct SpotStorage {
    /// Global maximum size in bytes.
    global_max_size: usize,

    /// Per-filter storage (filter + its storage).
    filters: Vec<(SpotFilter, RwLock<FilterStorage>)>,

    /// Total bytes across all filter storages.
    pub total_size_bytes: AtomicUsize,

    /// Count of global evictions (evictions due to global_max_size).
    pub global_evictions: AtomicU64,
}

impl SpotStorage {
    /// Create a new spot storage from config.
    pub fn new(config: &StorageConfig, filters: Vec<SpotFilter>) -> Self {
        let filter_storages: Vec<_> = filters
            .into_iter()
            .enumerate()
            .map(|(i, filter)| {
                let name = filter
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("filter_{}", i));
                let max_entries = filter
                    .max_kept_entries
                    .unwrap_or(config.default_max_kept_entries);
                let storage = FilterStorage::new(name, max_entries);
                (filter, RwLock::new(storage))
            })
            .collect();

        Self {
            global_max_size: config.global_max_size,
            filters: filter_storages,
            total_size_bytes: AtomicUsize::new(0),
            global_evictions: AtomicU64::new(0),
        }
    }

    /// Store a spot that matched the filter at the given index.
    ///
    /// Handles both per-filter and global limit enforcement with eviction.
    pub fn store_spot(&self, filter_index: usize, spot: CwSpot) {
        let spot_size = spot.json_size();

        // Enforce global limit by evicting from largest filter
        while self.total_size_bytes.load(Relaxed) + spot_size > self.global_max_size {
            if !self.evict_from_largest_filter() {
                // No spots to evict, can't store
                return;
            }
            self.global_evictions.fetch_add(1, Relaxed);
        }

        // Get the filter's storage
        let (_, storage_lock) = &self.filters[filter_index];
        let mut storage = storage_lock.write().unwrap();

        // Enforce per-filter limit
        while storage.len() >= storage.max_kept_entries {
            if let Some(removed_size) = storage.pop_oldest() {
                self.total_size_bytes.fetch_sub(removed_size, Relaxed);
            } else {
                break;
            }
        }

        // Add the new spot
        let added_size = storage.push(spot);
        self.total_size_bytes.fetch_add(added_size, Relaxed);
    }

    /// Try to match a spot against all filters and store in matching ones.
    ///
    /// Returns the indices of filters that matched.
    pub fn try_store(&self, spot: &CwSpot) -> Vec<usize> {
        let mut matched = Vec::new();
        for (i, (filter, _)) in self.filters.iter().enumerate() {
            if filter.matches(spot) {
                self.store_spot(i, spot.clone());
                matched.push(i);
            }
        }
        matched
    }

    /// Evict one spot from the filter with the most entries.
    ///
    /// Returns true if a spot was evicted, false if all filters are empty.
    fn evict_from_largest_filter(&self) -> bool {
        // Find the filter with the most entries
        let mut max_len = 0;
        let mut max_idx = None;

        for (i, (_, storage_lock)) in self.filters.iter().enumerate() {
            let storage = storage_lock.read().unwrap();
            if storage.len() > max_len {
                max_len = storage.len();
                max_idx = Some(i);
            }
        }

        // Evict from it
        if let Some(idx) = max_idx {
            let (_, storage_lock) = &self.filters[idx];
            let mut storage = storage_lock.write().unwrap();
            if let Some(removed_size) = storage.pop_oldest() {
                self.total_size_bytes.fetch_sub(removed_size, Relaxed);
                return true;
            }
        }

        false
    }

    /// Get the number of filters.
    pub fn filter_count(&self) -> usize {
        self.filters.len()
    }

    /// Get the global max size configuration.
    pub fn global_max_size(&self) -> usize {
        self.global_max_size
    }

    /// Iterate over filter storages for metrics collection.
    pub fn iter_storages(&self) -> impl Iterator<Item = &(SpotFilter, RwLock<FilterStorage>)> {
        self.filters.iter()
    }

    /// Get list of all filter names.
    pub fn filter_names(&self) -> Vec<String> {
        self.filters
            .iter()
            .map(|(_, storage_lock)| {
                let storage = storage_lock.read().unwrap();
                storage.name.clone()
            })
            .collect()
    }

    /// Get a filter's storage by name.
    ///
    /// Returns None if the filter name doesn't exist.
    pub fn get_filter_by_name(&self, name: &str) -> Option<&RwLock<FilterStorage>> {
        self.filters.iter().find_map(|(_, storage_lock)| {
            let storage = storage_lock.read().unwrap();
            if storage.name == name {
                drop(storage); // Release the read lock before returning
                Some(storage_lock)
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spot::{Mode, SpotType};
    use chrono::NaiveTime;

    fn make_spot(dx_call: &str) -> CwSpot {
        CwSpot {
            spotter: "TEST-#".to_string(),
            frequency_khz: 14025.0,
            dx_call: dx_call.to_string(),
            mode: Mode::Cw,
            snr_db: 15,
            wpm: 20,
            spot_type: SpotType::Cq,
            time: NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn test_filter_storage_basic() {
        let mut storage = FilterStorage::new("test".to_string(), 3);

        storage.push(make_spot("W1AW"));
        assert_eq!(storage.len(), 1);

        storage.push(make_spot("W2AW"));
        storage.push(make_spot("W3AW"));
        assert_eq!(storage.len(), 3);
    }

    #[test]
    fn test_per_filter_limit() {
        let config = StorageConfig {
            default_max_kept_entries: 2,
            global_max_size: 10 * 1024 * 1024, // 10MB, won't hit
        };

        let filter: SpotFilter = toml::from_str(r#"dx_call = "W*""#).unwrap();

        let storage = SpotStorage::new(&config, vec![filter]);

        // Store 3 spots, should only keep 2
        storage.store_spot(0, make_spot("W1AW"));
        storage.store_spot(0, make_spot("W2AW"));
        storage.store_spot(0, make_spot("W3AW"));

        let (_, fs_lock) = &storage.filters[0];
        let fs = fs_lock.read().unwrap();
        assert_eq!(fs.len(), 2);
        assert_eq!(fs.overflow_count.load(Relaxed), 1);
    }

    #[test]
    fn test_global_limit_eviction() {
        // Very small global limit to force eviction
        let spot_size = make_spot("W1AW").json_size();
        let config = StorageConfig {
            default_max_kept_entries: 100,
            global_max_size: spot_size * 2 + 1, // Allow ~2 spots
        };

        let filter1 = SpotFilter {
            name: Some("filter_a".to_string()),
            ..Default::default()
        };
        let filter2 = SpotFilter {
            name: Some("filter_b".to_string()),
            ..Default::default()
        };

        let storage = SpotStorage::new(&config, vec![filter1, filter2]);

        // Store 2 spots in filter 0
        storage.store_spot(0, make_spot("W1AW"));
        storage.store_spot(0, make_spot("W2AW"));

        // Store 1 spot in filter 1, should trigger global eviction from filter 0
        storage.store_spot(1, make_spot("W3AW"));

        // Total should be ~2 spots worth
        assert!(storage.total_size_bytes.load(Relaxed) <= config.global_max_size);
        assert!(storage.global_evictions.load(Relaxed) >= 1);
    }

    #[test]
    fn test_try_store_matches() {
        let config = StorageConfig {
            default_max_kept_entries: 10,
            global_max_size: 10 * 1024 * 1024,
        };

        let filter1: SpotFilter = toml::from_str(r#"dx_call = "W6*""#).unwrap();
        let filter2: SpotFilter = toml::from_str(r#"bands = ["20m"]"#).unwrap();

        let storage = SpotStorage::new(&config, vec![filter1, filter2]);

        // W6JSV on 20m matches both filters
        let spot = make_spot("W6JSV");
        let matched = storage.try_store(&spot);
        assert_eq!(matched, vec![0, 1]);

        // K1ABC on 20m matches only filter 2
        let spot2 = make_spot("K1ABC");
        let matched2 = storage.try_store(&spot2);
        assert_eq!(matched2, vec![1]);
    }

    #[test]
    fn test_sequence_numbers() {
        let config = StorageConfig {
            default_max_kept_entries: 10,
            global_max_size: 10 * 1024 * 1024,
        };

        let filter = SpotFilter {
            name: Some("test_filter".to_string()),
            ..Default::default()
        };

        let storage = SpotStorage::new(&config, vec![filter]);

        // Store 3 spots
        storage.store_spot(0, make_spot("W1AW"));
        storage.store_spot(0, make_spot("W2AW"));
        storage.store_spot(0, make_spot("W3AW"));

        let (_, fs_lock) = &storage.filters[0];
        let fs = fs_lock.read().unwrap();

        // Check sequence numbers are assigned correctly
        assert_eq!(fs.latest_seq(), 3);
        assert_eq!(fs.len(), 3);

        // Get all spots
        let all_spots = fs.get_spots_since(0);
        assert_eq!(all_spots.len(), 3);
        assert_eq!(all_spots[0].seq, 1);
        assert_eq!(all_spots[1].seq, 2);
        assert_eq!(all_spots[2].seq, 3);

        // Get spots since seq 1 (should get spots 2 and 3)
        let recent_spots = fs.get_spots_since(1);
        assert_eq!(recent_spots.len(), 2);
        assert_eq!(recent_spots[0].seq, 2);
        assert_eq!(recent_spots[1].seq, 3);

        // Get spots since seq 3 (should be empty)
        let no_spots = fs.get_spots_since(3);
        assert!(no_spots.is_empty());
    }

    #[test]
    fn test_filter_names() {
        let config = StorageConfig {
            default_max_kept_entries: 10,
            global_max_size: 10 * 1024 * 1024,
        };

        let filter1 = SpotFilter {
            name: Some("w6_calls".to_string()),
            ..Default::default()
        };
        let filter2 = SpotFilter {
            // No name, should get auto-generated name
            ..Default::default()
        };

        let storage = SpotStorage::new(&config, vec![filter1, filter2]);

        let names = storage.filter_names();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "w6_calls");
        assert_eq!(names[1], "filter_1");

        // Get by name
        assert!(storage.get_filter_by_name("w6_calls").is_some());
        assert!(storage.get_filter_by_name("filter_1").is_some());
        assert!(storage.get_filter_by_name("nonexistent").is_none());
    }
}
