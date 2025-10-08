use reth_log_index_common::{TreeIndex, TreeNode};

pub trait TreeReader {
    fn get(&self, index: TreeIndex) -> TreeNode;
    fn try_get(&self, index: TreeIndex) -> (TreeNode, bool, u64);
    fn is_leaf(&self, index: TreeIndex) -> bool;
}
