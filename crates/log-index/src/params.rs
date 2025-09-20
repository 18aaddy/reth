//! Row and column mapping algorithms for `FilterMaps` based on EIP-7745.
//! see: <https://eips.ethereum.org/EIPS/eip-7745>

use alloy_primitives::B256;
use fnv::FnvHasher;

use sha2::{Digest, Sha256};
use std::hash::Hasher;

use crate::{
    constants::{DEFAULT_PARAMS, EXPECTED_MATCHES, RANGE_TEST_PARAMS},
    hasher::{GTI_EPOCHS, GTI_LOG_ENTRIES},
    types::{FilterError, FilterMapRow, MapRowIndex},
    TreeIndex,
};

/// A strictly monotonically increasing list of log value indices in the range of a
/// filter map that are potential matches for certain filter criteria.
///
/// Note that nil is used as a wildcard and therefore means that all log value
/// indices in the filter map range are potential matches. If there are no
/// potential matches in the given map's range then an empty slice should be used.
type PotentialMatches = Vec<u64>;

/// `FilterMaps` parameters based on EIP-7745
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterMapParams {
    /// Log of map height (rows per map). Default: 16 (65,536 rows)
    pub log_map_height: u64,
    /// Log of map width (columns per row). Default: 24 (16,777,216 columns)
    pub log_map_width: u64,
    /// Log of maps per epoch. Default: 10 (1,024 maps)
    pub log_maps_per_epoch: u64,
    /// Log of values per map. Default: 16 (65,536 values)
    pub log_values_per_map: u64,
    /// baseRowLength / average row length
    pub base_row_length_ratio: u64,
    /// maxRowLength log2 growth per layer
    pub log_layer_diff: u64,
    ///
    pub log_epoch_history: u64,

    // Derived fields
    ///
    pub map_height: u32,
    ///
    pub maps_per_epoch: u32,
    ///
    pub base_row_length: u32,
    ///
    pub values_per_map: u64,

    /// (Not affecting consensus)
    pub base_row_group_length: u64,
}

impl Default for FilterMapParams {
    fn default() -> Self {
        DEFAULT_PARAMS
    }
}

impl FilterMapParams {
    /// Test parameters that put one log value per epoch, ensuring block exact tail unindexing for
    /// testing
    pub fn range_test() -> Self {
        RANGE_TEST_PARAMS
    }
    /// Get the number of rows per map
    pub const fn map_height(&self) -> u64 {
        1 << self.log_map_height
    }

    /// Get the number of columns per row
    pub const fn map_width(&self) -> u64 {
        1 << self.log_map_width
    }

    /// Get the number of maps per epoch
    pub const fn maps_per_epoch(&self) -> u64 {
        1 << self.log_maps_per_epoch
    }

    /// Get the number of values per map
    pub const fn values_per_map(&self) -> u64 {
        1 << self.log_values_per_map
    }

    /// Get the base row length
    pub const fn base_row_length(&self) -> u64 {
        (self.values_per_map() * self.base_row_length_ratio) / self.map_height()
    }

    /// Calculate the minimum number of layers required to store all values.
    ///
    /// This is based on the average row length and the maximum row length
    /// allowed at each layer.
    pub fn required_layers(&self) -> u64 {
        // Average row length is values_per_map / map_height
        // Use integer arithmetic to avoid float comparisons
        let values_per_map = self.values_per_map();
        let map_height = self.map_height() as u64;

        // Find the layer where max_row_length * map_height >= values_per_map
        let mut layer = 0u64;
        while self.max_row_length(layer) * map_height < values_per_map {
            layer += 1;
            if layer >= crate::constants::MAX_LAYERS {
                break;
            }
        }

        // Add one more layer for safety
        (layer + 1).min(crate::constants::MAX_LAYERS)
    }

    /// Calculate the row index in which the given log value should be marked
    /// on the given map and mapping layer.
    ///
    /// Note that row assignments are re-shuffled with a different frequency on each
    /// mapping layer, allowing efficient disk access and Merkle proofs for long
    /// sections of short rows on lower order layers while avoiding putting too many
    /// heavy rows next to each other on higher order layers.
    pub fn row_index(&self, map_index: u64, layer_index: u64, log_value: &B256) -> u64 {
        let mut hasher = Sha256::new();

        // First, add the log_value
        hasher.update(log_value.as_slice());

        // Calculate masked_map_index according to EIP
        let masked_index = self.masked_map_index(map_index, layer_index);

        // Add masked_map_index as 32-bit big-endian
        hasher.update(&(masked_index as u32).to_be_bytes());

        // Add layer_index as 32-bit big-endian
        hasher.update(&(layer_index as u32).to_be_bytes());

        let hash = hasher.finalize();

        // Take first 4 bytes as big-endian u32 and modulo by map height
        let row = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) as u64;
        row % self.map_height()
    }

    /// Returns the column index where the given log value at the given
    /// position should be marked.
    pub fn column_index(&self, lv_index: u64, log_value: &B256) -> u64 {
        let mut hasher = FnvHasher::default();
        hasher.write(&lv_index.to_be_bytes());
        hasher.write(log_value.as_slice());
        let hash = hasher.finish();

        let log_value_width = 1 << (self.log_map_width - self.log_values_per_map);

        // Calculate collision_filter according to EIP
        let shift_bits = self.log_map_width - self.log_values_per_map;
        let part1 = hash >> (64 - shift_bits);
        let part2 = hash >> (32 - shift_bits);
        let collision_filter = (part1 + part2) % log_value_width;
        // Combine position with collision filter
        let position_in_map = lv_index % self.values_per_map();

        position_in_map * log_value_width + collision_filter
    }

    /// Returns the index used for row mapping calculation on the given layer.
    ///
    /// On layer zero the mapping changes once per epoch, then the frequency of
    /// re-mapping increases with every new layer until it reaches the frequency
    /// where it is different for every mapIndex.
    pub fn masked_map_index(&self, map_index: u64, layer_index: u64) -> u64 {
        let layer_factor = (1u64 << (layer_index * self.log_layer_diff)).min(self.maps_per_epoch());
        let row_frequency = self.maps_per_epoch() / layer_factor;
        map_index - (map_index % row_frequency)
    }
    /// Returns the maximum length filter rows are populated up to when using the
    /// given mapping layer.
    ///
    /// A log value can be marked on the map according to a
    /// given mapping layer if the row mapping on that layer points to a row that
    /// has not yet reached the maxRowLength belonging to that layer.
    /// This means that a row that is considered full on a given layer may still be
    /// extended further on a higher order layer.
    /// Each value is marked on the lowest order layer possible, assuming that marks
    /// are added in ascending log value index order.
    /// When searching for a log value one should consider all layers and process
    /// corresponding rows up until the first one where the row mapped to the given
    /// layer is not full.
    pub fn max_row_length(&self, layer_index: u64) -> u64 {
        let log_layer_diff = (layer_index * self.log_layer_diff).min(self.log_maps_per_epoch);
        self.base_row_length() << log_layer_diff
    }

    /// Extract the position-within-map from a column value
    /// Column value format: [position_bits | collision_filter_bits]
    /// We extract the upper position_bits
    pub fn position_from_column(&self, column_value: u64) -> u64 {
        column_value >> (self.log_map_width - self.log_values_per_map)
    }

    /// Returns the list of log value indices potentially matching the given log
    /// value hash in the range of the filter map the row belongs to.
    ///
    /// Note that the list of indices is always sorted and potential duplicates
    /// are removed. Though the column indices are stored in the same order they
    /// were added and therefore the true matches are automatically reverse
    /// transformed in the right order, false positives can ruin this property.
    /// Since these can only be separated from true matches after the combined
    /// pattern matching of the outputs of individual log value matchers and this
    /// pattern matcher assumes a sorted and duplicate-free list of indices, we
    /// should ensure these properties here.
    pub fn potential_matches(
        &self,
        rows: &[FilterMapRow],
        map_index: u64,
        log_value: &B256,
    ) -> Result<PotentialMatches, FilterError> {
        let mut results = Vec::with_capacity(EXPECTED_MATCHES);
        let map_first_log_value_index = map_index << self.log_values_per_map;

        for row in rows {
            // Check each column in the row up to the effective length
            for &column_value in &row.columns {
                let potential_match =
                    map_first_log_value_index + self.position_from_column(column_value);

                let expected_column = self.column_index(potential_match, log_value);

                if column_value == expected_column {
                    results.push(potential_match);
                }
            }
        }

        // Sort and deduplicate results
        results.sort_unstable();
        results.dedup();
        Ok(results)
    }

    /// Calculate the global row index for database storage.
    ///
    /// This ensures rows from the same map are stored together.
    pub const fn global_row_index(&self, map_index: u64, row_index: u64) -> MapRowIndex {
        map_index * self.map_height() + row_index
    }

    /// Get the map index from a global row index.
    pub const fn map_from_global_row(&self, global_row: MapRowIndex) -> u64 {
        global_row / self.map_height()
    }

    /// Get the row index from a global row index.
    pub const fn row_from_global_row(&self, global_row: MapRowIndex) -> u64 {
        global_row % self.map_height()
    }

    pub fn gti_epoch_root(&self, epoch: u32) -> TreeIndex {
        GTI_EPOCHS.append(epoch.into(), self.log_epoch_history)
    }

    pub fn gti_log_entry_root(&self, lv_index: u64) -> TreeIndex {
        let log_tree_height = self.log_maps_per_epoch + self.log_values_per_map;
        self.gti_epoch_root((lv_index >> log_tree_height) as u32)
            .child(GTI_LOG_ENTRIES)
            .append(lv_index & (1 << log_tree_height - 1), log_tree_height)
    }

    ///
    pub fn derive_fields(&mut self) {
        self.map_height = 1<<self.log_map_height;
        self.maps_per_epoch = 1<<self.log_maps_per_epoch;
        self.values_per_map = 1 << self.log_values_per_map;
        self.base_row_length = (self.values_per_map * self.base_row_length_ratio / self.map_height as u64) as u32;
    }
}

/// Calculate the global row index for database storage.
///
/// This ensures rows from the same map are stored together.
#[allow(dead_code)]
pub(crate) const fn map_row_index(map_index: u64, row_index: u64, params: &FilterMapParams) -> u64 {
    (map_index * params.map_height()) + row_index
}

/// Get the map index from a log value index.
#[allow(dead_code)]
pub(crate) const fn map_index_from_lv_index(lv_index: u64, params: &FilterMapParams) -> u64 {
    lv_index >> params.log_values_per_map
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::b256;
    use rand::{rng, Rng};

    #[test]
    fn test_row_index_distribution() {
        let params = FilterMapParams::default();
        let value1 = b256!("0000000000000000000000000000000000000000000000000000000000000001");
        let value2 = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        // Same value should always map to same row for same map/layer
        let row1 = params.row_index(0, 0, &value1);
        let row2 = params.row_index(0, 0, &value1);
        assert_eq!(row1, row2);

        // Different values should (likely) map to different rows
        let row3 = params.row_index(0, 0, &value2);
        // Different values should produce different rows in most cases
        // We can't assert they're always different due to possible hash collisions
        // but we can verify they're valid row indices
        assert!(row3 < params.map_height());

        // Different layers should give different row mappings for the same value
        let row_layer0 = params.row_index(0, 0, &value1);
        let row_layer1 = params.row_index(0, 1, &value1);
        let row_layer2 = params.row_index(0, 2, &value1);

        // All should be valid indices
        assert!(row_layer0 < params.map_height());
        assert!(row_layer1 < params.map_height());
        assert!(row_layer2 < params.map_height());

        // At least one pair should differ (extremely unlikely all three are the same)
        assert!(
            row_layer0 != row_layer1 || row_layer1 != row_layer2 || row_layer0 != row_layer2,
            "All three layers produced the same row index, which is extremely unlikely"
        );
    }

    #[test]
    fn test_column_index() {
        let params = FilterMapParams::default();
        let value = b256!("0000000000000000000000000000000000000000000000000000000000000001");

        // Column index should include position information
        let col1 = params.column_index(0, &value);
        let col2 = params.column_index(params.values_per_map(), &value);

        // Both should be in same position (0) but with different hash bits
        assert_eq!(col1 >> 8, 0);
        assert_eq!(col2 >> 8, 0);

        // Different positions
        let col3 = params.column_index(1, &value);
        assert_eq!(col3 >> 8, 1);
    }

    #[test]
    fn test_single_match() {
        let params = FilterMapParams::default();

        for _ in 0..100_000 {
            // Generate a row with a single random entry
            let mut rng = rng();
            let map_index = rng.random::<u64>();

            let lv_index = map_index <<
                params.log_values_per_map + rng.random_range(0..params.values_per_map());

            // Generate random hash
            let mut lv_hash_bytes = [0u8; 32];
            rng.fill(&mut lv_hash_bytes);
            let lv_hash = B256::from(lv_hash_bytes);

            let row = FilterMapRow { columns: vec![params.column_index(lv_index, &lv_hash)] };
            let matches = params.potential_matches(&[row], map_index, &lv_hash).unwrap();

            assert_eq!(
                matches.len(),
                1,
                "Invalid length of matches (got {}, expected 1)",
                matches.len()
            );
            assert_eq!(
                matches[0], lv_index,
                "Incorrect match returned (got {}, expected {})",
                matches[0], lv_index
            );
        }
    }

    const TEST_PM_COUNT: usize = 50;
    const TEST_PM_LEN: usize = 1000;

    // TODO: I dont understand this test fully. this has been copied from go-ethereum.
    #[test]
    fn test_potential_matches() {
        let params = FilterMapParams::default();
        let mut false_positives = 0;

        for _ in 0..TEST_PM_COUNT {
            let mut rng = rng();
            let map_index = rng.random::<u64>();
            let lv_start = map_index << params.log_values_per_map;
            let mut row_columns = Vec::new();
            let mut lv_indices = Vec::with_capacity(TEST_PM_LEN);
            let mut lv_hashes = Vec::with_capacity(TEST_PM_LEN + 1);

            // Add TEST_PM_LEN single entries with different log value hashes at different indices
            for _ in 0..TEST_PM_LEN {
                let lv_index = lv_start + rng.random_range(0..params.values_per_map());
                lv_indices.push(lv_index);

                let mut lv_hash_bytes = [0u8; 32];
                rng.fill(&mut lv_hash_bytes);
                let lv_hash = B256::from(lv_hash_bytes);
                lv_hashes.push(lv_hash);

                row_columns.push(params.column_index(lv_index, &lv_hash));
            }

            // Add the same log value hash at the first TEST_PM_LEN log value indices of the map's
            // range
            let mut common_hash_bytes = [0u8; 32];
            rng.fill(&mut common_hash_bytes);
            let common_hash = B256::from(common_hash_bytes);
            lv_hashes.push(common_hash);

            for lv_index in lv_start..lv_start + TEST_PM_LEN as u64 {
                row_columns.push(params.column_index(lv_index, &common_hash));
            }

            // Randomly duplicate some entries
            for _ in 0..TEST_PM_LEN {
                let random_entry = row_columns[rng.random_range(0..row_columns.len())];
                row_columns.push(random_entry);
            }

            // Randomly mix up order of elements
            for i in (1..row_columns.len()).rev() {
                let j = rng.random_range(0..i);
                row_columns.swap(i, j);
            }

            // Split up into a list of rows if longer than allowed
            let mut rows = Vec::new();
            let mut remaining_columns = Some(row_columns);
            let mut layer_index = 0;

            while let Some(mut columns) = remaining_columns {
                let max_len = params.max_row_length(layer_index) as usize;
                if columns.len() > max_len {
                    let rest = columns.split_off(max_len);
                    rows.push(FilterMapRow { columns });
                    remaining_columns = Some(rest);
                } else {
                    rows.push(FilterMapRow { columns });
                    remaining_columns = None;
                }
                layer_index += 1;
            }

            // Check retrieved matches while also counting false positives
            for (i, lv_hash) in lv_hashes.iter().enumerate() {
                let matches = params.potential_matches(&rows, map_index, lv_hash).unwrap();

                if i < TEST_PM_LEN {
                    // Check single entry match
                    assert!(
                        !matches.is_empty(),
                        "Invalid length of matches (got {}, expected >=1)",
                        matches.len()
                    );

                    let mut found = false;
                    for &lvi in &matches {
                        if lvi == lv_indices[i] {
                            found = true;
                        } else {
                            false_positives += 1;
                        }
                    }

                    assert!(
                        found,
                        "Expected match not found (got {:?}, expected {})",
                        matches, lv_indices[i]
                    );
                } else {
                    // Check "long series" match
                    assert!(
                        matches.len() >= TEST_PM_LEN,
                        "Invalid length of matches (got {}, expected >={})",
                        matches.len(),
                        TEST_PM_LEN
                    );

                    // Since results are ordered, first TEST_PM_LEN entries should match exactly
                    for (j, _) in matches.iter().enumerate().take(TEST_PM_LEN) {
                        assert_eq!(
                            matches[j],
                            lv_start + j as u64,
                            "Incorrect match at index {} (got {}, expected {})",
                            j,
                            matches[j],
                            lv_start + j as u64
                        );
                    }

                    // The rest are false positives
                    false_positives += matches.len() - TEST_PM_LEN;
                }
            }
        }
        println!("Total false positives: {false_positives}");
    }
    #[test]
    fn test_required_layers() {
        let params = FilterMapParams::default();
        let layers = params.required_layers();

        // Should have reasonable number of layers
        assert!(layers > 0);
        assert!(layers <= crate::constants::MAX_LAYERS);

        // The max row length at the last layer should be >= average row length
        let avg_row_length = (params.values_per_map() as f64) / (params.map_height() as f64);
        let max_at_last_layer = params.max_row_length(layers - 1) as f64;
        assert!(
            max_at_last_layer >= avg_row_length,
            "Max row length at layer {} ({}) should be >= average row length ({})",
            layers - 1,
            max_at_last_layer,
            avg_row_length
        );
    }

    #[test]
    fn test_map_row_index() {
        let params = FilterMapParams::default();

        // First row of first map
        assert_eq!(map_row_index(0, 0, &params), 0);

        // Last row of first map
        let last_row = params.map_height() - 1;
        assert_eq!(map_row_index(0, last_row, &params), last_row as u64);

        // First row of second map
        assert_eq!(map_row_index(1, 0, &params), params.map_height() as u64);

        // Random row in random map
        let map_idx = 42;
        let row_idx = 1337;
        let expected = map_idx * params.map_height() + row_idx;
        assert_eq!(map_row_index(map_idx, row_idx, &params), expected);
    }

    #[test]
    fn test_map_index_from_lv_index() {
        let params = FilterMapParams::default();

        // First log value should be in map 0
        assert_eq!(map_index_from_lv_index(0, &params), 0);

        // Last log value of first map should still be in map 0
        let last_lv_first_map = params.values_per_map() - 1;
        assert_eq!(map_index_from_lv_index(last_lv_first_map, &params), 0);

        // First log value of second map should be in map 1
        assert_eq!(map_index_from_lv_index(params.values_per_map(), &params), 1);

        // Random log value
        let lv_index = 123456789u64;
        let expected_map = lv_index >> params.log_values_per_map;
        assert_eq!(map_index_from_lv_index(lv_index, &params), expected_map);
    }
}
