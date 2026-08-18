//! Port of `Quake/hash_map.c` — byte-blob key/value hash map with
//! insertion-order storage.
//!
//! The observable machine is preserved exactly (ROADMAP Phase 1: progs symbol
//! lookup relies on it; do not substitute std::collections::HashMap):
//! - insert pushes new entries at the *head* of their bucket chain, so lookup
//!   finds the most recently inserted duplicate first (pr_edict builds its
//!   symbol maps in reverse to get linear-scan first-match semantics)
//! - inserting an existing key overwrites the value in place and returns
//!   `true` (returns `false` when a new entry was added)
//! - erase swap-removes with the last entry and re-heads the moved entry into
//!   its bucket; the index order seen through get_key/get_value is observable
//!   (gl_model.c iterates by index)
//! - rehash re-inserts in ascending index order (reversing chains)
//! - growth is 2x with minimums 16 (storage) / 32 (buckets); rehash triggers
//!   when `n + n/4 >= hash_size`; all bucket math is `hash & (size - 1)`
//!
//! The hasher/comparator are boxed closures so the quake-capi FFI adapter can
//! capture C function pointers; a `None` comparator means byte equality
//! (C: memcmp == 0).

const MIN_KEY_VALUE_STORAGE_SIZE: u32 = 16;
const MIN_HASH_SIZE: u32 = 32;

/// `Q_nextPow2` (mathlib.h): identity on powers of two, 1 for val <= 1.
pub fn q_next_pow2(val: u32) -> u32 {
    if val > 1 {
        1u32 << ((31 - (val - 1).leading_zeros()) + 1)
    } else {
        1
    }
}

pub type Hasher = Box<dyn Fn(&[u8]) -> u32>;
pub type Comp = Box<dyn Fn(&[u8], &[u8]) -> bool>;

pub struct QHashMap {
    num_entries: u32,
    hash_size: u32,
    key_value_storage_size: u32,
    key_size: u32,
    value_size: u32,
    hasher: Hasher,
    comp: Option<Comp>,
    hash_to_index: Vec<u32>,
    index_chain: Vec<u32>,
    keys: Vec<u8>,
    values: Vec<u8>,
}

impl QHashMap {
    pub fn new(key_size: u32, value_size: u32, hasher: Hasher, comp: Option<Comp>) -> Self {
        QHashMap {
            num_entries: 0,
            hash_size: 0,
            key_value_storage_size: 0,
            key_size,
            value_size,
            hasher,
            comp,
            hash_to_index: Vec::new(),
            index_chain: Vec::new(),
            keys: Vec::new(),
            values: Vec::new(),
        }
    }

    pub fn key_size(&self) -> u32 {
        self.key_size
    }

    pub fn value_size(&self) -> u32 {
        self.value_size
    }

    pub fn size(&self) -> u32 {
        self.num_entries
    }

    pub fn get_key(&self, index: u32) -> &[u8] {
        let start = (self.key_size * index) as usize;
        &self.keys[start..start + self.key_size as usize]
    }

    pub fn get_value(&self, index: u32) -> &[u8] {
        let start = (self.value_size * index) as usize;
        &self.values[start..start + self.value_size as usize]
    }

    pub fn get_value_mut(&mut self, index: u32) -> &mut [u8] {
        let start = (self.value_size * index) as usize;
        let end = start + self.value_size as usize;
        &mut self.values[start..end]
    }

    /// Raw storage base pointers, for the FFI adapter's pointer-returning
    /// lookups (C callers receive pointers into the map's storage).
    pub fn key_storage_ptr(&mut self) -> *mut u8 {
        self.keys.as_mut_ptr()
    }

    pub fn value_storage_ptr(&mut self) -> *mut u8 {
        self.values.as_mut_ptr()
    }

    fn keys_equal(&self, key: &[u8], storage_key: &[u8]) -> bool {
        match &self.comp {
            Some(comp) => comp(key, storage_key),
            None => key == storage_key,
        }
    }

    fn rehash(&mut self, new_size: u32) {
        if self.hash_size >= new_size {
            return;
        }
        self.hash_size = new_size;
        self.hash_to_index.clear();
        self.hash_to_index.resize(new_size as usize, u32::MAX);
        for i in 0..self.num_entries {
            let hash = (self.hasher)(self.get_key(i));
            let hash_index = (hash & (self.hash_size - 1)) as usize;
            self.index_chain[i as usize] = self.hash_to_index[hash_index];
            self.hash_to_index[hash_index] = i;
        }
    }

    fn expand_key_value_storage(&mut self, new_size: u32) {
        self.keys.resize((new_size * self.key_size) as usize, 0);
        self.values.resize((new_size * self.value_size) as usize, 0);
        self.index_chain.resize(new_size as usize, 0);
        self.key_value_storage_size = new_size;
    }

    pub fn reserve(&mut self, capacity: i32) {
        let new_key_value_storage_size = q_next_pow2(capacity as u32);
        if self.key_value_storage_size < new_key_value_storage_size {
            self.expand_key_value_storage(new_key_value_storage_size);
        }
        let new_hash_size = q_next_pow2((capacity + (capacity / 4)) as u32);
        if self.hash_size < new_hash_size {
            self.rehash(new_hash_size);
        }
    }

    /// Removes all entries but keeps the allocated storage.
    pub fn clear(&mut self) {
        if self.num_entries != 0 && !self.hash_to_index.is_empty() {
            self.hash_to_index.fill(u32::MAX);
        }
        self.num_entries = 0;
    }

    /// Returns `true` if the key already existed (value overwritten in place).
    pub fn insert(&mut self, key: &[u8], value: &[u8]) -> bool {
        debug_assert_eq!(key.len(), self.key_size as usize);
        debug_assert_eq!(value.len(), self.value_size as usize);

        if self.num_entries >= self.key_value_storage_size {
            self.expand_key_value_storage(
                (self.key_value_storage_size * 2).max(MIN_KEY_VALUE_STORAGE_SIZE),
            );
        }
        if self.num_entries + self.num_entries / 4 >= self.hash_size {
            self.rehash((self.hash_size * 2).max(MIN_HASH_SIZE));
        }

        let hash = (self.hasher)(key);
        let hash_index = (hash & (self.hash_size - 1)) as usize;
        let mut storage_index = self.hash_to_index[hash_index];
        while storage_index != u32::MAX {
            if self.keys_equal(key, self.get_key(storage_index)) {
                self.get_value_mut(storage_index).copy_from_slice(value);
                return true;
            }
            storage_index = self.index_chain[storage_index as usize];
        }

        self.index_chain[self.num_entries as usize] = self.hash_to_index[hash_index];
        self.hash_to_index[hash_index] = self.num_entries;
        let n = self.num_entries;
        let key_start = (self.key_size * n) as usize;
        self.keys[key_start..key_start + self.key_size as usize].copy_from_slice(key);
        let value_start = (self.value_size * n) as usize;
        self.values[value_start..value_start + self.value_size as usize].copy_from_slice(value);
        self.num_entries += 1;

        false
    }

    pub fn erase(&mut self, key: &[u8]) -> bool {
        debug_assert_eq!(key.len(), self.key_size as usize);
        if self.num_entries == 0 {
            return false;
        }

        let hash = (self.hasher)(key);
        let hash_index = (hash & (self.hash_size - 1)) as usize;
        let mut storage_index = self.hash_to_index[hash_index];
        // C uses a pointer to the previous chain slot; an index enum works the
        // same: None = bucket head, Some(i) = index_chain[i]
        let mut prev_storage_index: Option<u32> = None;
        while storage_index != u32::MAX {
            if self.keys_equal(key, self.get_key(storage_index)) {
                // Remove found key from index
                let next = self.index_chain[storage_index as usize];
                match prev_storage_index {
                    None => self.hash_to_index[hash_index] = next,
                    Some(prev) => self.index_chain[prev as usize] = next,
                }

                let last_index = self.num_entries - 1;
                let last_hash = (self.hasher)(self.get_key(last_index));
                let last_hash_index = (last_hash & (self.hash_size - 1)) as usize;

                if storage_index == last_index {
                    self.num_entries -= 1;
                    return true;
                }

                // Remove last key from index
                if self.hash_to_index[last_hash_index] == last_index {
                    self.hash_to_index[last_hash_index] = self.index_chain[last_index as usize];
                } else {
                    let mut found = false;
                    let mut last_storage_index = self.hash_to_index[last_hash_index];
                    while last_storage_index != u32::MAX {
                        if self.index_chain[last_storage_index as usize] == last_index {
                            self.index_chain[last_storage_index as usize] =
                                self.index_chain[last_index as usize];
                            found = true;
                            break;
                        }
                        last_storage_index = self.index_chain[last_storage_index as usize];
                    }
                    debug_assert!(found);
                }

                // Copy last key to current key position and add back to index
                let key_size = self.key_size as usize;
                let value_size = self.value_size as usize;
                let (dst_k, src_k) = (
                    storage_index as usize * key_size,
                    last_index as usize * key_size,
                );
                self.keys.copy_within(src_k..src_k + key_size, dst_k);
                let (dst_v, src_v) = (
                    storage_index as usize * value_size,
                    last_index as usize * value_size,
                );
                self.values.copy_within(src_v..src_v + value_size, dst_v);
                self.index_chain[storage_index as usize] = self.hash_to_index[last_hash_index];
                self.hash_to_index[last_hash_index] = storage_index;

                self.num_entries -= 1;
                return true;
            }
            prev_storage_index = Some(storage_index);
            storage_index = self.index_chain[storage_index as usize];
        }
        false
    }

    /// Returns the storage index of the first matching entry (chain order =
    /// most recent insert first), or `None`.
    pub fn lookup(&self, key: &[u8]) -> Option<u32> {
        debug_assert_eq!(key.len(), self.key_size as usize);
        if self.num_entries == 0 {
            return None;
        }

        let hash = (self.hasher)(key);
        let hash_index = (hash & (self.hash_size - 1)) as usize;
        let mut storage_index = self.hash_to_index[hash_index];
        while storage_index != u32::MAX {
            if self.keys_equal(key, self.get_key(storage_index)) {
                return Some(storage_index);
            }
            storage_index = self.index_chain[storage_index as usize];
        }
        None
    }
}

/// The static-inline hashers from `Quake/hash_map.h`, for Rust-side callers
/// (the C header survives for C callers).
pub mod hashers {
    /// Murmur3 fmix32 (C `HashInt32`).
    pub fn hash_int32(v: u32) -> u32 {
        let mut h = v;
        h ^= h >> 16;
        h = h.wrapping_mul(0x85ebca6b);
        h ^= h >> 13;
        h = h.wrapping_mul(0xc2b2ae35);
        h ^= h >> 16;
        h
    }

    /// Murmur3 fmix64, truncated (C `HashInt64`).
    pub fn hash_int64(v: u64) -> u32 {
        let mut k = v;
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51afd7ed558ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
        k ^= k >> 33;
        k as u32
    }

    /// C `HashFloat`: -0.0 normalizes to +0.0 before hashing the bits.
    pub fn hash_float(v: f32) -> u32 {
        let mut bits = v.to_bits();
        if bits == 0x8000_0000 {
            bits = 0;
        }
        hash_int32(bits)
    }

    /// Murmur3 hash combine (C `HashCombine`).
    pub fn hash_combine(a: u32, b: u32) -> u32 {
        let mut a = a.wrapping_mul(0xcc9e2d51);
        a = a.rotate_right(17);
        a = a.wrapping_mul(0x1b873593);
        let mut b = b ^ a;
        b = b.rotate_right(19);
        b.wrapping_mul(5).wrapping_add(0xe6546b64)
    }

    pub fn hash_vec2(v: &[f32; 2]) -> u32 {
        hash_combine(hash_float(v[0]), hash_float(v[1]))
    }

    pub fn hash_vec3(v: &[f32; 3]) -> u32 {
        hash_combine(
            hash_float(v[0]),
            hash_combine(hash_float(v[1]), hash_float(v[2])),
        )
    }

    /// FNV-1a over a NUL-free byte string (C `HashStr`). Note the offset
    /// basis is 0, not the standard 0x811c9dc5 — observable through bucket
    /// distribution and preserved.
    pub fn hash_str(s: &[u8]) -> u32 {
        const FNV_32_PRIME: u32 = 0x0100_0193;
        let mut hval: u32 = 0;
        for &b in s {
            if b == 0 {
                break;
            }
            hval ^= b as u32;
            hval = hval.wrapping_mul(FNV_32_PRIME);
        }
        hval
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_map() -> QHashMap {
        QHashMap::new(
            4,
            8,
            Box::new(|k: &[u8]| hashers::hash_int32(u32::from_ne_bytes(k.try_into().unwrap()))),
            None,
        )
    }

    #[test]
    fn insert_overwrite_and_first_match() {
        let mut map = int_map();
        assert!(!map.insert(&1u32.to_ne_bytes(), &10u64.to_ne_bytes()));
        assert!(map.insert(&1u32.to_ne_bytes(), &20u64.to_ne_bytes()));
        let idx = map.lookup(&1u32.to_ne_bytes()).unwrap();
        assert_eq!(map.get_value(idx), &20u64.to_ne_bytes());
        assert_eq!(map.size(), 1);
    }

    #[test]
    fn most_recent_insert_wins_with_custom_comp() {
        // an always-equal comparator makes duplicates only detectable by the
        // comp itself; with an all-collide hasher, chain order is observable
        let mut map = QHashMap::new(4, 4, Box::new(|_| 7), None);
        map.insert(&1u32.to_ne_bytes(), &1u32.to_ne_bytes());
        map.insert(&2u32.to_ne_bytes(), &2u32.to_ne_bytes());
        map.insert(&3u32.to_ne_bytes(), &3u32.to_ne_bytes());
        // all in one bucket; distinct keys still found exactly
        for k in 1u32..=3 {
            let idx = map.lookup(&k.to_ne_bytes()).unwrap();
            assert_eq!(map.get_value(idx), &k.to_ne_bytes());
        }
    }

    #[test]
    fn basic_test_mirror() {
        // mirror of the C HashMap_BasicTest
        for reserve in [false, true] {
            let mut map = int_map();
            const TEST_SIZE: i32 = 1000;
            if reserve {
                map.reserve(TEST_SIZE);
            }
            for i in 0..TEST_SIZE {
                assert!(!map.insert(&(i as u32).to_ne_bytes(), &(i as u64).to_ne_bytes()));
            }
            for i in 0..TEST_SIZE {
                let idx = map.lookup(&(i as u32).to_ne_bytes()).unwrap();
                assert_eq!(map.get_value(idx), &(i as u64).to_ne_bytes());
            }
            for i in (0..TEST_SIZE).step_by(2) {
                assert!(map.erase(&(i as u32).to_ne_bytes()));
            }
            for i in (1..TEST_SIZE).step_by(2) {
                let idx = map.lookup(&(i as u32).to_ne_bytes()).unwrap();
                assert_eq!(map.get_value(idx), &(i as u64).to_ne_bytes());
            }
            for i in (0..TEST_SIZE).step_by(2) {
                assert!(map.lookup(&(i as u32).to_ne_bytes()).is_none());
            }
            for i in 0..TEST_SIZE {
                map.erase(&(i as u32).to_ne_bytes());
            }
            assert_eq!(map.size(), 0);
        }
    }

    #[test]
    fn next_pow2_matches_c() {
        assert_eq!(q_next_pow2(0), 1);
        assert_eq!(q_next_pow2(1), 1);
        assert_eq!(q_next_pow2(2), 2);
        assert_eq!(q_next_pow2(3), 4);
        assert_eq!(q_next_pow2(16), 16);
        assert_eq!(q_next_pow2(17), 32);
        assert_eq!(q_next_pow2(0x40000000), 0x40000000);
    }
}
