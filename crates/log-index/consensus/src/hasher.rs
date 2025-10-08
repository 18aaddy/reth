use crate::{mem_tree::MemTreeView, proof::TreeReader};
use alloy_consensus::Header;
use alloy_primitives::{Log, B256};
use reth_log_index::utils::{address_value, topic_value};
use reth_log_index_common::{FilterMapParams, TreeIndex, TreeNode, GTI_DELIMITER_META_BLOCK_HASH, GTI_DELIMITER_META_BLOCK_NUMBER, GTI_DELIMITER_META_DUMMY, GTI_DELIMITER_META_TIMESTAMP, GTI_DELIMITER_ZERO, GTI_EPOCHS, GTI_FILTER_MAPS, GTI_LOG_ADDRESS, GTI_LOG_DATA, GTI_LOG_ENTRIES, GTI_LOG_META_BLOCK_NUMBER, GTI_LOG_META_LOG_INDEX, GTI_LOG_META_TX_HASH, GTI_LOG_META_TX_INDEX, GTI_LOG_TOPICS_LENGTH, GTI_LOG_TOPICS_ROOT, GTI_LOG_ZERO, GTI_NEXT_INDEX, GTI_PROG_LIST_COUNT, GTI_PROG_LIST_NEXT_TREE, GTI_PROG_LIST_SUBTREE, GTI_PROG_LIST_TREE, ZERO_HASHES};

use schnellru::LruMap;


// TODO: Is LogValue needed or something else can be used?
type BlockNumber = u64;

/// Metadata for an actual log value (address or topic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogValue {
    /// The value hash (address or topic).
    pub value: B256,
    /// The transaction hash.
    pub transaction_hash: B256,
    /// The block number.
    pub block_number: BlockNumber,
    /// The transaction index in the block.
    pub transaction_index: u64,
    /// The log index within the transaction.
    pub log_in_tx_index: u64,
}

#[derive(Debug, Clone, Copy)]
///
pub struct LVPosition {
    row_index: u32,
    layer_index: u32,
}

trait LogIndexData: TreeReader {
    fn set(&self, tree_index: TreeIndex, node: TreeNode);
    fn finalize(&self, tree_index: TreeIndex);
}

#[derive(Debug)]
pub struct ProgListIndex<'a> {
    params: &'a FilterMapParams,
    list_root: TreeIndex,
    count_index: TreeIndex,
    tree_root: TreeIndex,
    subtree_height: u64,
    subtree_first: u64,
}

impl<'a> ProgListIndex<'a> {
    fn init(&mut self, params: &'a FilterMapParams, root: TreeIndex) {
        self.params = params;
        self.list_root = root;
        self.count_index = root.child(GTI_PROG_LIST_COUNT);
        self.tree_root = root.child(GTI_PROG_LIST_TREE);
        // TODO: self.subtree_height = params.progListHeightFirst
        self.subtree_height = 0;
        self.subtree_first = 0;
    }

    fn get_leaf(&mut self, list_index: u64) -> (TreeIndex, TreeIndex, u64, u64) {
        if list_index < self.subtree_first {
            self.init(self.params, self.list_root);
        }

        while self.subtree_first + (1 << self.subtree_height) <= list_index {
            self.subtree_first += 1 << self.subtree_height;
            // TODO: self.subtree_height += self.params.progListHeightStep
            self.subtree_height += 0;
            self.tree_root = self.tree_root.child(GTI_PROG_LIST_NEXT_TREE);
        }

        let subtree_index = list_index - self.subtree_first;
        (
            self.tree_root.child(GTI_PROG_LIST_SUBTREE).append(subtree_index, self.subtree_height),
            self.tree_root,
            subtree_index,
            self.subtree_height,
        )
    }
}

///
#[derive(Debug)]
pub struct Hasher<'a> {
    pub tree: Box<MemTreeView>, //Box<dyn LogIndexData>,
    pub params: &'a mut FilterMapParams,
    pub row_mapping_cache: LruMap<B256, LVPosition>,
}

impl<'a> Hasher<'a> {
    // TODO: Does this need Log and LogValue separately???
    pub fn add_log_event(&mut self, log: &Log, log_value: &LogValue) -> (u64, u64) {
        let add_count = log.topics().len() as u64 + 1;
        let lv_index = self.add_values(add_count);
        self.render_log(lv_index, log, log_value);

        for lvi in lv_index + 1..lv_index + add_count {
            self.tree.set(self.params.gti_log_entry_root(lvi), TreeNode::default());
        }
        self.add_to_map(lv_index, address_value(&log.address));
        for i in 0..log.topics().len() {
            self.add_to_map(lv_index + i as u64 + 1, topic_value(&log.topics()[i]));
        }
        self.advance(lv_index, add_count);
        (lv_index, lv_index + add_count)
    }

    pub fn add_block_delimiter(&mut self, header: &Header) -> (u64, u64) {
        let lv_index = self.add_values(1);
        self.render_block_delimiter(lv_index, header);
        self.advance(lv_index, 1);
        (lv_index, lv_index + 1)
    }

    fn add_values(&mut self, add_count: u64) -> u64 {
        let mut next_index = node_to_u64(self.tree.get(GTI_NEXT_INDEX));
        let left_from_map =
            self.params.values_per_map() - next_index % self.params.values_per_map();
        if left_from_map < add_count {
            next_index += left_from_map;
        }
        if next_index % self.params.values_per_map() == 0 {
            self.add_new_map((next_index / self.params.values_per_map()) as u32);
        }
        next_index
    }

    fn advance(&mut self, start_index: u64, add_count: u64) {
        let values_per_epoch = self.params.maps_per_epoch() as u64 * self.params.values_per_map();
        let log_entries_root = self
            .params
            .gti_epoch_root((start_index / values_per_epoch) as u32)
            .child(GTI_LOG_ENTRIES);
        let first_sub_index = start_index % values_per_epoch;
        let last_sub_index = (start_index + add_count - 1) % values_per_epoch;
        self.expand_vector(
            log_entries_root,
            first_sub_index,
            last_sub_index,
            (self.params.log_maps_per_epoch + self.params.log_values_per_map).into(),
        );
        self.finalize_vector(
            log_entries_root,
            first_sub_index,
            last_sub_index,
            (self.params.log_maps_per_epoch + self.params.log_values_per_map).into(),
        );
        self.tree.set(GTI_NEXT_INDEX, u64_to_node(start_index + add_count));
    }

    fn render_block_delimiter(&mut self, lv_index: u64, header: &Header) {
        let log_entry_root = self.params.gti_log_entry_root(lv_index);
        self.tree.set(log_entry_root.child(GTI_DELIMITER_ZERO), TreeNode::default());
        self.tree
            .set(log_entry_root.child(GTI_DELIMITER_META_BLOCK_NUMBER), u64_to_node(header.number));

        let mut block_hash = TreeNode::default();
        block_hash.copy_from_slice(&header.hash_slow().0);
        self.tree.set(log_entry_root.child(GTI_DELIMITER_META_BLOCK_HASH), block_hash);

        self.tree
            .set(log_entry_root.child(GTI_DELIMITER_META_TIMESTAMP), u64_to_node(header.timestamp));
        self.tree.set(log_entry_root.child(GTI_DELIMITER_META_DUMMY), u64_to_node(u64::MAX));
    }

    fn render_log(&mut self, lv_index: u64, log: &Log, log_value: &LogValue) {
        let log_entry_root = self.params.gti_log_entry_root(lv_index);

        // Set address
        let mut addr = TreeNode::default();
        addr[..log.address.0.len()].copy_from_slice(&log.address.0.as_slice());
        self.tree.set(log_entry_root.child(GTI_LOG_ADDRESS), addr);

        // Set topics length
        self.tree.set(
            log_entry_root.child(GTI_LOG_TOPICS_LENGTH),
            u64_to_node(log.topics().len() as u64),
        );

        // Set each topic
        for i in 0..4 {
            let mut node = TreeNode::default();
            if i < log.topics().len() {
                node.copy_from_slice(&log.topics()[i].0);
            }
            self.tree.set(log_entry_root.child(GTI_LOG_TOPICS_ROOT).append(i as u64, 2), node);
        }

        // Handle log data
        let data_len = log.data.data.len() as u64;
        let mut pl: ProgListIndex<'_> = ProgListIndex {
            params: &FilterMapParams::default(),
            list_root: TreeIndex::default(),
            count_index: TreeIndex::default(),
            tree_root: TreeIndex::default(),
            subtree_height: 0,
            subtree_first: 0,
        };
        pl.init(self.params, log_entry_root.child(GTI_LOG_DATA));
        self.tree.set(pl.count_index, u64_to_node(data_len));

        let mut chunk_index = 0;
        let mut ptr = 0;
        while ptr < data_len {
            let (leaf_index, _, _, _) = pl.get_leaf(chunk_index);
            let mut node = TreeNode::default();
            let end = std::cmp::min(ptr + 32, data_len);
            node[..(end - ptr) as usize]
                .copy_from_slice(&log.data.data[(ptr as usize)..(end as usize)]);
            ptr = end;
            self.tree.set(leaf_index, node);
            chunk_index += 1;
        }

        if chunk_index == 0 {
            self.tree.set(pl.tree_root, TreeNode::default());
        } else {
            let (_, tree_root, subtree_index, subtree_height) = pl.get_leaf(chunk_index - 1);
            self.tree.set(tree_root.child(GTI_PROG_LIST_NEXT_TREE), TreeNode::default());
            let subtree_root = tree_root.child(GTI_PROG_LIST_SUBTREE);
            self.expand_vector(subtree_root, 0, subtree_index, subtree_height);
        }

        // Set log metadata
        self.tree.set(log_entry_root.child(GTI_LOG_ZERO), TreeNode::default());
        self.tree.set(
            log_entry_root.child(GTI_LOG_META_BLOCK_NUMBER),
            u64_to_node(log_value.block_number),
        );

        let mut tx_hash = TreeNode::default();
        tx_hash.copy_from_slice(&log_value.transaction_hash.0);
        self.tree.set(log_entry_root.child(GTI_LOG_META_TX_HASH), tx_hash);

        self.tree.set(
            log_entry_root.child(GTI_LOG_META_TX_INDEX),
            u64_to_node(log_value.transaction_index),
        );
        self.tree.set(
            log_entry_root.child(GTI_LOG_META_LOG_INDEX),
            // TODO: Log index in the block needed
            u64_to_node(log_value.log_in_tx_index),
        );
    }

    pub fn init_genesis(&mut self) {
        self.tree.set(GTI_EPOCHS, ZERO_HASHES[self.params.log_epoch_history as usize]);
        self.tree.set(GTI_NEXT_INDEX, u64_to_node(0));
    }

    pub fn init_with_proof(&self, _proof: &[u8]) {
        unimplemented!("init_with_proof not implemented");
    }

    pub fn make_init_proof(&self) -> Vec<u8> {
        unimplemented!("make_init_proof not implemented");
    }

    fn add_new_epoch(&mut self, next_epoch: u32) {
        if next_epoch > 0 {
            self.finalize_vector(
                GTI_EPOCHS,
                (next_epoch - 1) as u64,
                (next_epoch - 1) as u64,
                self.params.log_epoch_history,
            );
        }

        let epoch_root_index = self.params.gti_epoch_root(next_epoch);
        for row_index in 0..self.params.map_height() {
            self.tree.set(
                epoch_root_index
                    .child(GTI_FILTER_MAPS)
                    .append(row_index as u64, self.params.log_map_height as u64),
                ZERO_HASHES[self.params.log_maps_per_epoch as usize],
            );
        }

        self.tree.set(
            epoch_root_index.child(GTI_LOG_ENTRIES),
            ZERO_HASHES[(self.params.log_maps_per_epoch + self.params.log_values_per_map) as usize],
        ); // Zero hash
        self.expand_vector(
            GTI_EPOCHS,
            next_epoch as u64,
            next_epoch as u64,
            self.params.log_epoch_history,
        );
    }

    fn add_new_map(&mut self, next_map: u32) {
        let epoch = next_map >> self.params.log_maps_per_epoch;
        let map_sub_index = next_map % self.params.maps_per_epoch() as u32;

        if map_sub_index == 0 {
            self.add_new_epoch(epoch);
        }

        let filter_maps_root_index = self.params.gti_epoch_root(epoch).child(GTI_FILTER_MAPS);
        for row_index in 0..self.params.map_height() {
            let epoch_row_root_index =
                filter_maps_root_index.append(row_index as u64, self.params.log_map_height.into());

            if map_sub_index > 0 {
                self.finalize_vector(
                    epoch_row_root_index,
                    (map_sub_index - 1) as u64,
                    (map_sub_index - 1) as u64,
                    self.params.log_maps_per_epoch.into(),
                );
            }

            let map_row_root_index =
                epoch_row_root_index.append(map_sub_index as u64, self.params.log_maps_per_epoch.into());
            self.tree.set(map_row_root_index.child(GTI_PROG_LIST_TREE), TreeNode::default());
            self.tree.set(map_row_root_index.child(GTI_PROG_LIST_COUNT), TreeNode::default());
            self.expand_vector(
                epoch_row_root_index,
                map_sub_index as u64,
                map_sub_index as u64,
                self.params.log_maps_per_epoch.into(),
            );
        }

        if self.row_mapping_cache.is_empty() {
            // Cannot directly purge the cache in Rust's LruMap, but we can re-create it if needed
            // In practice, you might want to implement a method to clear the cache
        }
    }

    fn add_to_map(&mut self, lv_index: u64, log_value: B256) {
        let map_index = (lv_index >> self.params.log_values_per_map) as u32;

        let mut lvp = if let Some(cached_lvp) = self.row_mapping_cache.get(&log_value).cloned() {
            cached_lvp
        } else {
            LVPosition {
                row_index: self.params.row_index(map_index, 0, &log_value) as u32,
                layer_index: 0,
            }
        };

        let column_index = self.params.column_index(lv_index, &log_value);
        while !self.add_to_row(
            map_index,
            lvp.row_index,
            column_index.into(),
            self.params.max_row_length(lvp.layer_index).into(),
        ) {
            lvp.layer_index += 1;
            lvp.row_index =
                self.params.row_index(map_index.into(), lvp.layer_index.into(), &log_value) as u32;
        }

        // Update cache - in a real implementation, you would need to handle this safely
        // This is a simplified version assuming row_mapping_cache can be mutated
    }

    fn add_to_row(&mut self, map_index: u32, row_index: u32, entry: u64, max_len: u64) -> bool {
        let epoch = map_index >> self.params.log_maps_per_epoch;
        let map_sub_index = map_index % self.params.maps_per_epoch() as u32;

        let filter_maps_root_index = self.params.gti_epoch_root(epoch).child(GTI_FILTER_MAPS);
        let epoch_row_root_index =
            filter_maps_root_index.append(row_index as u64, self.params.log_map_height.into());
        let map_row_root_index =
            epoch_row_root_index.append(map_sub_index as u64, self.params.log_maps_per_epoch.into());

        let mut pl = ProgListIndex {
            params: self.params,
            list_root: TreeIndex::default(),
            count_index: TreeIndex::default(),
            tree_root: TreeIndex::default(),
            subtree_height: 0,
            subtree_first: 0,
        };
        pl.init(self.params, map_row_root_index);

        let next_entry = node_to_u64(self.tree.get(pl.count_index));
        if next_entry >= max_len as u64 {
            return false;
        }

        self.tree.set(pl.count_index, u64_to_node(next_entry + 1));

        let list_index = next_entry / 8;
        let list_sub_index = next_entry % 8;

        let (leaf_index, tree_root, subtree_index, subtree_height) = pl.get_leaf(list_index);

        if subtree_index == 0 && list_sub_index == 0 {
            self.tree.set(tree_root.child(GTI_PROG_LIST_NEXT_TREE), TreeNode::default());
        }

        let subtree_root = tree_root.child(GTI_PROG_LIST_SUBTREE);

        let mut leaf = if list_sub_index == 0 {
            self.expand_vector(subtree_root, subtree_index, subtree_index, subtree_height);
            TreeNode::default()
        } else {
            self.tree.get(leaf_index)
        };

        // Set entry in the leaf node (similar to binary.LittleEndian.PutUint32)
        let start_idx = (list_sub_index * 4) as usize;
        leaf[start_idx..start_idx + 4].copy_from_slice(&entry.to_le_bytes());

        self.tree.set(leaf_index, leaf);

        true
    }

    fn finalize_vector(
        &self,
        _vector_root: TreeIndex,
        first_index: u64,
        last_index: u64,
        height: u64,
    ) {
        for shift in 0..=height {
            let after_last_shift = (last_index + 1) >> shift;
            if first_index >> shift == after_last_shift {
                break; // No subtree's last item is included on this level or below
            }

            if after_last_shift & 1 == 1 {
                // Left subtree's last item included, finalize
                // TODO: self.tree.finalize(vector_root.append(after_last_shift - 1, height -
                // shift));
            }
        }
    }

    fn expand_vector(
        &mut self,
        vector_root: TreeIndex,
        first_index: u64,
        last_index: u64,
        height: u64,
    ) {
        for shift in 0..height {
            let last_shift = last_index >> shift;
            if first_index > 0 && ((first_index - 1) >> shift == last_shift) {
                break; // No subtree's first item is included on this level or below
            }

            if last_shift & 1 == 0 {
                // Left subtree's first item included, initialize right sibling
                self.tree.set(
                    vector_root.append(last_shift + 1, height - shift),
                    ZERO_HASHES[shift as usize],
                );
            }
        }
    }
}

pub(crate) fn node_to_u64(node: TreeNode) -> u64 {
    u64::from_le_bytes(node[0..8].try_into().unwrap())
}

pub(crate) fn u64_to_node(value: u64) -> TreeNode {
    let mut node = [0u8; 32];
    node[..8].copy_from_slice(&value.to_le_bytes());
    node
}
