use std::{sync::Arc, time::Instant};

use alloy_primitives::{BlockNumber, B256};
use reth_ethereum_primitives::Receipt;
use reth_log_index::{extract_log_values_from_block, LogIndexHasher};
use reth_provider::{test_utils::MockEthProvider, BlockReader, ReceiptProvider};
use std::ops::RangeInclusive;

use crate::utils::create_test_provider_with_random_blocks_and_receipts;
use tracing::info;

const START_BLOCK: BlockNumber = 0;
const BLOCKS_COUNT: usize = 500;
const TX_COUNT: u8 = 150;
const LOG_COUNT: u8 = 1;
const MAX_TOPICS: usize = 4;

fn test_works(provider: Arc<MockEthProvider>, range: RangeInclusive<BlockNumber>) -> B256{
    let mut hasher = LogIndexHasher::new();
    let blocks = provider.block_range(range.clone()).expect("Invalid block range");
    let receipts: Vec<Vec<Receipt>> =
        provider.receipts_by_block_range(range.clone()).expect("Invalid block range");

    let mut log_tree_root: B256 = B256::ZERO;
    blocks
        .into_iter()
        .zip(receipts)
        .for_each(|(block, receipt)| {
            let header = &block.header.clone();
            let (_block_delimeter, log_values) = extract_log_values_from_block(block, receipt.clone());
            // TODO: Fix B256::ZERO
            hasher.add_header(header, B256::ZERO);
            log_tree_root = hasher.add_receipts(B256::ZERO, B256::ZERO, receipt, log_values);
        });
    log_tree_root
}

#[tokio::test]
async fn test_log_index_root() {
    reth_tracing::init_test_tracing();

    let start = Instant::now();
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
    let range = START_BLOCK..=START_BLOCK + BLOCKS_COUNT as u64 - 1;
    let root_hash = test_works(provider, range);

    println!("Log Tree Index Root Hash: {:?}", root_hash);
}