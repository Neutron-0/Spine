use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use spine_core::{FileMode, HashAlgorithm, ObjectId, ObjectType};
use spine_index::{Index, IndexEntry, StageStatus};
use spine_object_store::{MemoryObjectStore, ObjectStore};
use spine_worktree::write_index_to_tree;

fn bench_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashing_throughput");
    for size in [1024, 65536, 1048576].iter() {
        let payload = vec![0x42u8; *size];
        group.throughput(Throughput::Bytes(*size as u64));

        group.bench_with_input(BenchmarkId::new("BLAKE3", size), size, |b, _| {
            b.iter(|| {
                black_box(ObjectId::compute(
                    HashAlgorithm::Blake3,
                    ObjectType::Blob,
                    &payload,
                ))
            });
        });

        group.bench_with_input(BenchmarkId::new("SHA-256", size), size, |b, _| {
            b.iter(|| {
                black_box(ObjectId::compute(
                    HashAlgorithm::Sha256,
                    ObjectType::Blob,
                    &payload,
                ))
            });
        });
    }
    group.finish();
}

fn bench_object_store_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("object_store_memory");
    let payload = vec![0xa5u8; 4096];

    group.bench_function("write_object_4kb", |b| {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);
        b.iter(|| black_box(store.write_object(ObjectType::Blob, &payload).unwrap()));
    });

    let store = MemoryObjectStore::new(HashAlgorithm::Blake3);
    let oid = store.write_object(ObjectType::Blob, &payload).unwrap();

    group.bench_function("read_object_4kb", |b| {
        b.iter(|| black_box(store.read_object(&oid).unwrap()));
    });

    group.finish();
}

fn bench_index_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("index_serialization");
    let mut index = Index::new();
    for i in 0..1000 {
        index.add_or_update(IndexEntry {
            path: format!("src/module_{}/sub_{}/file_{}.rs", i / 100, i % 100, i),
            mode: FileMode::Regular,
            file_size: 4096,
            mtime_secs: 1730000000 + i as i64,
            mtime_nanos: 123456,
            inode: i as u64,
            object_id: ObjectId::from_bytes([i as u8; 32]),
            stage_status: StageStatus::Clean,
        });
    }

    let bytes = index.serialize();
    group.throughput(Throughput::Elements(1000));

    group.bench_function("serialize_1000_entries", |b| {
        b.iter(|| black_box(index.serialize()));
    });

    group.bench_function("deserialize_1000_entries", |b| {
        b.iter(|| black_box(Index::deserialize(&bytes).unwrap()));
    });

    group.finish();
}

fn bench_tree_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("merkle_tree_construction");
    let store = MemoryObjectStore::new(HashAlgorithm::Blake3);
    let mut index = Index::new();
    for i in 0..500 {
        index.add_or_update(IndexEntry {
            path: format!("crates/pkg_{}/src/file_{}.rs", i / 50, i),
            mode: FileMode::Regular,
            file_size: 2048,
            mtime_secs: 1730000000,
            mtime_nanos: 0,
            inode: i as u64,
            object_id: ObjectId::from_bytes([(i % 256) as u8; 32]),
            stage_status: StageStatus::Staged,
        });
    }

    group.throughput(Throughput::Elements(500));
    group.bench_function("write_tree_500_entries", |b| {
        b.iter(|| black_box(write_index_to_tree(&index, &store).unwrap()));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_hashing,
    bench_object_store_memory,
    bench_index_serialization,
    bench_tree_builder
);
criterion_main!(benches);
