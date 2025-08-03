//! Integration tests that creates a MockEthProvider, adds some blocks, and then
//! creates a filter map and verifies that the filter map can be used to query the
//! blocks and verify that the filter map is correct.

use alloy_primitives::{BlockNumber, Log, B256};
use alloy_rpc_types_eth::Filter;
use rand::seq::SliceRandom;
use reth_provider::{test_utils::MockEthProvider, ReceiptProvider};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, trace};

use crate::filter::get_logs_in_block_range;
use crate::indexer::index;
use crate::storage::InMemoryFilterMapsProvider;
use crate::utils::create_test_provider_with_random_blocks_and_receipts;

const START_BLOCK: BlockNumber = 0;
const BLOCKS_COUNT: usize = 500;
const TX_COUNT: u8 = 150;
const LOG_COUNT: u8 = 1;
const MAX_TOPICS: usize = 4;

#[tokio::test]
async fn test_filter_map() {
    reth_tracing::init_test_tracing();

    let start = Instant::now();
    info!("creating provider with {} blocks, and {} txs per block", BLOCKS_COUNT, TX_COUNT);
    let provider: MockEthProvider = create_test_provider_with_random_blocks_and_receipts(
        START_BLOCK,
        BLOCKS_COUNT,
        TX_COUNT,
        LOG_COUNT,
        MAX_TOPICS,
    )
    .await;
    info!("provider created in {:?}s", start.elapsed().as_secs_f64());

    let provider = Arc::new(provider);

    let storage = InMemoryFilterMapsProvider::new(provider.clone());

    let storage = Arc::new(storage);

    let range = START_BLOCK..=START_BLOCK + BLOCKS_COUNT as u64 - 1;

    let start = Instant::now();
    info!("indexing logs");
    let result = index(provider.clone(), range.clone(), storage.clone()).await;
    info!("Indexed all logs in {:?}s", start.elapsed().as_secs_f64());

    assert!(result.is_ok());

    // get all the logs from the provider
    let receipts = provider.receipts_by_block_range(range.clone()).unwrap_or_default();
    let logs: Vec<Log> = receipts
        .iter()
        .flat_map(|receipt| receipt.iter().flat_map(|r| r.logs.iter()))
        .cloned()
        .collect();

    info!("Number of logs indexed: {}", logs.len());

    // shuffle the logs
    // let mut rng = rand::rng();
    // let mut logs = logs.clone();
    // logs.shuffle(&mut rng);

    // find all the logs in the filter map
    for (i, log) in logs.iter().enumerate() {
        let address = log.address.clone();

        let topics: Vec<B256> = log.topics().iter().map(|&topic| topic).collect();

        let mut filter = Filter::new().address(address);

        for (i, topic) in topics.iter().enumerate() {
            filter = match i {
                0 => filter.event_signature(*topic),
                1 => filter.topic1(*topic),
                2 => filter.topic2(*topic),
                3 => filter.topic3(*topic),
                _ => filter,
            };
        }

        // fetch the same log with and without indexing.
        // benchmark the difference.
        let start = Instant::now();
        let logs_with_indexing = get_logs_in_block_range(
            storage.clone(),
            filter.clone(),
            *range.clone().start(),
            *range.clone().end(),
            false,
        )
        .await;
        let end = Instant::now();
        let indexed_time = end.duration_since(start);

        let start = Instant::now();
        let logs_with_bloom = get_logs_in_block_range(
            storage.clone(),
            filter,
            *range.clone().start(),
            *range.clone().end(),
            true,
        )
        .await;
        let end = Instant::now();
        let bloom_time = end.duration_since(start);

        trace!(
            "bloom_time/indexed_time: {:?}",
            bloom_time.as_secs_f64() / indexed_time.as_secs_f64()
        );
        trace!("bloom_time: {:?}, indexed_time: {:?}", bloom_time, indexed_time);

        assert!(logs_with_bloom.is_ok());

        assert!(logs_with_indexing.is_ok());
        assert!(logs_with_bloom.is_ok());
        let logs = logs_with_indexing.unwrap();

        assert!(
            !logs.is_empty(),
            "Should find at least one matching log, expected log address: {:?}, topics: {:?}, iteration: {:?}",
            log.address,
            topics.len(),
            i
        );

        for l in logs.iter() {
            assert_eq!(l.address, log.address, "log address mismatch");
            assert_eq!(l.topics(), log.topics(), "log topics mismatch");
            assert_eq!(l.data, log.data, "log data mismatch");
        }
    }
}
