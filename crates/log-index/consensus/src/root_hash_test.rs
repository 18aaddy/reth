use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use alloy_consensus::BlockHeader;
use alloy_primitives::{BlockNumber, B256};
use reth_ethereum_primitives::Receipt;
use reth_log_index::{
    address_value, extract_log_values_from_block, topic_value
};
use reth_provider::{test_utils::MockEthProvider, BlockReader, ReceiptProvider};
use schnellru::{ByLength, LruMap};
use std::ops::RangeInclusive;

use crate::utils::create_test_provider_with_random_blocks_and_receipts;
use tracing::info;

const START_BLOCK: BlockNumber = 0;
const BLOCKS_COUNT: usize = 500;
const TX_COUNT: u8 = 150;
const LOG_COUNT: u8 = 1;
const MAX_TOPICS: usize = 4;

// fn test_works(provider: Arc<MockEthProvider>, range: RangeInclusive<BlockNumber>) -> B256{
//     let mut hasher = LogIndexHasher::new();
//     let blocks = provider.block_range(range.clone()).expect("Invalid block range");
//     let receipts: Vec<Vec<Receipt>> =
//         provider.receipts_by_block_range(range.clone()).expect("Invalid block range");

//     let mut log_tree_root: B256 = B256::ZERO;
//     blocks
//         .into_iter()
//         .zip(receipts)
//         .for_each(|(block, receipt)| {
//             let header = &block.header.clone();

//             println!("Block number: {}, f: test_works(), l: 31", header.number);

//             let (_block_delimeter, log_values) = extract_log_values_from_block(block,
// receipt.clone());             // TODO: Fix B256::ZERO
//             hasher.add_header(header, header.state_root);
//             log_tree_root = hasher.add_receipts(header.parent_hash, header.state_root, receipt,
// log_values);         });
//     log_tree_root
// }

fn test_works(provider: Arc<MockEthProvider>, range: RangeInclusive<BlockNumber>) -> B256 {
    let mt = MemTree::default();
    let mut tree = MemTreeView::new_writer(Arc::new(Mutex::new(mt)), 0, B256::ZERO, B256::ZERO);
    let mut h = Hasher {
        tree: Box::new(tree.clone()),
        params: &mut DEFAULT_PARAMS,
        row_mapping_cache: LruMap::new(ByLength::new(CACHED_ROW_MAPPINGS)),
    };
    h.params.derive_fields();
    h.init_genesis();

    let blocks = provider.block_range(range.clone()).expect("Invalid block range");
    let receipts_vec: Vec<Vec<Receipt>> =
        provider.receipts_by_block_range(range.clone()).expect("Invalid block range");

    blocks.into_iter().zip(receipts_vec).for_each(|(block, receipts)| {
        let header = &block.header.clone();

        println!("Block number: {}, f: test_works(), l: 31", header.number);

        let transactions = block.body.transactions();

        for (tx_index, (receipt, transaction)) in receipts.iter().zip(transactions).enumerate() {
            let transaction_hash = *transaction.tx_hash();
            for (log_index, log) in receipt.logs.iter().enumerate() {
                // Address value
                let address_value = address_value(&log.address);
                let log_value = LogValue {
                    value: address_value,
                    transaction_hash,
                    block_number: header.number,
                    transaction_index: tx_index as u64,
                    log_in_tx_index: log_index as u64,
                };

                h.add_log_event(&log, &log_value);
            }
        }

        if header.number < *range.end() {
            h.add_block_delimiter(header);
        }
    });

    tree.root_hash()
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
