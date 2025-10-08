use std::sync::{Arc, Mutex};

use alloy_consensus::Header;
use alloy_primitives::B256;
use reth_ethereum_primitives::Receipt;
use schnellru::{ByLength, LruMap};

use crate::{
    hasher::Hasher,
    mem_tree::{MemTree, MemTreeView},
};
use reth_log_index_common::{TreeIndex, DEFAULT_PARAMS};

const CACHED_ROW_MAPPINGS: u32 = 10_000;

#[derive(Debug)]
pub struct LogIndexHasher {
    header_cache: LruMap<B256, Header>,
    id_cache: LruMap<B256, B256>,
    mem_tree: Arc<Mutex<MemTree>>,
    block_ptrs: Arc<Mutex<Vec<u64>>>,
    hasher: Hasher<'static>,
}

impl LogIndexHasher {
    // ? Resolve the new() function
    pub fn new() -> Self {
        let mem_tree = MemTree::default();
        let row_mapping_cache = LruMap::new(ByLength::new(CACHED_ROW_MAPPINGS));
        // Use Box::leak to extend the lifetime of DEFAULT_PARAMS for Hasher
        let params: &'static mut _ = Box::leak(Box::new(DEFAULT_PARAMS));
        let tree = Box::new(MemTreeView {
            tree: Arc::new(Mutex::new(MemTree::default())),
            block_number: 0,
            last_shift_index: TreeIndex::default(),
            last_height: 0,
            last_node_pos: [0; 128],
        });
        let hasher = Hasher { tree, params, row_mapping_cache };
        hasher.params.derive_fields();
        LogIndexHasher {
            header_cache: LruMap::new(ByLength::new(100)),
            id_cache: LruMap::new(ByLength::new(100)),
            mem_tree: Arc::new(Mutex::new(mem_tree)),
            block_ptrs: Arc::new(Mutex::new(Vec::<u64>::new())),
            hasher,
        }
    }

    pub fn add_header(&mut self, header: &Header, block_id: B256) {
        let hash = header.hash_slow();
        self.header_cache.insert(hash, header.clone());
        self.id_cache.insert(hash, block_id);
    }

    ///
    pub fn add_receipts(
        &mut self,
        parent_hash: B256,
        block_id: B256,
        receipts: Vec<Receipt>,
        log_values: Vec<LogValue>,
    ) -> B256 {
        // Initializing values
        let mut block_number: u64 = 0;
        let mut parent_header = &Some(Header::default());
        let mut parent_id = B256::ZERO;

        let parent_header_value = self.header_cache.get(&parent_hash).cloned();
        if !parent_hash.is_zero() {
            parent_header = &parent_header_value;
            block_number = match parent_header {
                Some(p) => p.number + 1,
                None => 1,
            };
            parent_id = *self.id_cache.get(&parent_hash).unwrap();
        }
        println!("Block number: {} f: add_receipts(), l: 97", block_number);
        let tree = Box::new(MemTreeView::new_writer(
            self.mem_tree.clone(),
            block_number,
            parent_id,
            block_id,
        ));
        self.hasher.tree = tree;
        match parent_header {
            Some(p) => {
                self.hasher.add_block_delimiter(&p);
            }
            None => self.hasher.init_genesis(),
        }

        // TODO: Find better method to get LogValue
        let mut i = 1;
        for receipt in receipts {
            for log in &receipt.logs {
                let (_, next_ptr) = self.hasher.add_log_event(log, &log_values[i]);
                let arc_block_ptrs = Arc::clone(&self.block_ptrs);
                let mut block_ptrs = arc_block_ptrs.lock().unwrap();
                if block_ptrs.len() < block_number as usize {
                    panic!("invalid block number");
                }
                block_ptrs.splice(block_number as usize..block_number as usize, [next_ptr]);
                i += 1;
            }
        }

        self.hasher.tree.root_hash()
    }
}
