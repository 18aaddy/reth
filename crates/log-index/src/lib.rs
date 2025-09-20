//! Log index implementation for efficient log indexing.
//!
//! This crate provides the core implementation of log index,
//! which enables efficient querying of Ethereum logs without relying on bloom filters.
//!
//! ## Overview
//!
//! Log index is a two-dimensional sparse bit maps that index log values (addresses and topics)
//! for efficient searching. It provides:
//!
//! - Constant-time lookups for log queries
//! - Better storage efficiency compared to bloom filters
//! - Support for range queries across blocks
//! - Multi-layer overflow handling for collision management
//!
//! ## Usage
//!
//! ```rust
//! use reth_log_index::{LogIndexAccumulator, LogIndexParams, extract_log_values_from_block};
//! use reth_log_index::storage::{LogIndexReader, LogIndexWriter};
//!
//! // Create an accumulator
//! let params = LogIndexParams::default();
//! let mut accumulator = LogIndexAccumulator::new(params, map_index);
//!
//! // Extract log values from block
//! let log_values = extract_log_values_from_block(
//!     log_value_index,
//!     block_number,
//!     block_hash,
//!     parent_hash,
//!     receipts
//! );
//!
//! // Process log values
//! for log_value in log_values {
//!     accumulator.add_log_value(log_value.index, log_value.value)?;
//! }
//! ```

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod accumulator;
mod constants;
mod params;
mod provider;
mod types;
mod utils;
mod hasher;
mod proof;
mod log_index_hasher;
mod mem_tree;

pub use accumulator::{FilterMap, FilterMapAccumulator};
pub use constants::{DEFAULT_PARAMS, EXPECTED_MATCHES, MAX_LAYERS, RANGE_TEST_PARAMS};
pub use params::FilterMapParams;
pub use provider::{FilterMapsReader, FilterMapsWriter};
pub use types::*;
pub use utils::{address_value, extract_log_values_from_block, topic_value};
// pub use hasher::get_root_hash;
pub use hasher::TreeIndex;
pub use proof::TreeReader;
pub use log_index_hasher::LogIndexHasher;