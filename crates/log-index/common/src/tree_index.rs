#[derive(Debug, Default, Clone, Copy, PartialEq)]
///
pub struct TreeIndex {
    pub lo: u64,
    pub hi: u64,
}

pub type TreeNode = [u8; 32];

pub const GTI_EPOCHS: TreeIndex = TreeIndex { lo: 2, hi: 0 };
pub const ROOT_INDEX: TreeIndex = TreeIndex { lo: 1, hi: 0 };
pub const GTI_NEXT_INDEX: TreeIndex = TreeIndex { lo: 3, hi: 0 };
pub const GTI_LOG_ENTRIES: TreeIndex = TreeIndex { lo: 3, hi: 0 };

pub const GTI_DELIMITER_ZERO: TreeIndex = TreeIndex { lo: 1, hi: 0 };
pub const GTI_DELIMITER_META_BLOCK_NUMBER: TreeIndex = TreeIndex { lo: 2, hi: 0 };
pub const GTI_DELIMITER_META_BLOCK_HASH: TreeIndex = TreeIndex { lo: 3, hi: 0 };
pub const GTI_DELIMITER_META_TIMESTAMP: TreeIndex = TreeIndex { lo: 4, hi: 0 };
pub const GTI_DELIMITER_META_DUMMY: TreeIndex = TreeIndex { lo: 5, hi: 0 };

pub const GTI_LOG_ADDRESS: TreeIndex = TreeIndex { lo: 1, hi: 0 };
pub const GTI_LOG_TOPICS_LENGTH: TreeIndex = TreeIndex { lo: 2, hi: 0 };
pub const GTI_LOG_TOPICS_ROOT: TreeIndex = TreeIndex { lo: 3, hi: 0 };
pub const GTI_LOG_DATA: TreeIndex = TreeIndex { lo: 4, hi: 0 };
pub const GTI_LOG_ZERO: TreeIndex = TreeIndex { lo: 5, hi: 0 };
pub const GTI_LOG_META_BLOCK_NUMBER: TreeIndex = TreeIndex { lo: 6, hi: 0 };
pub const GTI_LOG_META_TX_HASH: TreeIndex = TreeIndex { lo: 7, hi: 0 };
pub const GTI_LOG_META_TX_INDEX: TreeIndex = TreeIndex { lo: 8, hi: 0 };
pub const GTI_LOG_META_LOG_INDEX: TreeIndex = TreeIndex { lo: 9, hi: 0 };

pub const GTI_FILTER_MAPS: TreeIndex = TreeIndex { lo: 1, hi: 0 };
pub const GTI_PROG_LIST_TREE: TreeIndex = TreeIndex { lo: 1, hi: 0 };
pub const GTI_PROG_LIST_COUNT: TreeIndex = TreeIndex { lo: 2, hi: 0 };
pub const GTI_PROG_LIST_NEXT_TREE: TreeIndex = TreeIndex { lo: 3, hi: 0 };
pub const GTI_PROG_LIST_SUBTREE: TreeIndex = TreeIndex { lo: 4, hi: 0 };

pub const ZERO_HASHES: [TreeNode; 256] = [[0u8; 32]; 256];

// Constants for caching
pub const CACHED_ROW_MAPPINGS: u32 = 1000;

impl TreeIndex {
    pub fn leading_zeros(self) -> u64 {
        if self.hi == 0 {
            return self.lo.leading_zeros() as u64 + 64;
        }
        self.hi.leading_zeros() as u64
    }

    pub fn level(self) -> u64 {
        127 - self.leading_zeros()
    }

    pub fn shift_left(self, b: u64) -> Self {
        if b == 0 {
            return self;
        }
        if b >= 64 {
            return Self { lo: 0, hi: self.lo << (b - 64) };
        }
        Self { lo: self.lo << b, hi: (self.hi << b) + (self.lo >> (64 - b)) }
    }

    pub fn shift_right(self, b: u64) -> Self {
        if b == 0 {
            return self;
        }
        if b >= 64 {
            return Self { lo: self.hi >> (b - 64), hi: 0 };
        }
        Self { lo: (self.lo >> b) + (self.hi << (64 - b)), hi: self.hi >> b }
    }

    pub fn add_int(self, add: i64) -> Self {
        let mut r = self.clone();
        r.lo += add as u64;
        if add > 0 && r.lo < self.lo {
            r.hi += 1;
        }
        if add < 0 && r.lo > self.lo {
            r.hi -= 1;
        }
        r
    }

    pub fn bit(self, b: u64) -> u64 {
        if b < 64 {
            return (self.lo >> b) & 1;
        }
        (self.hi >> (b - 64)) & 1
    }

    pub fn lower_bits(self, b: u64) -> Self {
        if b <= 64 {
            return Self { lo: self.lo & ((1 << b) - 1), hi: 0 };
        }
        Self { lo: self.lo, hi: self.hi & ((1 << (b - 64)) - 1) }
    }

    pub fn split(self, split_level: u64) -> (Self, Self) {
        let mut level = self.level();
        if level <= split_level {
            return (self, ROOT_INDEX);
        }
        level -= split_level;
        (self.shift_right(level), self.lower_bits(level))
    }

    pub fn or(self, s: Self) -> Self {
        Self { lo: self.lo | s.lo, hi: self.hi | s.hi }
    }

    pub fn xor(self, s: Self) -> Self {
        Self { lo: self.lo ^ s.lo, hi: self.hi ^ s.hi }
    }

    ///
    pub fn child(self, s: Self) -> Self {
        let l = s.level();
        self.shift_left(l).or(s.lower_bits(l))
    }

    ///
    pub fn append(self, index: u64, height: u64) -> Self {
        let mut res = self.shift_left(height);
        res.lo |= index;
        res
    }
}
