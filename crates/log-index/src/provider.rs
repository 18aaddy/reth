//! Storage types for filter maps (EIP-7745).
//!
//! This module defines the storage representations of filter maps data
//! that can be stored in the database.

use crate::{
    constants::MAX_LAYERS,
    params::FilterMapParams,
    types::{FilterError, FilterMapMetadata, FilterMapRow, FilterResult, MapRowIndex},
    utils::{address_value, topic_value},
};
use alloy_primitives::{Address, BlockNumber, FixedBytes, B256};
use std::{ops::RangeInclusive, vec::Vec};

/// Provider trait for reading filter map data.
pub trait FilterMapsReader: Send + Sync {
    /// Get filter map metadata.
    fn get_metadata(&self) -> FilterResult<Option<FilterMapMetadata>>;

    /// Get filter map row for a given map and row index.
    /// Returns None if the row is not found.
    fn get_filter_map_row(&self, global_row_index: u64) -> FilterResult<Option<FilterMapRow>>;

    /// Get the log value index for a given block number.
    fn get_log_value_index_for_block(&self, block: BlockNumber) -> FilterResult<Option<u64>>;

    /// Find which block contains the given log value index, using binary search.
    /// Returns None if the log index is beyond the indexed range.
    ///
    /// The returned tuple contains the block number and the log value index of the first log in the block.
    fn find_block_for_log_value_index(
        &self,
        log_value_index: u64,
    ) -> FilterResult<Option<(BlockNumber, u64)>> {
        // Check if we are in indexed range
        let metadata = self.get_metadata()?;
        let meta = match metadata {
            Some(meta) => meta,
            None => return Ok(None),
        };

        // Check if log index is beyond the indexed range
        if log_value_index >= meta.next_log_value_index {
            return Ok(None);
        }

        // Binary search for the block
        let mut start = meta.first_indexed_block;
        let mut end = meta.last_indexed_block;
        let mut result = None;

        while start <= end {
            let mid = start + (end - start) / 2;

            // get delimiter for the block
            let block_log_value_index = self.get_log_value_index_for_block(mid)?;
            if let Some(block_log_value_index) = block_log_value_index {
                if log_value_index >= block_log_value_index {
                    // this block starts before or at our target
                    result = Some((mid, block_log_value_index));
                    start = mid + 1;
                } else {
                    // this block starts after our target
                    end = mid.saturating_sub(1);
                }
            } else {
                println!("no delimiter found for block: {:?}", mid);
                // no delimiter found, this shouldnt happen in valid data
                return Err(FilterError::Database(
                    "No start log value index found for block".to_string(),
                ));
            }
        }

        Ok(result)
    }
    /// Check if a block range is fully indexed.
    fn is_block_range_indexed(&self, range: RangeInclusive<BlockNumber>) -> FilterResult<bool> {
        let metadata = self.get_metadata()?;
        let meta = match metadata {
            Some(meta) => meta,
            None => return Ok(false),
        };

        Ok(range.start() >= &meta.first_indexed_block && range.end() <= &meta.last_indexed_block)
    }

    /// Performs a log query using `FilterMaps`.
    ///
    /// This is the main entry point for log filtering using the `FilterMaps` data structure.
    /// It constructs the appropriate matcher hierarchy and processes the query efficiently.
    ///
    /// # Arguments
    ///
    /// * `provider` - The provider for accessing `FilterMaps` data
    /// * `first_block` - The first block to search (inclusive)
    /// * `last_block` - The last block to search (inclusive)
    /// * `addresses` - List of addresses to match (empty = match all)
    /// * `topics` - List of topic filters, where each position can have multiple values (OR)
    ///
    /// # Returns
    ///
    /// A list of logs matching the filter criteria. Note that false positives are possible
    /// and should be filtered out by the caller if exact matching is required.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The filter matches all logs (use legacy filtering instead)
    /// - There's an issue accessing the `FilterMaps` data
    fn query_logs(
        &self,
        range: RangeInclusive<BlockNumber>,
        address: Address,
        topics: Vec<B256>,
    ) -> FilterResult<Vec<u64>> {
        let params = FilterMapParams::default(); // TODO: figure out a better way to get the params

        let first_index = self.get_log_value_index_for_block(*range.start())?.unwrap_or_default();

        let last_index = self.get_log_value_index_for_block(*range.end())?.unwrap_or_default();

        // Calculate map indices to search
        let first_map = first_index >> params.log_values_per_map;
        let last_map = last_index >> params.log_values_per_map;
        let map_indices: Vec<u64> = (first_map..=last_map).collect();

        let address_value = address_value(&address);
        let topic_values: Vec<FixedBytes<32>> =
            topics.iter().map(|topic| topic_value(topic)).collect();
        let mut log_indices = Vec::new();

        for map_index in map_indices {
            let address_matches =
                self.get_potential_matches_in_map(map_index, &address_value, &params)?;

            // for the address matches, we need to check if the subsequent topics match the expected indices.
            'address_loop: for address_match in address_matches {
                let mut expected_topic_index = address_match + 1; // the first topic should appear after the address match, so we start at address_match + 1
                for expected_topic_value in topic_values.iter() {
                    if !self.check_value_at_index(
                        expected_topic_value,
                        expected_topic_index,
                        &params,
                    )? {
                        continue 'address_loop;
                    }
                    expected_topic_index += 1;
                }

                log_indices.push(address_match);
            }
        }

        Ok(log_indices)
    }

    fn get_potential_matches_in_map(
        &self,
        map_index: u64,
        address_value: &B256,
        params: &FilterMapParams,
    ) -> FilterResult<Vec<u64>> {
        let rows = self.get_filter_map_rows(map_index, address_value, params)?;
        params.potential_matches(&rows, map_index, address_value)
    }

    fn check_value_at_index(
        &self,
        log_value: &B256,
        expected_log_value_index: u64,
        params: &FilterMapParams,
    ) -> FilterResult<bool> {
        let map_index = expected_log_value_index >> params.log_values_per_map;

        let matches = self.get_potential_matches_in_map(map_index, log_value, params)?;
        Ok(matches.contains(&expected_log_value_index))
    }

    fn get_filter_map_rows(
        &self,
        map_index: u64,
        log_value: &B256,
        params: &FilterMapParams,
    ) -> FilterResult<Vec<FilterMapRow>> {
        // TODO: Clean this up.
        let mut rows = Vec::new();

        for layer_index in 0..MAX_LAYERS {
            // Calculate row index for this layer
            let row_index = params.row_index(map_index, layer_index, log_value);
            // Calculate max row length for this layer
            let max_row_length = params.max_row_length(layer_index) as usize;

            // Fetch the row
            let global_row_index = params.global_row_index(map_index, row_index);
            let row = self.get_filter_map_row(global_row_index)?;
            if row.is_none() {
                break;
            }

            let row = row.unwrap();
            let row_len = row.columns.len();

            // Add the row to the result
            rows.push(row);

            // If this row isn't full, we're done with layers for this map.
            if row_len < max_row_length {
                break;
            }
        }

        Ok(rows)
    }
}

/// Provider trait for writing filter map data.
pub trait FilterMapsWriter: Send + Sync {
    /// Store filter map metadata.
    fn store_metadata(&self, metadata: FilterMapMetadata) -> FilterResult<()>;

    /// Store filter map rows.
    fn store_filter_map_row(
        &self,
        map_row_index: MapRowIndex,
        row: FilterMapRow,
    ) -> FilterResult<()>;

    /// Store block to log value pointer mapping.
    fn store_log_value_index_for_block(
        &self,
        block: BlockNumber,
        log_value_index: u64,
    ) -> FilterResult<()>;
}
