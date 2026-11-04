use spine_core::{AnnotatedTag, Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_annotated_tag, serialize_commit, serialize_tree};
use spine_migration::archive_security::{
    extract_zip_secure, validate_and_sanitize_archive_path, ArchiveSecurityError, ExtractionLimits,
};
use spine_migration::engine::{MigrationEngine, MigrationSourceKind};
use spine_migration::fast_stream::{
    parse_fast_import_stream, write_fast_command, FastBlob, FastCommand, FastCommit, FastFileOp,
    FastReset, FastTag,
};
use spine_migration::git_exporter::GitExporter;
use spine_migration::git_format::{
    build_pack_and_index, compare_git_tree_entries, parse_git_commit, parse_git_tag,
    parse_git_tree, parse_loose_object, parse_packfile, read_git_refs, serialize_git_commit,
    serialize_git_tag, serialize_git_tree, serialize_loose_object, write_git_refs,
    GitAnnotatedTag, GitCommit, GitHead, GitObjectType, GitOid, GitRefMap, GitTreeEntry,
    PackObjectToBuild,
};
use spine_migration::git_importer::GitImporter;
use spine_migration::lfs_interop::GitLfsPointer;
use spine_migration::platform::attribution::AttributionPolicy;
use spine_migration::platform::models::{
    parse_github_metadata_dir, parse_gitlab_metadata_dir, PlatformIssue, PlatformPullRequest,
    PlatformUser,
};
use spine_migration::platform::sanitization::{is_workflow_file, scan_for_secrets};
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::process::Command;
use tempfile::tempdir;
use zip::write::FileOptions;
use zip::ZipWriter;

#[test]
fn test_git_oid_and_loose_blob_roundtrip() {
    let payload = b"Hello from Spine to Git interop!\n";
    let (compressed, oid) =
        serialize_loose_object(GitObjectType::Blob, payload).expect("Serialization failed");

    // Compute expected Git SHA-1 manually
    let expected_oid = GitOid::compute_object("blob", payload);
    assert_eq!(oid, expected_oid);

    // Decompress and verify
    let (obj_type, decompressed, computed_oid) =
        parse_loose_object(&compressed, Some(oid)).expect("Parsing failed");

    assert_eq!(obj_type, GitObjectType::Blob);
    assert_eq!(decompressed, payload);
    assert_eq!(computed_oid, oid);
}

#[test]
fn test_git_tree_serialization_and_sorting() {
    let blob_oid1 = GitOid::compute(b"blob1");
    let blob_oid2 = GitOid::compute(b"blob2");
    let tree_oid = GitOid::compute(b"subtree");

    let entries = vec![
        GitTreeEntry {
            mode: 0o100644,
            name: "README.md".to_string(),
            oid: blob_oid1,
        },
        GitTreeEntry {
            mode: 0o040000,
            name: "src".to_string(),
            oid: tree_oid,
        },
        GitTreeEntry {
            mode: 0o100755,
            name: "build.sh".to_string(),
            oid: blob_oid2,
        },
    ];

    let serialized = serialize_git_tree(entries.clone());
    let parsed = parse_git_tree(&serialized).expect("Tree parsing failed");

    assert_eq!(parsed.len(), 3);
    // In Git sorting, directories are sorted as if ending in '/', so "build.sh" < "src/" < "README.md"
    let names: Vec<String> = parsed.iter().map(|e| e.name.clone()).collect();
    assert!(names.contains(&"README.md".to_string()));
    assert!(names.contains(&"src".to_string()));
    assert!(names.contains(&"build.sh".to_string()));
}

#[test]
fn test_git_commit_and_tag_roundtrip() {
    let tree_oid = GitOid::compute(b"test_tree");
    let parent_oid = GitOid::compute(b"parent_commit");

    let commit = GitCommit {
        tree: tree_oid,
        parents: vec![parent_oid],
        author: "Alice <alice@example.com> 1700000000 +0000".to_string(),
        committer: "Bob <bob@example.com> 1700000100 +0000".to_string(),
        gpgsig: None,
        message: "feat: batch 15 git interoperability\n\nFull details here.".to_string(),
    };

    let serialized = serialize_git_commit(&commit);
    let parsed = parse_git_commit(&serialized).expect("Commit parsing failed");

    assert_eq!(parsed.tree, tree_oid);
    assert_eq!(parsed.parents, vec![parent_oid]);
    assert_eq!(parsed.message, commit.message);

    let (author_name, author_email, ts) = parsed.parse_author();
    assert_eq!(author_name, "Alice");
    assert_eq!(author_email, "alice@example.com");
    assert_eq!(ts, 1700000000);

    // Annotated Tag test
    let commit_oid = GitOid::compute_object("commit", &serialized);
    let tag = GitAnnotatedTag {
        object: commit_oid,
        target_type: "commit".to_string(),
        tag_name: "v1.0.0".to_string(),
        tagger: "Tagger <tagger@example.com> 1700000200 +0000".to_string(),
        message: "Release v1.0.0".to_string(),
    };

    let tag_serialized = serialize_git_tag(&tag);
    let parsed_tag = parse_git_tag(&tag_serialized).expect("Tag parsing failed");

    assert_eq!(parsed_tag.object, commit_oid);
    assert_eq!(parsed_tag.tag_name, "v1.0.0");
    assert_eq!(parsed_tag.message, "Release v1.0.0");
}

#[test]
fn test_git_pack_builder_and_delta_resolution() {
    let blob1_payload = b"Original baseline content line 1\nLine 2\nLine 3\n".to_vec();
    let blob2_payload = b"Original baseline content line 1\nModified Line 2\nLine 3\n".to_vec();

    let oid1 = GitOid::compute_object("blob", &blob1_payload);
    let oid2 = GitOid::compute_object("blob", &blob2_payload);

    let objects = vec![
        PackObjectToBuild {
            obj_type: GitObjectType::Blob,
            payload: blob1_payload.clone(),
            oid: oid1,
        },
        PackObjectToBuild {
            obj_type: GitObjectType::Blob,
            payload: blob2_payload.clone(),
            oid: oid2,
        },
    ];

    let (pack_data, idx_data, pack_oid) =
        build_pack_and_index(objects).expect("Pack builder failed");

    assert!(pack_data.starts_with(b"PACK"));
    assert!(idx_data.starts_with(b"\xFFtOc"));
    assert_ne!(pack_oid, GitOid::ZERO);

    // Parse the packfile back and ensure both objects are cleanly reconstructed
    let parsed_objects = parse_packfile(&pack_data).expect("Packfile parsing failed");
    assert_eq!(parsed_objects.len(), 2);

    let oids: Vec<GitOid> = parsed_objects.iter().map(|o| o.oid).collect();
    assert!(oids.contains(&oid1));
    assert!(oids.contains(&oid2));
}

#[test]
fn test_fast_import_and_export_stream_roundtrip() {
    let fast_blob = FastBlob {
        mark: Some(1),
        data: b"fn main() { println!(\"Fast stream\"); }\n".to_vec(),
    };
    let fast_commit = FastCommit {
        target_ref: "refs/heads/main".to_string(),
        mark: Some(2),
        author: Some("Author <author@example.com> 1600000000 +0000".to_string()),
        committer: "Committer <committer@example.com> 1600000000 +0000".to_string(),
        message: "Initial fast-import commit\n".to_string(),
        from: None,
        merges: Vec::new(),
        file_ops: vec![FastFileOp::Modify {
            mode: "100644".to_string(),
            dataref: ":1".to_string(),
            path: "src/main.rs".to_string(),
        }],
    };
    let fast_tag = FastTag {
        tag_name: "v0.1".to_string(),
        from: ":2".to_string(),
        tagger: Some("Tagger <tagger@example.com> 1600000100 +0000".to_string()),
        message: "Tag v0.1\n".to_string(),
    };
    let fast_reset = FastReset {
        target_ref: "refs/heads/feature".to_string(),
        from: Some(":2".to_string()),
    };

    let mut stream_buf = Vec::new();
    write_fast_command(&mut stream_buf, &FastCommand::Blob(fast_blob.clone())).unwrap();
    write_fast_command(&mut stream_buf, &FastCommand::Commit(fast_commit.clone())).unwrap();
    write_fast_command(&mut stream_buf, &FastCommand::Tag(fast_tag.clone())).unwrap();
    write_fast_command(&mut stream_buf, &FastCommand::Reset(fast_reset.clone())).unwrap();

    let commands =
        parse_fast_import_stream(Cursor::new(&stream_buf)).expect("Fast stream parse failed");
    assert_eq!(commands.len(), 4);

    match &commands[0] {
        FastCommand::Blob(b) => {
            assert_eq!(b.mark, Some(1));
            assert_eq!(b.data, fast_blob.data);
        }
        _ => panic!("Expected blob command"),
    }

    match &commands[1] {
        FastCommand::Commit(c) => {
            assert_eq!(c.target_ref, "refs/heads/main");
            assert_eq!(c.file_ops.len(), 1);
        }
        _ => panic!("Expected commit command"),
    }
}

#[test]
fn test_lfs_pointer_parsing_and_detection() {
    let fake_data = b"Some large dataset or binary payload for LFS testing";
    let pointer = GitLfsPointer::from_data(fake_data);
    let serialized = pointer.serialize();

    assert!(GitLfsPointer::is_lfs_pointer(&serialized));
    let parsed = GitLfsPointer::parse(&serialized).expect("Pointer parse failed");

    assert_eq!(parsed.oid_sha256, pointer.oid_sha256);
    assert_eq!(parsed.size_bytes, fake_data.len() as u64);

    // Negative check on random binary
    assert!(!GitLfsPointer::is_lfs_pointer(b"\x00\x01\x02\x03\x04\x05"));
}

#[test]
fn test_archive_security_zip_slip_and_bomb_protection() {
    let tmp = tempdir().unwrap();
    let zip_path = tmp.path().join("malicious.zip");

    // 1. Create a zip with path traversal (Zip Slip)
    {
        let file = File::create(&zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = FileOptions::default();
        zip.start_file("../../outside.txt", options).unwrap();
        zip.write_all(b"malicious payload").unwrap();
        zip.finish().unwrap();
    }

    let extract_dir = tmp.path().join("extracted");
    let limits = ExtractionLimits::default();
    let res = extract_zip_secure(&zip_path, &extract_dir, &limits);
    assert!(res.is_err());
    match res {
        Err(ArchiveSecurityError::ZipSlipAttempt(_)) => {}
        other => panic!("Expected ZipSlipAttempt error, got: {:?}", other),
    }

    // 2. Windows reserved name rejection
    let res_name = validate_and_sanitize_archive_path("nested/CON.txt");
    assert!(res_name.is_err());
    match res_name {
        Err(ArchiveSecurityError::WindowsReservedName(_)) => {}
        other => panic!("Expected WindowsReservedName error, got: {:?}", other),
    }

    // 3. Decompression bomb byte limit
    let safe_zip_path = tmp.path().join("safe.zip");
    {
        let file = File::create(&safe_zip_path).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = FileOptions::default();
        zip.start_file("large.txt", options).unwrap();
        zip.write_all(&vec![b'A'; 2000]).unwrap();
        zip.finish().unwrap();
    }

    let strict_limits = ExtractionLimits {
        max_total_bytes: 500, // lower than 2000
        max_entries: 10,
        max_ratio: 100,
    };
    let bomb_res = extract_zip_secure(&safe_zip_path, &extract_dir, &strict_limits);
    assert!(bomb_res.is_err());
    match bomb_res {
        Err(ArchiveSecurityError::TotalBytesExceeded { .. }) => {}
        other => panic!("Expected TotalBytesExceeded error, got: {:?}", other),
    }
}

#[test]
fn test_platform_metadata_and_safe_attribution() {
    let tmp = tempdir().unwrap();
    let meta_dir = tmp.path();

    // Create synthetic repository.json
    let repo_json = r#"{
        "name": "migrated-sample",
        "description": "Migrated from GitHub",
        "default_branch": "main",
        "private": true,
        "topics": ["rust", "vcs"]
    }"#;
    fs::write(meta_dir.join("repository.json"), repo_json).unwrap();

    // Create synthetic issues.json
    let issues_json = r#"[{
        "number": 1,
        "title": "First issue",
        "body": "Issue body",
        "state": "open",
        "author": { "username": "ext_alice", "email": "alice@external.org" },
        "labels": ["bug"],
        "milestone": null,
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": null,
        "closed_at": null,
        "comments": []
    }]"#;
    fs::write(meta_dir.join("issues.json"), issues_json).unwrap();

    let data = parse_github_metadata_dir(meta_dir).expect("Metadata parse failed");
    assert_eq!(data.name, "migrated-sample");
    assert_eq!(data.issues.len(), 1);
    assert_eq!(data.issues[0].title, "First issue");

    // Attribution policy test
    let mut policy = AttributionPolicy::default();
    policy.create_tombstone_placeholders = true;

    let resolved = policy.resolve_user(&data.issues[0].author);
    assert!(resolved.is_tombstone);
    assert_eq!(resolved.spine_username, "ext_alice-migrated");

    // Workflow file detection and secret scan
    assert!(is_workflow_file(".github/workflows/ci.yml"));
    assert!(is_workflow_file(".gitlab-ci.yml"));
    assert!(!is_workflow_file("src/main.rs"));

    let code_with_secret = "let token = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\";\n";
    let detections = scan_for_secrets("config.rs", code_with_secret);
    assert_eq!(detections.len(), 1);
    assert_eq!(detections[0].secret_kind, "GitHub Personal Access Token");
}

#[test]
fn test_roundtrip_spine_to_git_and_official_oracle_verification() {
    let tmp = tempdir().unwrap();
    let spine_repo = tmp.path().join("spine_repo");
    let exported_git = tmp.path().join("exported_git");

    // 1. Create a native Spine repository
    let spine_dir = spine_repo.join(".spine");
    fs::create_dir_all(&spine_dir).unwrap();
    let store =
        FileSystemObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).expect("Store init failed");

    // Write a blob
    let blob_content = b"Content generated natively in Spine!\n";
    let blob_id = store
        .write_object(ObjectType::Blob, blob_content)
        .expect("Write blob failed");

    // Write a tree
    let tree_entries = vec![TreeEntry {
        mode: FileMode::Regular,
        name: "hello.txt".to_string(),
        object_id: blob_id,
    }];
    let tree_bytes = serialize_tree(&tree_entries);
    let tree_id = store
        .write_object(ObjectType::Tree, &tree_bytes)
        .expect("Write tree failed");

    // Write a commit
    let commit = Commit::new_simple(
        tree_id,
        Vec::new(),
        "Spine Engineer".to_string(),
        "engineer@spine.local".to_string(),
        1700000000 * 1_000_000_000,
        "feat: native spine initial commit\n".to_string(),
    );
    let commit_bytes = serialize_commit(&commit);
    let commit_id = store
        .write_object(ObjectType::Commit, &commit_bytes)
        .expect("Write commit failed");

    // Create refs/heads/main and HEAD
    let heads_dir = spine_dir.join("refs").join("heads");
    fs::create_dir_all(&heads_dir).unwrap();
    fs::write(heads_dir.join("main"), format!("{}\n", commit_id)).unwrap();
    fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    // 2. Export Spine repository to standard Git repository format (non-bare)
    let mut exporter = GitExporter::new(&spine_repo, &exported_git, false)
        .expect("Exporter initialization failed");
    let stats = exporter.execute_export().expect("Export failed");

    assert_eq!(stats.commits_count, 1);
    assert_eq!(stats.trees_count, 1);
    assert_eq!(stats.blobs_count, 1);
    assert_eq!(stats.branches_count, 1);

    // Verify target .git exists and files are checked out
    assert!(exported_git.join(".git").exists());
    assert!(exported_git.join("hello.txt").exists());
    let checked_out = fs::read(exported_git.join("hello.txt")).unwrap();
    assert_eq!(checked_out, blob_content);

    // 3. External Test Oracle Verification: run official Git CLI on the exported repository
    // Host environment has `git` version 2.53.0.windows.1 available
    let git_check = Command::new("git").arg("--version").output();
    if let Ok(output) = git_check {
        if output.status.success() {
            // A. Run `git fsck --full` -> must pass with zero corruption
            let fsck_output = Command::new("git")
                .arg("-C")
                .arg(&exported_git)
                .args(["fsck", "--full"])
                .output()
                .expect("Failed to execute git fsck");

            assert!(
                fsck_output.status.success(),
                "git fsck failed: stdout={}, stderr={}",
                String::from_utf8_lossy(&fsck_output.stdout),
                String::from_utf8_lossy(&fsck_output.stderr)
            );

            // B. Run `git log` -> must show our commit message and author
            let log_output = Command::new("git")
                .arg("-C")
                .arg(&exported_git)
                .args(["log", "-1", "--format=%an <%ae> %s"])
                .output()
                .expect("Failed to execute git log");

            assert!(log_output.status.success());
            let log_str = String::from_utf8_lossy(&log_output.stdout);
            assert!(log_str.contains("Spine Engineer <engineer@spine.local>"));
            assert!(log_str.contains("feat: native spine initial commit"));

            // C. Run `git cat-file -p HEAD`
            let cat_output = Command::new("git")
                .arg("-C")
                .arg(&exported_git)
                .args(["cat-file", "-p", "HEAD"])
                .output()
                .expect("Failed to execute git cat-file");

            assert!(cat_output.status.success());
            let cat_str = String::from_utf8_lossy(&cat_output.stdout);
            assert!(cat_str.contains("tree "));
            assert!(cat_str.contains("author Spine Engineer"));
        }
    }

    // 4. Roundtrip back: Import the exported Git repository into a brand new Spine repo!
    let roundtrip_spine = tmp.path().join("roundtrip_spine");
    let rt_spine_dir = roundtrip_spine.join(".spine");
    let mut importer = GitImporter::new(&exported_git, &rt_spine_dir, Some(roundtrip_spine.clone()))
        .expect("Importer init failed");
    importer.load_git_objects().expect("Load git objects failed");
    let imp_stats = importer.execute_import().expect("Import execute failed");

    assert_eq!(imp_stats.commits_count, 1);
    assert_eq!(imp_stats.trees_count, 1);
    assert_eq!(imp_stats.blobs_count, 1);

    // Verify checked-out file in the roundtrip repo
    assert!(roundtrip_spine.join("hello.txt").exists());
    let rt_content = fs::read(roundtrip_spine.join("hello.txt")).unwrap();
    assert_eq!(rt_content, blob_content);
}
