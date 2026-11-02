use spine_cluster::repair::{AntiEntropy, RepairJob, RepairStatus};
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore};
use tempfile::tempdir;

#[test]
fn test_anti_entropy_pack_diff_and_repair_generation() {
    let source_packs = vec![
        "pack-100".to_string(),
        "pack-101".to_string(),
        "pack-102".to_string(),
        "pack-103".to_string(),
    ];
    let replica_packs = vec!["pack-100".to_string(), "pack-102".to_string()];

    let missing = AntiEntropy::diff_packs(&source_packs, &replica_packs);
    assert_eq!(
        missing,
        vec!["pack-101".to_string(), "pack-103".to_string()]
    );

    let dir = tempdir().unwrap();
    let store = FileBackedCoordinatorStore::new_file_backed(dir.path().join("coord.json")).unwrap();

    let job = RepairJob::new(
        "job-repair-01".into(),
        "repo-game-engine".into(),
        "replica-node-2".into(),
        "primary-node-1".into(),
        missing.clone(),
        1000,
    );

    assert_eq!(job.status, RepairStatus::Queued);
    assert_eq!(job.missing_packs.len(), 2);

    store.create_repair_job(job).unwrap();
    let jobs = store.list_repair_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_node_id, "replica-node-2");
    assert_eq!(jobs[0].missing_packs, missing);
}
