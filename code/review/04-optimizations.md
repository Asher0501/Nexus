# 🟢 Non-Urgent Optimization Opportunities

> Performance improvements for future consideration. Not blocking for v1.

---

## O1: `EdgeState.received` — HashSet → FixedBitSet for large fan-in

**File**: `nexus-engine/src/graph/edge.rs:60`

```rust
pub struct EdgeState {
    pub triggered: bool,
    pub event_count: u64,
    pub received: HashSet<NodeIndex>,  // Used only for Strategy::All
}
```

### Current behavior

`HashSet<NodeIndex>` tracks which `from_nodes` have signalled. For small fan-in (2-10 nodes) the overhead is negligible. For very large fan-in (100+ nodes), `HashSet` has:
- Higher constant overhead per insertion (hash + allocation)
- Poor cache locality (scattered heap nodes)
- Memory overhead (~32 bytes per entry vs ~1 byte for a bit)

### Opportunity

Since `NodeIndex` values are dense (`0..n-1`), a `FixedBitSet` with capacity `from_nodes.len()` would be:
- More cache-friendly (single contiguous allocation)
- O(1) insert with lower constant
- ~1 bit per entry vs ~32 bytes for HashSet entry

### Trade-off

- Requires knowing `from_nodes.len()` at construction time
- `EdgeState` would need to be initialized differently for Strategy::All vs Strategy::Any
- Not worth it until fan-in edges with 50+ from_nodes are common

---

## O2: Scheduler hot path HashMap → Vec for dense node indices

**File**: `nexus-engine/src/graph/scheduler.rs:206-213`

```rust
let transfer: Option<&NodeTransfer> = self.graph.transfers().get(&node);
let node_state: Option<&mut NodeState> = self.state.states.get_mut(&node);
let counter: Option<&mut NodeCounters> = self.state.counters.get_mut(&node);
```

`handle_event()` does three HashMap lookups per invocation. Since node indices are dense (`0..n-1`), these could be `Vec<NodeState>` indexed by `node.index()`.

### Trade-off

- HashMap: flexible (sparse indices work), O(1) amortized, but hashing overhead
- Vec: O(1) direct, better cache locality, but requires dense indices (already guaranteed)
- Not worth it for typical workflow sizes (10-100 nodes). Only matters if:
  - handle_event is called 10k+ times in tight loop (e.g., high-threshold cycles)
  - Graphs with 1000+ nodes

---

## O3: `GraphDef::node_indices()` rebuilds iterator each call

**File**: `nexus-engine/src/graph/graph_def.rs:224-226`

```rust
pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
    (0..self.graph.node_count()).map(NodeIndex::new)
}
```

Currently called in `Scheduler::new()` which runs once. If called in hot path, caching the Vec would save repeated computation. Not an issue today.

---

## O4: `DataRouter.build_input()` clones output strings

**File**: `nexus-engine/src/graph/data_router.rs:54`

```rust
let output = self.outputs.get(node_idx).cloned().unwrap_or_default();
```

Each `build_input()` call clones all requested outputs. For workflows where:
- A large output (MB+) is referenced by many downstream nodes
- Or `build_input` is called many times for the same node

Consider `Arc<String>` or reference-counted outputs. Not an issue for typical use cases where outputs are small strings (<1KB).

---

## O5: `Builder::build_edges()` uses `HashSet<NodeIndex>` then converts to `Vec`

**File**: `nexus-engine/src/graph/builder.rs:113`

```rust
let mut edge_groups: HashMap<EdgeKey, HashSet<NodeIndex>> = HashMap::new();
// ...
let mut from_nodes: Vec<NodeIndex> = from_set.into_iter().collect();
from_nodes.sort_by_key(|ni| ni.index());
```

Using `HashSet` for deduplication then converting to sorted `Vec` is correct but could be more efficient with `Vec` + `sort` + `dedup`. Not worth optimizing — edge counts are small per group.

---

## O6: `validate()` builds adjacency maps 3 times (BFS, exit BFS, unreachable BFS)

**File**: `nexus-engine/src/graph/validator.rs`

The `validate` function builds the `child_map` HashMap twice:
1. Line 87-94: for BFS from entries (UnreachableNode check)
2. Line 165-173: again for InputSourceUnreachable check

Both could share the same map. Currently they're built in separate code blocks. Small refactoring win.
