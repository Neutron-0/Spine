use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::transaction::{ReplicationTransaction, TransactionPhase};

#[test]
fn test_durability_policy_enforcement_and_thresholds() {
    // 1. Single copy policy (requires 1 copy)
    let mut tx_single = ReplicationTransaction::new(
        "tx-s1".into(),
        "repo-core".into(),
        0,
        1,
        "node-1".into(),
        1,
        DurabilityPolicy::SingleCopy,
        vec!["node-1".into()],
    );
    assert!(tx_single.start_preparing().is_ok());
    assert!(tx_single.acknowledge_prepared("node-1").unwrap());
    assert_eq!(tx_single.phase, TransactionPhase::Prepared);
    assert!(tx_single.decide_commit(100).is_ok());
    assert!(tx_single.mark_committed(110).is_ok());

    // 2. Two copies policy (requires primary + 1 replica = 2 total)
    let mut tx_two = ReplicationTransaction::new(
        "tx-t2".into(),
        "repo-core".into(),
        1,
        2,
        "node-1".into(),
        1,
        DurabilityPolicy::TwoCopies,
        vec!["node-1".into(), "node-2".into()],
    );
    assert!(tx_two.start_preparing().is_ok());

    // Only primary acknowledged -> must NOT be satisfied yet!
    assert!(!tx_two.acknowledge_prepared("node-1").unwrap());
    assert_eq!(tx_two.phase, TransactionPhase::Preparing);
    assert!(
        tx_two.decide_commit(200).is_err(),
        "Cannot decide commit underprepared"
    );

    // Replica acknowledges -> policy satisfied!
    assert!(tx_two.acknowledge_prepared("node-2").unwrap());
    assert_eq!(tx_two.phase, TransactionPhase::Prepared);
    assert!(tx_two.decide_commit(210).is_ok());
    assert!(tx_two.mark_committed(220).is_ok());

    // 3. Three copies policy (requires primary + 2 replicas = 3 total)
    let mut tx_three = ReplicationTransaction::new(
        "tx-th3".into(),
        "repo-core".into(),
        2,
        3,
        "node-1".into(),
        1,
        DurabilityPolicy::ThreeCopies,
        vec!["node-1".into(), "node-2".into(), "node-3".into()],
    );
    assert!(tx_three.start_preparing().is_ok());
    assert!(!tx_three.acknowledge_prepared("node-1").unwrap());
    assert!(!tx_three.acknowledge_prepared("node-2").unwrap());
    assert_eq!(tx_three.phase, TransactionPhase::Preparing);
    assert!(tx_three.decide_commit(300).is_err());

    // Third node acknowledges -> reaches Prepared
    assert!(tx_three.acknowledge_prepared("node-3").unwrap());
    assert_eq!(tx_three.phase, TransactionPhase::Prepared);
    assert!(tx_three.decide_commit(310).is_ok());
    assert!(tx_three.mark_committed(320).is_ok());
}

#[test]
fn test_coordinator_crash_recovery_semantics() {
    // Scenario A: Coordinator crashed while preparing -> incomplete write must safely abort!
    let mut tx_in_flight = ReplicationTransaction::new(
        "tx-crash-1".into(),
        "repo-1".into(),
        1,
        2,
        "node-1".into(),
        1,
        DurabilityPolicy::TwoCopies,
        vec!["node-1".into(), "node-2".into()],
    );
    tx_in_flight.start_preparing().unwrap();
    tx_in_flight.acknowledge_prepared("node-1").unwrap();
    assert_eq!(tx_in_flight.phase, TransactionPhase::Preparing);

    let recovered_phase = tx_in_flight.recover_on_restart(500);
    assert_eq!(recovered_phase, TransactionPhase::Aborted);
    assert_eq!(tx_in_flight.phase, TransactionPhase::Aborted);

    // Scenario B: Coordinator reached CommitDecided before crash -> must complete commit!
    let mut tx_decided = ReplicationTransaction::new(
        "tx-crash-2".into(),
        "repo-1".into(),
        1,
        2,
        "node-1".into(),
        1,
        DurabilityPolicy::TwoCopies,
        vec!["node-1".into(), "node-2".into()],
    );
    tx_decided.start_preparing().unwrap();
    tx_decided.acknowledge_prepared("node-1").unwrap();
    tx_decided.acknowledge_prepared("node-2").unwrap();
    tx_decided.decide_commit(600).unwrap();
    assert_eq!(tx_decided.phase, TransactionPhase::CommitDecided);

    let recovered_phase = tx_decided.recover_on_restart(700);
    assert_eq!(recovered_phase, TransactionPhase::Committed);
    assert_eq!(tx_decided.phase, TransactionPhase::Committed);
}
