use std::{
    cmp::min,
    sync::{Arc, Mutex, MutexGuard},
};

use alloy_primitives::{map::HashMap, B256};
use reth_log_index_common::{TreeIndex, TreeNode, ROOT_INDEX};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub struct MemTreeRoot {
    node_index: u32,
    block_id: B256,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct MemTreeNode {
    node: TreeNode,
    left: u32,
    right: u32,
}

impl MemTreeNode {
    fn left_child(&self) -> u32 {
        self.left & ((1 << 31) - 1)
    }

    fn right_child(&self) -> u32 {
        self.right & ((1 << 31) - 1)
    }

    fn is_leaf(&self) -> bool {
        self.left_child() == ((1 << 31) - 1)
    }

    fn is_known(&self) -> bool {
        self.left & (1 << 31) != 0
    }

    fn is_finalized(&self) -> bool {
        self.right & (1 << 31) != 0
    }

    fn set_children(&mut self, left: u32, right: u32) {
        (self.left, self.right) =
            ((self.left & (1 << 31)) + left, (self.right & (1 << 31)) + right);
    }

    fn set_leaf(&mut self) {
        self.set_children((1 << 31) - 1, (1 << 31) - 1);
    }

    fn set_known(&mut self, b: bool) {
        self.left &= (1 << 31) - 1;
        if b {
            self.left += 1 << 31;
        }
    }

    fn set_finalized(&mut self, b: bool) {
        self.right &= (1 << 31) - 1;
        if b {
            self.right += 1 << 31;
        }
    }
}

// TODO: Mutex and Arc
#[derive(Clone, Debug, Default)]
pub struct MemTree {
    pub nodes: Vec<MemTreeNode>,
    pub node_count: u32,
    pub blocks: Range<u64>,
    pub roots: HashMap<u64, MemTreeRoot>,
}

// impl Default for MemTree {
//     fn default() -> Self {
//         Self {
//             // ? 5?
//             nodes: vec![MemTreeNode { node: [0; 32], left: 0, right: 0 }],
//             node_count: 0,
//             blocks: Range::default(),
//             roots: HashMap::<u64, MemTreeRoot>::default(),
//         }
//     }
// }

#[derive(Clone, Copy, Default, Debug)]
pub struct Range<T> {
    pub first: T,
    pub after_last: T,
}

impl<T> Range<T>
where
    T: PartialOrd + Copy,
{
    pub fn set_first(&mut self, v: T) {
        self.first = v;
        if self.after_last < self.first {
            self.after_last = self.first;
        }
    }
}

impl MemTree {
    fn need_expand(&self) -> bool {
        self.nodes.len() < self.node_count as usize + 1000
    }

    fn expand(&mut self) {
        self.nodes.reserve(self.nodes.len() / 8 + 1000);
    }

    fn add_node(&mut self) -> u32 {
        let new_node = self.node_count;
        self.node_count += 1;
        new_node
    }

    fn hash_node(&mut self, index: TreeIndex, node_index: u32) {
        let node = self.nodes[node_index as usize];
        if node.is_known() {
            return;
        }
        if node.is_leaf() {
            println!("unknown {} {}", index.hi, index.lo);
            panic!("unknown leaf error occured during hashing");
        }
        self.hash_node(index.append(0, 1), node.left_child());
        self.hash_node(index.append(1, 1), node.right_child());
        let mut hasher = Sha256::new();
        hasher.update(&self.nodes[node.left_child() as usize].node);
        hasher.update(&self.nodes[node.right_child() as usize].node);
        let res = hasher.finalize();
        self.nodes[node_index as usize].node.copy_from_slice(&res);
        self.nodes[node_index as usize].set_known(true);
    }

    fn known_nodes(&self) -> u32 {
        let mut res = 0;
        for node in &self.nodes[0..self.node_count as usize] {
            if node.is_known() {
                res += 1;
            }
        }
        res
    }
}

impl MemTreeView {
    pub fn new_reader(tree: Arc<Mutex<MemTree>>, block_number: u64) -> Self {
        let tree_guard = tree.lock().unwrap();
        let root = match tree_guard.roots.get(&block_number) {
            Some(r) => *r,
            None => panic!("block number not found in memory tree"),
        };
        drop(tree_guard);

        Self {
            tree,
            block_number: 0,
            last_shift_index: TreeIndex::default(),
            last_height: 0,
            last_node_pos: {
                let mut arr = [0u32; 128];
                arr[0] = root.node_index;
                arr
            },
        }
    }

    pub fn new_writer(
        tree: Arc<Mutex<MemTree>>,
        block_number: u64,
        parent_id: B256,
        block_id: B256,
    ) -> Self {
        let mut tree_guard = tree.lock().unwrap();
        if tree_guard.need_expand() {
            tree_guard.expand();
        }
        let new_root = tree_guard.add_node();

        println!("Node count: {}, f: new_writer(), l: 191", tree_guard.node_count);

        if block_number > 0 {
            let parent_root = match tree_guard.roots.get(&(block_number - 1)) {
                Some(r) => r,
                None => panic!("parent block missing from memory tree"),
            };

            if parent_root.block_id != parent_id {
                panic!("parent block missing from memory tree");
            }
            let mem_tree_node =
                *tree_guard.nodes.get(parent_root.node_index as usize).expect("Invalid node index");
            tree_guard.nodes.insert(new_root as usize, mem_tree_node);
        } else {
            tree_guard.nodes.insert(
                new_root as usize,
                MemTreeNode { node: [0; 32], left: (1 << 31) - 1, right: (1 << 31) - 1 },
            );
        }
        let mv = MemTreeView {
            tree: Arc::clone(&tree),
            block_number,
            last_shift_index: TreeIndex::default(),
            last_height: 0,
            last_node_pos: {
                let mut arr = [0u32; 128];
                arr[0] = new_root;
                arr
            },
        };

        println!("Block number: {}, f: new_writer(), l: 219", mv.block_number);

        tree_guard.roots.insert(mv.block_number, MemTreeRoot { node_index: new_root, block_id });
        mv
    }

    pub fn prune(tree: Arc<Mutex<MemTree>>, before_block: u64) {
        let mut tree_guard = tree.lock().unwrap();
        if tree_guard.blocks.first > before_block || before_block > tree_guard.blocks.after_last {
            panic!("invalid prune limit block number");
        }
        let node_boundary = tree_guard.roots.get(&before_block).unwrap().node_index;
        let mut pos_map = Vec::<u32>::with_capacity(node_boundary as usize);
        Self::mark(&tree_guard, node_boundary, &mut pos_map);
        let mut new_pos = 0;
        for i in 0..pos_map.len() {
            if pos_map[i] == 0 {
                continue;
            }
            pos_map[i] = new_pos;
            if new_pos != i as u32 {
                tree_guard.nodes[new_pos as usize] = tree_guard.nodes[i];
            }
            new_pos += 1;
        }

        let node_count = tree_guard.node_count;
        let slice: Vec<MemTreeNode> =
            tree_guard.nodes[node_boundary as usize..tree_guard.node_count as usize].to_vec();
        let mut_slice = tree_guard
            .nodes
            .get_mut(new_pos as usize..(node_count + new_pos - node_boundary) as usize)
            .unwrap();
        mut_slice.copy_from_slice(&slice);

        for pos in tree_guard.node_count + new_pos - node_boundary..tree_guard.node_count {
            tree_guard.nodes[pos as usize] = MemTreeNode::default();
        }

        let pos_mapping = |old_pos: u32| -> u32 {
            if old_pos < node_boundary {
                return *pos_map.get(old_pos as usize).unwrap();
            }
            old_pos + new_pos - node_boundary
        };
        for pos in 0..tree_guard.node_count {
            let node = tree_guard.nodes.get_mut(pos as usize).unwrap();
            if !node.is_leaf() {
                node.set_children(pos_mapping(node.left_child()), pos_mapping(node.right_child()));
            }
        }
        // TODO: Is this clone necessary?
        let roots = tree_guard.roots.clone();
        for (block, root) in &roots {
            if *block < before_block {
                tree_guard.roots.remove(block);
            } else {
                let mut_guard = tree_guard.roots.get_mut(block).unwrap();
                *mut_guard = MemTreeRoot {
                    node_index: pos_mapping(root.node_index),
                    block_id: root.block_id,
                };
            }
        }
        tree_guard.blocks.set_first(before_block);
        tree_guard.node_count = pos_mapping(tree_guard.node_count);
    }

    // mark nodes referenced by the first remaining block
    fn mark(tree_guard: &MutexGuard<'_, MemTree>, node_pos: u32, pos_map: &mut Vec<u32>) {
        pos_map[node_pos as usize] = 1;
        let node = &tree_guard.nodes[node_pos as usize];
        if !node.is_finalized() && !node.is_leaf() {
            Self::mark(tree_guard, node.left_child(), pos_map);
            Self::mark(tree_guard, node.right_child(), pos_map);
        }
    }

    pub fn find_position(&mut self, index: TreeIndex) -> (u32, u64, u64) {
        let height = 127 - index.leading_zeros();
        let shift_index = index.shift_left(128 - height);
        let mut node_height = min(
            min(shift_index.xor(self.last_shift_index).leading_zeros(), height),
            self.last_height,
        );
        let mut node_pos = self.last_node_pos[node_height as usize];
        let tree_guard = self.tree.lock().unwrap();
        while node_height < height && !tree_guard.nodes[node_pos as usize].is_leaf() {
            if shift_index.bit(127 - node_height) == 0 {
                node_pos = tree_guard.nodes[node_pos as usize].left_child();
            } else {
                node_pos = tree_guard.nodes[node_pos as usize].right_child();
            }
            node_height += 1;
            self.last_node_pos[node_height as usize] = node_pos;
        }
        (self.last_shift_index, self.last_height) = (shift_index, node_height);
        (node_pos, node_height, height - node_height)
    }

    pub fn try_get(&mut self, index: TreeIndex) -> (TreeNode, bool, u64) {
        let (node_pos, _height, below) = self.find_position(index);
        let tree_guard = self.tree.lock().unwrap();
        let n = tree_guard.nodes.get(node_pos as usize).unwrap();
        (n.node, n.is_known(), below)
    }

    pub fn get(&mut self, index: TreeIndex) -> TreeNode {
        let (node_pos, _height, below) = self.find_position(index);
        let tree_guard = self.tree.lock().unwrap();
        if below != 0 {
            panic!("cannot read non-existent node");
        }
        let n = tree_guard.nodes.get(node_pos as usize).unwrap();
        if !n.is_known() {
            panic!("cannot read unknown node contents");
        }
        n.node
    }

    pub fn is_leaf(&mut self, index: TreeIndex) -> bool {
        let (node_pos, _height, below) = self.find_position(index);
        let tree_guard = self.tree.lock().unwrap();
        if below != 0 {
            panic!("cannot read non-existent node");
        }
        let n = tree_guard.nodes.get(node_pos as usize).unwrap();
        n.is_leaf()
    }

    pub fn is_known(&mut self, index: TreeIndex) -> bool {
        let (node_pos, _height, below) = self.find_position(index);
        let tree_guard = self.tree.lock().unwrap();
        if below != 0 {
            panic!("cannot read non-existent node");
        }
        let n = tree_guard.nodes.get(node_pos as usize).unwrap();
        n.is_known()
    }

    pub fn add_new_path(
        &mut self,
        index: TreeIndex,
        old_height: u64,
        copy_old_nodes: bool,
    ) -> MemTreeNode {
        let mut node_height = old_height;
        while self.last_node_pos[node_height as usize] < self.last_node_pos[0] {
            node_height -= 1;
        }
        let mut node_pos = self.last_node_pos[node_height as usize];
        let mut tree_guard = self.tree.lock().unwrap();

        let mut current_node = tree_guard.nodes[node_pos as usize];
        let mut old_node = current_node;

        if old_node.is_known() {
            old_node.set_known(false);
            tree_guard.nodes[node_pos as usize] = old_node;
            current_node = old_node;
        }

        let target_height = 127 - index.leading_zeros();

        while node_height < target_height {
            println!("Node count: {}, f: add_new_path(), l: 386", tree_guard.node_count);

            let new_node_pos = tree_guard.add_node();
            let mut new_sibling = 0;

            let copy_sibling = !old_node.is_leaf() && old_node != MemTreeNode::default();
            if !copy_sibling {
                println!("Node count: {}, f: add_new_path(), l: 394", tree_guard.node_count);

                new_sibling = tree_guard.add_node();
                tree_guard.nodes[new_sibling as usize].set_leaf();
            }

            if index.bit(target_height - node_height - 1) == 0 {
                if copy_old_nodes {
                    if !old_node.is_leaf() {
                        let left_child_pos = old_node.left_child();
                        let left_child = tree_guard.nodes[left_child_pos as usize];
                        tree_guard.nodes[new_node_pos as usize] = left_child;
                    }
                }
                if copy_sibling {
                    new_sibling = old_node.right_child();
                }
                current_node.set_children(new_node_pos, new_sibling);
                tree_guard.nodes[node_pos as usize] = current_node;
            } else {
                if copy_old_nodes {
                    if !old_node.is_leaf() {
                        let right_child_pos = old_node.right_child();
                        let right_child = tree_guard.nodes[right_child_pos as usize];
                        tree_guard.nodes[new_node_pos as usize] = right_child;
                    }
                }
                if copy_sibling {
                    new_sibling = old_node.left_child();
                }
                current_node.set_children(new_sibling, new_node_pos);
                tree_guard.nodes[node_pos as usize] = current_node;
            }

            node_height += 1;
            node_pos = new_node_pos;
            current_node = tree_guard.nodes[node_pos as usize];

            if node_height < old_height {
                let old_node_pos = self.last_node_pos[node_height as usize];
                old_node = tree_guard.nodes[old_node_pos as usize];
            } else {
                old_node = MemTreeNode::default();
            }
            self.last_node_pos[node_height as usize] = node_pos;
        }

        tree_guard.nodes[node_pos as usize]
    }

    pub fn set(&mut self, index: TreeIndex, value: TreeNode) {
        let (_node_pos, old_height, _below) = self.find_position(index);
        let mut node = self.add_new_path(index, old_height, false);
        node.node = value;
        node.set_leaf();
        node.set_known(true);

        self.expand();
    }

    pub fn finalize(&mut self, index: TreeIndex) {
        let (_node_pos, old_height, below) = self.find_position(index);
        if below != 0 {
            panic!("cannot finalize non-existent node");
        }
        let mut node = self.add_new_path(index, old_height, true);
        node.set_finalized(true);
        self.expand();
    }

    pub fn root_hash(&mut self) -> B256 {
        let mut tree_guard = self.tree.lock().unwrap();
        tree_guard.hash_node(ROOT_INDEX, self.last_node_pos[0]);
        B256::from_slice(&tree_guard.nodes[self.last_node_pos[0] as usize].node)
    }

    fn expand(&mut self) {
        let mut tree_guard = self.tree.lock().unwrap();
        let expand = tree_guard.need_expand();
        if expand {
            tree_guard.expand();
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemTreeView {
    pub tree: Arc<Mutex<MemTree>>,
    pub block_number: u64,
    pub last_shift_index: TreeIndex,
    pub last_height: u64,
    pub last_node_pos: [u32; 128],
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{hex::FromHex, map::HashMap};

    use super::*;

    #[test]
    fn test_mem_tree_needs_expand() {
        let mut mem_tree = MemTree {
            nodes: vec![MemTreeNode::default()],
            node_count: 1,
            blocks: Range { first: 1, after_last: 6 },
            roots: HashMap::<u64, MemTreeRoot>::default(),
        };

        let need_expand = mem_tree.need_expand();
        assert_eq!(need_expand, true);

        println!("Capacity before expand: {}", mem_tree.nodes.capacity());
        mem_tree.expand();
        println!("Capacity after expand: {}", mem_tree.nodes.capacity());
    }

    #[test]
    fn test_node_hash() {
        let mut mem_tree = MemTree {
            nodes: vec![
                MemTreeNode { node: [0u8; 32], left: 1, right: 2 },
                MemTreeNode {
                    node: [1u8; 32],
                    left: (1 << 31) - 1 | (1 << 31),
                    right: (1 << 31) - 1,
                },
                MemTreeNode {
                    node: [2u8; 32],
                    left: (1 << 31) - 1 | (1 << 31),
                    right: (1 << 31) - 1,
                },
            ],
            node_count: 3,
            blocks: Range { first: 0, after_last: 1 },
            roots: {
                let mut map = HashMap::<u64, MemTreeRoot>::default();
                map.insert(0, MemTreeRoot { node_index: 0, block_id: B256::ZERO });
                map
            },
        };

        println!(
            "Left child: {}, right child: {}",
            mem_tree.nodes[0].left_child(),
            mem_tree.nodes[0].right_child()
        );
        mem_tree.hash_node(ROOT_INDEX, 0);
        println!("Nodes after hashing: {:?}", mem_tree.nodes);

        mem_tree.nodes[1].set_children(1, 2);
        assert_eq!(mem_tree.nodes[1].is_leaf(), false);
        println!("Nodes after setting children: {:?}", mem_tree.nodes);
    }

    #[test]
    fn test_mem_tree_view() {
        let mut mem_tree = MemTree {
            nodes: vec![
                MemTreeNode { node: [0u8; 32], left: 1, right: 2 }, // 0
                MemTreeNode { node: [1u8; 32], left: 3, right: 4 }, // 1
                MemTreeNode { node: [2u8; 32], left: 5, right: 6 }, // 2
                MemTreeNode { node: [3u8; 32], left: 0, right: 0 }, // 3
                MemTreeNode { node: [4u8; 32], left: 0, right: 0 }, // 4
                MemTreeNode { node: [5u8; 32], left: 0, right: 0 }, // 5
                MemTreeNode { node: [6u8; 32], left: 0, right: 0 }, // 6
            ],
            node_count: 7,
            blocks: Range { first: 0, after_last: 3 },
            roots: {
                let mut map = HashMap::<u64, MemTreeRoot>::default();
                map.insert(0, MemTreeRoot { node_index: 0, block_id: B256::ZERO });
                map.insert(
                    1,
                    MemTreeRoot {
                        node_index: 1,
                        block_id: B256::from_hex(
                            "6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b",
                        )
                        .unwrap(),
                    },
                );
                map.insert(
                    2,
                    MemTreeRoot {
                        node_index: 1,
                        block_id: B256::from_hex(
                            "d4735e3a265e16eee03f59718b9b5d03019c07d8b6c51f90da3a666eec13ab35",
                        )
                        .unwrap(),
                    },
                );
                map
            },
        };

        mem_tree.nodes[0].set_known(true);
        mem_tree.nodes[1].set_known(true);
        mem_tree.nodes[2].set_known(true);
        mem_tree.nodes[3].set_known(true);
        mem_tree.nodes[4].set_known(true);
        mem_tree.nodes[5].set_known(true);
        mem_tree.nodes[6].set_known(true);

        let block_number = 2;
        let parent_id =
            B256::from_hex("6b86b273ff34fce19d6b804eff5a3f5747ada4eaa22f1d49c01e52ddb7875b4b")
                .unwrap();
        let block_id =
            B256::from_hex("d4735e3a265e16eee03f59718b9b5d03019c07d8b6c51f90da3a666eec13ab35")
                .unwrap();

        let mut mv = MemTreeView::new_writer(
            Arc::new(Mutex::new(mem_tree)),
            block_number,
            parent_id,
            block_id,
        );

        let _mem_tree_ref = mv.tree.lock().unwrap();
        println!(
            "Node 0 leaf: {}, Node 1 leaf: {}, Node 2 leaf: {}, Node 3 leaf: {}, Node 4 leaf: {}, Node 5 leaf: {}",
            _mem_tree_ref.nodes[0].is_leaf(),
            _mem_tree_ref.nodes[1].is_leaf(),
            _mem_tree_ref.nodes[2].is_leaf(),
            _mem_tree_ref.nodes[3].is_leaf(),
            _mem_tree_ref.nodes[4].is_leaf(),
            _mem_tree_ref.nodes[5].is_leaf()
        );
        drop(_mem_tree_ref);

        println!("Root hash: {}", mv.root_hash());
    }
}
