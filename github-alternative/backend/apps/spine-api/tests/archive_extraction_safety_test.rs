use spine_api::backup::safe_unpack_archive;
use std::fs;
use tar::{Archive, Builder, EntryType, Header};
use tempfile::tempdir;

#[test]
fn test_safe_archive_unpack_valid_files_and_directories() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);

        // Valid directory
        let mut dir_header = Header::new_gnu();
        dir_header.set_entry_type(EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder
            .append_data(&mut dir_header, "subdir", &[][..])
            .unwrap();

        // Valid file inside directory
        let mut file_header = Header::new_gnu();
        file_header.set_entry_type(EntryType::Regular);
        let content = b"hello world secure archive";
        file_header.set_size(content.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder
            .append_data(&mut file_header, "subdir/hello.txt", &content[..])
            .unwrap();

        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let count =
        safe_unpack_archive(archive, &staged_dir).expect("Valid archive should unpack cleanly");
    assert_eq!(count, 1);
    assert_eq!(
        fs::read_to_string(staged_dir.join("subdir/hello.txt")).unwrap(),
        "hello world secure archive"
    );
}

#[test]
fn test_archive_rejects_symlink_entry() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_link_name("../../../outside_file").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "evil_symlink", &[][..])
            .unwrap();
        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let err = safe_unpack_archive(archive, &staged_dir)
        .expect_err("Symlink entry must be strictly rejected");
    assert!(err.to_string().contains("Prohibited unsafe entry type"));
}

#[test]
fn test_archive_rejects_hardlink_entry() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Link);
        header.set_size(0);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "evil_hardlink", &[][..])
            .unwrap();
        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let err = safe_unpack_archive(archive, &staged_dir)
        .expect_err("Hardlink entry must be strictly rejected");
    assert!(err.to_string().contains("Prohibited unsafe entry type"));
}

#[test]
fn test_archive_rejects_parent_traversal() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        let data = b"ATTACK";
        header.set_size(data.len() as u64);
        let name_bytes = b"../escaped.txt\0";
        header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_cksum();
        builder.append(&header, &data[..]).unwrap();
        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let err = safe_unpack_archive(archive, &staged_dir)
        .expect_err("Parent traversal path must be rejected");
    assert!(err.to_string().contains("Unsafe path traversal component"));
}

#[test]
fn test_archive_rejects_absolute_path() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Regular);
        let data = b"ATTACK";
        header.set_size(data.len() as u64);
        let name_bytes = b"/etc/shadow\0";
        header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_cksum();
        builder.append(&header, &data[..]).unwrap();
        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let err =
        safe_unpack_archive(archive, &staged_dir).expect_err("Absolute path must be rejected");
    assert!(err.to_string().contains("Unsafe path traversal component"));
}

#[test]
fn test_archive_rejects_duplicate_path_overwrite() {
    let tmp = tempdir().unwrap();
    let staged_dir = tmp.path().join("staged");
    fs::create_dir_all(&staged_dir).unwrap();

    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        let mut header1 = Header::new_gnu();
        header1.set_entry_type(EntryType::Regular);
        let data1 = b"original";
        header1.set_size(data1.len() as u64);
        header1.set_cksum();
        builder
            .append_data(&mut header1, "file.txt", &data1[..])
            .unwrap();

        let mut header2 = Header::new_gnu();
        header2.set_entry_type(EntryType::Regular);
        let data2 = b"duplicate replacement";
        header2.set_size(data2.len() as u64);
        header2.set_cksum();
        builder
            .append_data(&mut header2, "file.txt", &data2[..])
            .unwrap();

        builder.finish().unwrap();
    }

    let archive = Archive::new(&tar_bytes[..]);
    let err = safe_unpack_archive(archive, &staged_dir)
        .expect_err("Duplicate entry path must be rejected");
    assert!(err.to_string().contains("Duplicate entry path detected"));
}
