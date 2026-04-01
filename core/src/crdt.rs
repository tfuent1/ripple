use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CrdtError {
    #[error("serialization failed: {0}")]
    Serialization(#[from] rmp_serde::encode::Error),

    #[error("deserialization failed: {0}")]
    Deserialization(#[from] rmp_serde::decode::Error),
}

// ── CrdtValue ─────────────────────────────────────────────────────────────────

/// The concrete value type stored inside LWWRegister and ORSet entries.
///
/// Using an enum rather than a generic keeps SharedState serializable without
/// any extra complexity — rmp-serde will encode this as a tagged MessagePack
/// array, one variant per type. Add variants here as new use cases emerge.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CrdtValue {
    Text(String),
    Bytes(Vec<u8>),
    Int(i64),
}

// ── LWWRegister ───────────────────────────────────────────────────────────────

/// Last-Write-Wins register. Holds a single value with conflict resolution.
///
/// **How merge works:**
/// Two nodes update the same register concurrently. When they sync, we compare
/// timestamps. Higher timestamp wins. If timestamps are equal (rare but possible
/// with imprecise clocks), the node with the lexicographically larger public key
/// wins — arbitrary, but deterministic, so both sides always agree.
///
/// **Why this is a CRDT:**
/// The merge function is commutative (A⊔B == B⊔A), associative, and idempotent.
/// Any merge order converges to the same result.
///
/// **The tradeoff:**
/// LWW requires reasonably synchronized clocks. A node with a skewed clock can
/// "win" stale updates. For Ripple's use cases (status messages, display names)
/// this is acceptable — the stakes are low and clock skew is bounded in practice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LWWRegister {
    pub value: CrdtValue,
    /// Unix timestamp seconds. Higher wins on merge.
    pub timestamp: i64,
    /// Ed25519 pubkey of the author. Tiebreaker when timestamps match.
    pub author: [u8; 32],
}

impl LWWRegister {
    /// Create a new register with an initial value.
    pub fn new(value: CrdtValue, timestamp: i64, author: [u8; 32]) -> Self {
        Self {
            value,
            timestamp,
            author,
        }
    }

    /// Update the register. The new value only takes effect if it would win
    /// a merge — i.e., the timestamp is strictly later, or equal with a
    /// larger author key. This keeps local state consistent with merge semantics.
    pub fn set(&mut self, value: CrdtValue, timestamp: i64, author: [u8; 32]) {
        if Self::wins_over(timestamp, &author, self.timestamp, &self.author) {
            self.value = value;
            self.timestamp = timestamp;
            self.author = author;
        }
    }

    /// Merge another register into this one. The winner is retained in place.
    pub fn merge(&mut self, other: &LWWRegister) {
        if Self::wins_over(other.timestamp, &other.author, self.timestamp, &self.author) {
            self.value = other.value.clone();
            self.timestamp = other.timestamp;
            self.author = other.author;
        }
    }

    /// Pure merge — returns the winning register without mutating either input.
    /// Used by SharedState::merge to build a new merged state.
    pub fn merged(a: &LWWRegister, b: &LWWRegister) -> LWWRegister {
        if Self::wins_over(b.timestamp, &b.author, a.timestamp, &a.author) {
            b.clone()
        } else {
            a.clone()
        }
    }

    /// Returns true if (ts_a, author_a) beats (ts_b, author_b).
    ///
    /// Rules:
    ///   1. Higher timestamp wins.
    ///   2. On tie: larger author pubkey wins (byte-by-byte comparison).
    ///
    /// **Rust note:** `[u8; 32]` implements `Ord` — arrays of orderable types
    /// are compared lexicographically, element by element. So `a > b` on two
    /// pubkeys just works and gives us a stable, deterministic tiebreaker.
    fn wins_over(ts_a: i64, author_a: &[u8; 32], ts_b: i64, author_b: &[u8; 32]) -> bool {
        ts_a > ts_b || (ts_a == ts_b && author_a > author_b)
    }
}

// ── ORSet ─────────────────────────────────────────────────────────────────────

/// Observed-Remove Set. A set that supports concurrent add and remove correctly.
///
/// **The problem with naive sets:**
/// If Alice adds pin P and Bob removes pin P concurrently, a naive "remove wins"
/// rule deletes Alice's pin even though she added it after Bob's last observation.
/// A "add wins" rule lets Alice's pin survive Bob's remove, which is also wrong.
///
/// **How ORSet fixes this:**
/// Every add operation tags the element with a fresh UUID. A remove operation
/// records the *specific tags* being removed — not the element by value. When
/// merging, an element survives if it has any add-tag that is not in the
/// remove-set. Alice's new add has a new UUID Bob has never seen, so it survives
/// Bob's remove of the old tags.
///
/// **Concretely:**
/// - `add_set`: HashMap<Uuid, CrdtValue> — every add gets a unique tag
/// - `remove_set`: HashSet<Uuid>         — removed tags accumulate forever
/// - Current elements = add_set entries whose tag is NOT in remove_set
///
/// **Why remove-set grows forever:**
/// Tombstones (removed tags) must be retained so a merge with a lagging node
/// doesn't resurrect a removed element. Garbage-collecting tombstones requires
/// distributed coordination and is out of scope for Phase 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ORSet {
    /// Each live or formerly-live element: tag → value.
    add_set: HashMap<Uuid, CrdtValue>,
    /// Tags that have been explicitly removed.
    remove_set: HashSet<Uuid>,
}

impl ORSet {
    pub fn new() -> Self {
        Self {
            add_set: HashMap::new(),
            remove_set: HashSet::new(),
        }
    }

    /// Add a value to the set. Returns the tag UUID so the caller can later
    /// remove this specific instance.
    pub fn add(&mut self, value: CrdtValue) -> Uuid {
        let tag = Uuid::new_v4();
        self.add_set.insert(tag, value);
        tag
    }

    /// Remove all instances of `value` currently visible in the set.
    ///
    /// Only removes tags the local node has observed (i.e., tags in our
    /// add_set). Concurrent adds by other nodes with new tags are unaffected.
    pub fn remove(&mut self, value: &CrdtValue) {
        // Collect all tags associated with this value that are currently live.
        let tags_to_remove: Vec<Uuid> = self
            .add_set
            .iter()
            .filter(|(tag, v)| *v == value && !self.remove_set.contains(tag))
            .map(|(tag, _)| *tag)
            .collect();

        for tag in tags_to_remove {
            self.remove_set.insert(tag);
        }
    }

    /// The current visible elements — add_set entries not in remove_set.
    ///
    /// **Rust note:** `.filter_map()` is like `.map()` followed by `.flatten()`.
    /// Here we produce `Some(value)` for live elements and `None` for tombstoned
    /// ones, then filter_map discards the Nones automatically. The `.collect()`
    /// gathers everything into a Vec.
    pub fn elements(&self) -> Vec<&CrdtValue> {
        self.add_set
            .iter()
            .filter_map(|(tag, value)| {
                if self.remove_set.contains(tag) {
                    None
                } else {
                    Some(value)
                }
            })
            .collect()
    }

    /// Merge another ORSet into this one (in place).
    ///
    /// Merge rule: union the add_sets, union the remove_sets.
    /// This satisfies all three CRDT laws automatically — union is commutative,
    /// associative, and idempotent.
    pub fn merge(&mut self, other: &ORSet) {
        // Union add_sets — other's entries win on tag collision (same UUID means
        // same add, so the value is identical; this is a no-op in practice).
        for (tag, value) in &other.add_set {
            self.add_set.entry(*tag).or_insert_with(|| value.clone());
        }
        // Union remove_sets.
        for tag in &other.remove_set {
            self.remove_set.insert(*tag);
        }
    }

    /// Pure merge — returns a new merged ORSet without mutating either input.
    pub fn merged(a: &ORSet, b: &ORSet) -> ORSet {
        let mut result = a.clone();
        result.merge(b);
        result
    }
}

impl Default for ORSet {
    fn default() -> Self {
        Self::new()
    }
}

// ── SharedState ───────────────────────────────────────────────────────────────

/// Top-level CRDT state container.
///
/// Holds named LWWRegisters (single values) and named ORSets (collections).
/// This is what gets serialized into a bundle payload and exchanged between
/// nodes during a sync session.
///
/// Example layout for Phase 2 use cases:
/// ```text
/// registers["status"]      → LWWRegister { value: Text("Shelter open"), ... }
/// sets["map_pins"]         → ORSet { ... all current map pins ... }
/// sets["resource_posts"]   → ORSet { ... resource availability posts ... }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedState {
    pub registers: HashMap<String, LWWRegister>,
    pub sets: HashMap<String, ORSet>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            registers: HashMap::new(),
            sets: HashMap::new(),
        }
    }

    /// Set a named register value.
    pub fn set_register(
        &mut self,
        key: impl Into<String>,
        value: CrdtValue,
        timestamp: i64,
        author: [u8; 32],
    ) {
        let key = key.into();
        match self.registers.get_mut(&key) {
            Some(reg) => reg.set(value, timestamp, author),
            None => {
                self.registers
                    .insert(key, LWWRegister::new(value, timestamp, author));
            }
        }
    }

    /// Get the current value of a named register, if it exists.
    pub fn get_register(&self, key: &str) -> Option<&CrdtValue> {
        self.registers.get(key).map(|r| &r.value)
    }

    /// Get or create a named ORSet, returning a mutable reference.
    ///
    /// **Rust note:** `.entry(...).or_default(...)` is the idiomatic pattern
    /// for "give me the value at this key, inserting a default if absent." We've
    /// seen this before in PeerManager — same pattern, different type.
    pub fn get_or_create_set(&mut self, key: impl Into<String>) -> &mut ORSet {
        self.sets.entry(key.into()).or_default()
    }
    /// Read access to a named set.
    pub fn get_set(&self, key: &str) -> Option<&ORSet> {
        self.sets.get(key)
    }

    /// Merge two SharedStates into a new one. Neither input is mutated.
    ///
    /// This is the function that gets called when a sync bundle is received —
    /// we merge the remote state into our local state and get a new converged
    /// state back.
    ///
    /// Keys present in only one state are taken as-is (there's nothing to merge
    /// against). Keys present in both are merged by their type's merge rule.
    pub fn merge(a: &SharedState, b: &SharedState) -> SharedState {
        let mut result = SharedState::new();

        // Merge registers — union of keys, LWWRegister::merged for shared keys.
        for (key, reg_a) in &a.registers {
            let merged = match b.registers.get(key) {
                Some(reg_b) => LWWRegister::merged(reg_a, reg_b),
                None => reg_a.clone(),
            };
            result.registers.insert(key.clone(), merged);
        }
        // Keys only in b.
        for (key, reg_b) in &b.registers {
            if !result.registers.contains_key(key) {
                result.registers.insert(key.clone(), reg_b.clone());
            }
        }

        // Merge sets — same pattern as registers.
        for (key, set_a) in &a.sets {
            let merged = match b.sets.get(key) {
                Some(set_b) => ORSet::merged(set_a, set_b),
                None => set_a.clone(),
            };
            result.sets.insert(key.clone(), merged);
        }
        for (key, set_b) in &b.sets {
            if !result.sets.contains_key(key) {
                result.sets.insert(key.clone(), set_b.clone());
            }
        }

        result
    }

    /// Serialize to MessagePack bytes for inclusion in a Bundle payload.
    pub fn to_bytes(&self) -> Result<Vec<u8>, CrdtError> {
        Ok(rmp_serde::to_vec(self)?)
    }

    /// Deserialize from MessagePack bytes received in a Bundle payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CrdtError> {
        Ok(rmp_serde::from_slice(bytes)?)
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed pubkeys for deterministic tiebreaker tests.
    const ALICE: [u8; 32] = [1u8; 32];
    const BOB: [u8; 32] = [2u8; 32];
    const NOW: i64 = 1_700_000_000;

    // ── LWWRegister ───────────────────────────────────────────────────────────

    #[test]
    fn test_lww_higher_timestamp_wins() {
        let mut reg = LWWRegister::new(CrdtValue::Text("old".into()), NOW, ALICE);
        let newer = LWWRegister::new(CrdtValue::Text("new".into()), NOW + 1, BOB);
        reg.merge(&newer);
        assert_eq!(reg.value, CrdtValue::Text("new".into()));
    }

    #[test]
    fn test_lww_lower_timestamp_loses() {
        let mut reg = LWWRegister::new(CrdtValue::Text("current".into()), NOW + 1, ALICE);
        let stale = LWWRegister::new(CrdtValue::Text("stale".into()), NOW, BOB);
        reg.merge(&stale);
        assert_eq!(reg.value, CrdtValue::Text("current".into()));
    }

    #[test]
    fn test_lww_tiebreaker_larger_pubkey_wins() {
        // BOB = [2u8; 32], ALICE = [1u8; 32] — BOB is larger.
        let alice_reg = LWWRegister::new(CrdtValue::Text("alice".into()), NOW, ALICE);
        let bob_reg = LWWRegister::new(CrdtValue::Text("bob".into()), NOW, BOB);
        let merged = LWWRegister::merged(&alice_reg, &bob_reg);
        assert_eq!(merged.value, CrdtValue::Text("bob".into()));
    }

    #[test]
    fn test_lww_commutativity() {
        let a = LWWRegister::new(CrdtValue::Int(1), NOW, ALICE);
        let b = LWWRegister::new(CrdtValue::Int(2), NOW + 1, BOB);

        let ab = LWWRegister::merged(&a, &b);
        let ba = LWWRegister::merged(&b, &a);
        assert_eq!(ab, ba); // merge(A, B) == merge(B, A)
    }

    #[test]
    fn test_lww_idempotency() {
        let a = LWWRegister::new(CrdtValue::Int(42), NOW, ALICE);
        let merged = LWWRegister::merged(&a, &a);
        assert_eq!(merged, a); // merge(A, A) == A
    }

    #[test]
    fn test_lww_associativity() {
        let a = LWWRegister::new(CrdtValue::Int(1), NOW, ALICE);
        let b = LWWRegister::new(CrdtValue::Int(2), NOW + 1, BOB);
        let c = LWWRegister::new(CrdtValue::Int(3), NOW + 2, ALICE);

        // merge(merge(A, B), C)
        let ab_c = LWWRegister::merged(&LWWRegister::merged(&a, &b), &c);
        // merge(A, merge(B, C))
        let a_bc = LWWRegister::merged(&a, &LWWRegister::merged(&b, &c));

        assert_eq!(ab_c, a_bc);
    }

    // ── ORSet ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_orset_add_and_elements() {
        let mut set = ORSet::new();
        set.add(CrdtValue::Text("pin_a".into()));
        set.add(CrdtValue::Text("pin_b".into()));

        let elems: Vec<_> = set.elements();
        assert_eq!(elems.len(), 2);
    }

    #[test]
    fn test_orset_remove() {
        let mut set = ORSet::new();
        set.add(CrdtValue::Text("pin".into()));
        assert_eq!(set.elements().len(), 1);

        set.remove(&CrdtValue::Text("pin".into()));
        assert_eq!(set.elements().len(), 0);
    }

    #[test]
    fn test_orset_concurrent_add_survives_remove() {
        // Alice adds a pin.
        let mut alice_set = ORSet::new();
        alice_set.add(CrdtValue::Text("shelter".into()));

        // Bob starts from a copy of Alice's state, removes the pin.
        let mut bob_set = alice_set.clone();
        bob_set.remove(&CrdtValue::Text("shelter".into()));

        // Concurrently, Alice adds the pin again (new tag).
        alice_set.add(CrdtValue::Text("shelter".into()));

        // Merge: Alice's new add has a tag Bob has never seen, so it survives.
        let merged = ORSet::merged(&alice_set, &bob_set);
        assert_eq!(merged.elements().len(), 1);
    }

    #[test]
    fn test_orset_commutativity() {
        let mut a = ORSet::new();
        a.add(CrdtValue::Text("x".into()));

        let mut b = ORSet::new();
        b.add(CrdtValue::Text("y".into()));

        let ab = ORSet::merged(&a, &b);
        let ba = ORSet::merged(&b, &a);

        // Same elements, regardless of merge order.
        let mut ab_elems: Vec<String> = ab
            .elements()
            .into_iter()
            .map(|v| format!("{:?}", v))
            .collect();
        let mut ba_elems: Vec<String> = ba
            .elements()
            .into_iter()
            .map(|v| format!("{:?}", v))
            .collect();
        ab_elems.sort();
        ba_elems.sort();
        assert_eq!(ab_elems, ba_elems);
    }

    #[test]
    fn test_orset_idempotency() {
        let mut set = ORSet::new();
        set.add(CrdtValue::Text("once".into()));

        let merged = ORSet::merged(&set, &set);
        // Merging with yourself shouldn't duplicate elements.
        assert_eq!(merged.elements().len(), 1);
    }

    #[test]
    fn test_orset_associativity() {
        let mut a = ORSet::new();
        a.add(CrdtValue::Int(1));

        let mut b = ORSet::new();
        b.add(CrdtValue::Int(2));

        let mut c = ORSet::new();
        c.add(CrdtValue::Int(3));

        let ab_c = ORSet::merged(&ORSet::merged(&a, &b), &c);
        let a_bc = ORSet::merged(&a, &ORSet::merged(&b, &c));

        let mut ab_c_elems: Vec<String> = ab_c
            .elements()
            .into_iter()
            .map(|v| format!("{:?}", v))
            .collect();
        let mut a_bc_elems: Vec<String> = a_bc
            .elements()
            .into_iter()
            .map(|v| format!("{:?}", v))
            .collect();
        ab_c_elems.sort();
        a_bc_elems.sort();
        assert_eq!(ab_c_elems, a_bc_elems);
    }

    // ── SharedState ───────────────────────────────────────────────────────────

    #[test]
    fn test_shared_state_register_roundtrip() {
        let mut state = SharedState::new();
        state.set_register("status", CrdtValue::Text("Shelter open".into()), NOW, ALICE);

        assert_eq!(
            state.get_register("status"),
            Some(&CrdtValue::Text("Shelter open".into()))
        );
    }

    #[test]
    fn test_shared_state_merge_commutativity() {
        let mut a = SharedState::new();
        a.set_register("status", CrdtValue::Text("open".into()), NOW, ALICE);

        let mut b = SharedState::new();
        b.set_register("status", CrdtValue::Text("full".into()), NOW + 1, BOB);

        let ab = SharedState::merge(&a, &b);
        let ba = SharedState::merge(&b, &a);
        assert_eq!(ab, ba);
    }

    #[test]
    fn test_shared_state_merge_idempotency() {
        let mut state = SharedState::new();
        state.set_register("x", CrdtValue::Int(1), NOW, ALICE);

        let merged = SharedState::merge(&state, &state);
        assert_eq!(merged, state);
    }

    #[test]
    fn test_shared_state_serialization_roundtrip() {
        let mut state = SharedState::new();
        state.set_register("status", CrdtValue::Text("ok".into()), NOW, ALICE);
        state
            .get_or_create_set("pins")
            .add(CrdtValue::Bytes(vec![1, 2, 3]));

        let bytes = state.to_bytes().unwrap();
        let restored = SharedState::from_bytes(&bytes).unwrap();
        assert_eq!(state, restored);
    }

    #[test]
    fn test_shared_state_keys_only_in_one_side_survive_merge() {
        let mut a = SharedState::new();
        a.set_register("alpha", CrdtValue::Int(1), NOW, ALICE);

        let mut b = SharedState::new();
        b.set_register("beta", CrdtValue::Int(2), NOW, BOB);

        let merged = SharedState::merge(&a, &b);
        assert!(merged.get_register("alpha").is_some());
        assert!(merged.get_register("beta").is_some());
    }
}
