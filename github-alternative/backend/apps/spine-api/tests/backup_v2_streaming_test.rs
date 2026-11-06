use spine_api::backup::{v2, ConsistencyBarrier};
use std::io::Cursor;
use std::time::Duration;

#[test]
fn test_spnebak_v2_streaming_chunked_roundtrip() {
    let passphrase = "correct_master_passphrase_for_testing_1234!";
    let key_version = 1u16;

    // Create 2.5 MiB of pseudorandom deterministic data (across 3 chunks of 1 MiB chunk size)
    let total_size = (2.5 * 1024.0 * 1024.0) as usize;
    let mut original_data = vec![0u8; total_size];
    for (i, b) in original_data.iter_mut().enumerate() {
        *b = ((i * 31 + 7) % 256) as u8;
    }

    let input_cursor = Cursor::new(&original_data);
    let mut encrypted_output = Vec::new();

    // 1. Encrypt stream
    let chunk_size = 1024 * 1024; // 1 MiB
    let enc_summary = v2::encrypt_stream(
        input_cursor,
        &mut encrypted_output,
        passphrase,
        key_version,
        chunk_size,
    )
    .expect("encrypt_stream failed");

    assert_eq!(enc_summary.total_chunks, 3); // 2 full chunks + 1 partial chunk
    assert_eq!(enc_summary.total_plaintext_bytes, total_size as u64);

    // 2. Decrypt stream
    let enc_cursor = Cursor::new(&encrypted_output);
    let mut decrypted_output = Vec::new();

    let dec_summary = v2::decrypt_stream(enc_cursor, &mut decrypted_output, passphrase)
        .expect("decrypt_stream failed");

    assert_eq!(dec_summary.total_chunks, 3);
    assert_eq!(dec_summary.total_plaintext_bytes, total_size as u64);
    assert_eq!(enc_summary.stream_sha256, dec_summary.stream_sha256);
    assert_eq!(original_data, decrypted_output);
}

#[test]
fn test_spnebak_v2_tampering_and_authentication_rejection() {
    let passphrase = "secure_tamper_detection_passphrase_xyz";
    let original_data = b"Critical infrastructure backup payload that cannot be tampered with.";

    let input = Cursor::new(original_data);
    let mut encrypted_bytes = Vec::new();
    v2::encrypt_stream(input, &mut encrypted_bytes, passphrase, 1, 1024 * 1024)
        .expect("encryption failed");

    // 1. Decrypt with wrong passphrase -> must fail
    let mut dec_out = Vec::new();
    let wrong_pass_result =
        v2::decrypt_stream(Cursor::new(&encrypted_bytes), &mut dec_out, "wrong_pass");
    assert!(
        wrong_pass_result.is_err(),
        "Decryption with wrong passphrase must fail authentication"
    );

    // 2. Tamper with ciphertext payload byte
    let mut tampered_bytes = encrypted_bytes.clone();
    let header_len = v2::HEADER_V2_LEN;
    // Tamper byte inside the ciphertext payload
    tampered_bytes[header_len + 20] ^= 0xFF;

    let mut tampered_out = Vec::new();
    let tamper_result =
        v2::decrypt_stream(Cursor::new(&tampered_bytes), &mut tampered_out, passphrase);
    assert!(
        tamper_result.is_err(),
        "Decryption of modified ciphertext must fail AEAD tag check"
    );
}

#[test]
fn test_spnebak_v2_streaming_key_rotation() {
    let old_passphrase = "old_legacy_secret_key_v1";
    let new_passphrase = "new_hardened_secret_key_v2";

    let plaintext =
        b"Database snapshot and git repositories to be re-keyed safely in streaming memory.";
    let mut v1_archive = Vec::new();
    v2::encrypt_stream(
        Cursor::new(plaintext),
        &mut v1_archive,
        old_passphrase,
        1,
        64 * 1024,
    )
    .expect("encrypt failed");

    // Rotate stream in-memory without decrypting to disk
    let mut v2_rotated_archive = Vec::new();
    v2::rotate_stream(
        Cursor::new(&v1_archive),
        &mut v2_rotated_archive,
        old_passphrase,
        new_passphrase,
        2,
    )
    .expect("rotate_stream failed");

    // Verify old passphrase no longer decrypts rotated archive
    let mut fail_out = Vec::new();
    assert!(v2::decrypt_stream(
        Cursor::new(&v2_rotated_archive),
        &mut fail_out,
        old_passphrase
    )
    .is_err());

    // Verify new passphrase successfully decrypts rotated archive
    let mut success_out = Vec::new();
    let summary = v2::decrypt_stream(
        Cursor::new(&v2_rotated_archive),
        &mut success_out,
        new_passphrase,
    )
    .expect("decrypt rotated archive failed");

    assert_eq!(&success_out, plaintext);
    assert_eq!(summary.total_plaintext_bytes, plaintext.len() as u64);
}

#[tokio::test]
async fn test_consistency_barrier_draining_window() {
    let barrier = ConsistencyBarrier::new();
    assert!(!barrier.is_maintenance_active());

    {
        let _guard = barrier.enter_maintenance(Duration::from_millis(50)).await;
        assert!(
            barrier.is_maintenance_active(),
            "Maintenance mode must be active during guard lifetime"
        );
    }

    assert!(
        !barrier.is_maintenance_active(),
        "Maintenance mode must be inactive once guard is dropped"
    );
}

#[test]
fn test_spnebak_v2_rejects_trailing_bytes_after_final_manifest() {
    let passphrase = "test_passphrase_for_trailing_bytes_detection";
    let data = b"Valid payload that must not have trailing data appended.";

    let mut enc = Vec::new();
    v2::encrypt_stream(Cursor::new(data), &mut enc, passphrase, 1, 64 * 1024).unwrap();

    // Append 5 malicious trailing bytes after final manifest chunk
    enc.extend_from_slice(b"EXTRA");

    let mut dec = Vec::new();
    let res = v2::decrypt_stream(Cursor::new(&enc), &mut dec, passphrase);
    assert!(res.is_err(), "Must reject stream with trailing data");
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("trailing data"),
        "Error message must specify trailing data: {}",
        err
    );
}

#[test]
fn test_spnebak_v2_header_bounds_validation() {
    // 1. Valid header
    let valid_header = v2::BackupHeaderV2 {
        key_version: 1,
        pbkdf2_iterations: 100_000,
        salt: [0u8; v2::SALT_LEN],
        archive_id: [0u8; 16],
        nonce_prefix: [0u8; v2::NONCE_PREFIX_LEN],
        chunk_size: 1024 * 1024,
    };
    let bytes = valid_header.to_bytes();
    assert!(v2::BackupHeaderV2::from_bytes(&bytes).is_ok());

    // 2. PBKDF2 iterations too low (< 10,000)
    let mut low_iter_header = valid_header.clone();
    low_iter_header.pbkdf2_iterations = 5_000;
    let bytes = low_iter_header.to_bytes();
    let res = v2::BackupHeaderV2::from_bytes(&bytes);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("PBKDF2 iterations"));

    // 3. PBKDF2 iterations too high (> 1,000,000)
    let mut high_iter_header = valid_header.clone();
    high_iter_header.pbkdf2_iterations = 2_000_000;
    let bytes = high_iter_header.to_bytes();
    let res = v2::BackupHeaderV2::from_bytes(&bytes);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("PBKDF2 iterations"));

    // 4. Chunk size too small (< 4096)
    let mut small_chunk_header = valid_header.clone();
    small_chunk_header.chunk_size = 2048;
    let bytes = small_chunk_header.to_bytes();
    let res = v2::BackupHeaderV2::from_bytes(&bytes);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("chunk size"));

    // 5. Chunk size too large (> 16 MiB)
    let mut large_chunk_header = valid_header.clone();
    large_chunk_header.chunk_size = 32 * 1024 * 1024;
    let bytes = large_chunk_header.to_bytes();
    let res = v2::BackupHeaderV2::from_bytes(&bytes);
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("chunk size"));
}
