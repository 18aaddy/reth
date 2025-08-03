use crate::storage::InMemoryFilterMapsProvider;
use alloy_primitives::BlockNumber;
use reth_ethereum_primitives::Receipt;
use reth_log_index::{
    extract_log_values_from_block, FilterMapAccumulator, FilterMapParams, FilterMapsReader,
    FilterMapsWriter, FilterResult,
};
use reth_provider::test_utils::MockEthProvider;
use reth_provider::{BlockReader, ReceiptProvider};
use std::ops::RangeInclusive;
use std::sync::Arc;
use tracing::info;

fn persist(
    accumulator: &mut FilterMapAccumulator,
    storage: &InMemoryFilterMapsProvider,
) -> FilterResult<()> {
    for completed_map in accumulator.drain_completed_maps() {
        // store filter map rows

        for (map_row_index, row) in completed_map.rows {
            storage.store_filter_map_row(map_row_index, row)?;
        }

        // store block log value indices
        for (block_number, log_value_index) in completed_map.block_log_value_indices {
            storage.store_log_value_index_for_block(block_number, log_value_index)?;
        }
    }
    info!("last_indexed_block: {:?}", accumulator.metadata.last_indexed_block);
    info!("last_map_index: {:?}", accumulator.metadata.last_map_index);

    // store metadata
    let mut metadata = storage.get_metadata()?.unwrap_or_default();
    metadata.last_indexed_block = accumulator.metadata.last_indexed_block;
    metadata.last_map_index = accumulator.metadata.last_map_index;
    metadata.next_log_value_index = accumulator.log_value_index + 1;

    storage.store_metadata(metadata).unwrap();

    Ok(())
}

// this will be the execute() in the index logs stage
pub(crate) async fn index(
    provider: Arc<MockEthProvider>,
    range: RangeInclusive<BlockNumber>,
    storage: Arc<InMemoryFilterMapsProvider>,
) -> FilterResult<()> {
    let params = FilterMapParams::default();

    // Get starting position from storage
    let metadata = storage.get_metadata()?.unwrap_or_default();
    let mut accumulator = FilterMapAccumulator::new(params.clone(), metadata);

    let blocks = provider.block_range(range.clone()).unwrap_or_default();
    let receipts: Vec<Vec<Receipt>> =
        provider.receipts_by_block_range(range.clone()).unwrap_or_default();

    // Process all blocks to extract log values
    blocks
        .into_iter()
        .zip(receipts)
        .map(|(block, receipts)| extract_log_values_from_block(block, receipts))
        .for_each(|(block_delimiter, log_values)| {
            let _ = accumulator
                .add_block(block_delimiter, log_values)
                .map_err(|e| eprintln!("Error processing block: {:?}", e));
        });

    // write filter maps and block log value indices to storage
    let _ = persist(&mut accumulator, &storage).map_err(|e| eprintln!("Error persisting: {:?}", e));

    Ok(())
}
