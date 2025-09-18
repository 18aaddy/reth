use crate::{hasher::TreeNode, TreeIndex};

pub trait TreeReader {
    fn get(&self, index: TreeIndex) -> TreeNode;
    fn try_get(&self, index: TreeIndex) -> (TreeNode, bool, u64);
    fn is_leaf(&self, index: TreeIndex) -> bool;
}