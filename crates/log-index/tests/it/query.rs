use alloy_primitives::Log;
use alloy_rpc_types_eth::BlockHashOrNumber;
use reth_log_index::{FilterMapsReader, FilterResult};
use reth_provider::ReceiptProvider;
use std::sync::Arc;

use crate::storage::InMemoryFilterMapsProvider;

pub fn get_log_at_index(
    provider: Arc<InMemoryFilterMapsProvider>,
    log_index: u64,
) -> FilterResult<Option<Log>> {
    let block_log_value_index = provider.find_block_for_log_value_index(log_index)?;
    if block_log_value_index.is_none() {
        return Ok(None); // No block contains this log index
    }
    let (block_number, block_start_log_value_index) = block_log_value_index.unwrap();

    // Get receipts for the block
    let receipts = provider
        .provider
        .receipts_by_block(BlockHashOrNumber::Number(block_number))?
        .unwrap_or_default();

    let mut current_log_value_index = block_start_log_value_index + 1;

    // Iterate through all logs in the block to find the one at log_index
    for receipt in receipts {
        for log in receipt.logs {
            // Each log occupies multiple log value indices:
            // - 1 for the address
            // - 1 for each topic
            let log_value_count = 1 + log.topics().len() as u64;

            // check if this is the log we are looking for
            if log_index == current_log_value_index {
                return Ok(Some(log));
            } else if log_index < current_log_value_index {
                println!("we walked past the log! this shouldnt happen log_index: {:?}, current_log_value_index: {:?}", log_index, current_log_value_index);
                return Ok(None);
            }

            // increment the current log value index by the number of log values in this log and check the next log
            current_log_value_index += log_value_count;
        }
    }
    println!("log index not found in this block: {:?}", log_index);

    // Log index not found in this block (shouldn't happen if index is correct)
    Ok(None)
}
