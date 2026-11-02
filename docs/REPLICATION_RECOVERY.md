# Spine Replication Recovery & Anti-Entropy

## 1. Objectives & Scope

Replication in distributed systems faces unpredictable transient network disruptions, packet loss, node crashes, and silent storage failures. To prevent silent replica divergence without stopping active cluster operations, Spine provides a dual-layer recovery architecture:

1. **Transactional Crash Recovery:** Immediate handling of in-flight two-phase commit transactions upon node or coordinator restart.
2. **Anti-Entropy Synchronization:** Continuous background reconciliation of repository pack catalogs across replicas and primaries.

---

## 2. Anti-Entropy Reconciliation Algorithm

The Spine Anti-Entropy engine (`AntiEntropy` in `spine_cluster::repair`) operates on pack catalogs rather than expensive object-by-object tree traversals. Because Spine packfiles (`.spk`) are immutable and content-addressed via BLAKE3 hashes of their compressed contents, comparing the set of pack IDs across nodes yields an exact set difference in $O(N \log N)$ or $O(N)$ hash-set time.

### 2.1 Pack Diffing Logic

Given source node pack catalog $P_{\text{src}}$ and target replica pack catalog $P_{\text{rep}}$:

$$\Delta_{\text{missing}} = P_{\text{src}} \setminus P_{\text{rep}} = \{ p \in P_{\text{src}} \mid p \notin P_{\text{rep}} \}$$

```rust
pub fn diff_packs(source_packs: &[String], replica_packs: &[String]) -> Vec<String> {
    let rep_set: HashSet<&String> = replica_packs.iter().collect();
    source_packs
        .iter()
        .filter(|p| !rep_set.contains(p))
        .cloned()
        .collect()
}
```

### 2.2 Repair Job Generation & Dispatch
1. When a node's heartbeat reports `node_generation < committed_generation` or an operator triggers `spine replication repair --repo-id <ID> --target-node <NODE>`, the coordinator generates a `RepairJob`.
2. The coordinator identifies missing pack files $\Delta_{\text{missing}}$.
3. For each missing pack $p_i \in \Delta_{\text{missing}}$:
   - A `RepairPack` RPC is dispatched to a healthy source node.
   - The source node streams the pack binary and index binary.
   - The target node verifies the BLAKE3 hash:
     $$\text{BLAKE3}(B_{\text{received}}) \stackrel{?}{=} H_{\text{expected}}$$
   - On match, the target node persists $p_i$ into its `<objects>/pack/` store.
4. Once all missing packs are verified and installed, the target node's local generation is advanced to match the authoritative generation:
   $$\text{Generation}(R) \leftarrow \text{CommittedGeneration}$$
5. The `RepairJob` transitions to `Completed`.

---

## 3. Crash Recovery Matrix & Guarantees

| In-Flight State at Crash | Local Action on Boot | Cluster Action on Boot |
| :--- | :--- | :--- |
| **Node crashed during staging** | Discard temporary `<staging>/<tx_id>/` | Coordinator retries on another replica or aborts if below durability threshold |
| **Node crashed after staging** | Staged pack retained until commit or abort RPC received | Coordinator re-transmits `CommitPush` if `CommitDecided`, else `AbortPush` |
| **Coordinator crashed after CommitDecided** | Retain staged packs | Coordinator restarts, reads `CommitDecided` log, completes commits on all nodes |
| **Node missed offline commits** | Boots normally with older generation | Anti-Entropy detects generation gap $\rightarrow$ schedules `RepairJob` |
