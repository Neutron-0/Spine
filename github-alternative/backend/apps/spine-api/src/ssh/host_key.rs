use anyhow::{Context, Result};
use rand::RngCore;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{LineEnding, PrivateKey};
use std::fs;
use std::path::Path;

pub fn resolve_ssh_dir(storage_root: &Path) -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("SSH_DIR") {
        let p = std::path::PathBuf::from(dir.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    storage_root.join("ssh")
}

pub fn load_or_generate_host_key(storage_root: &Path) -> Result<PrivateKey> {
    let ssh_dir = resolve_ssh_dir(storage_root);
    if !ssh_dir.exists() {
        fs::create_dir_all(&ssh_dir)
            .with_context(|| format!("Failed to create SSH host key dir: {:?}", ssh_dir))?;
    }

    let key_path = ssh_dir.join("host_key_ed25519");
    if key_path.exists() {
        let pem_str = fs::read_to_string(&key_path)
            .with_context(|| format!("Failed to read host key from {:?}", key_path))?;
        let key = PrivateKey::from_openssh(&pem_str)
            .with_context(|| format!("Failed to parse OpenSSH host key from {:?}", key_path))?;
        tracing::info!("Loaded SSH host key from {:?}", key_path);
        return Ok(key);
    }

    tracing::info!("Generating new Ed25519 SSH host key at {:?}", key_path);
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let keypair = Ed25519Keypair::from_seed(&seed);
    let key = PrivateKey::from(keypair);

    let pem = key
        .to_openssh(LineEnding::LF)
        .context("Failed to encode OpenSSH private key")?;

    // Atomic write to prevent corrupted keys
    let tmp_path = ssh_dir.join("host_key_ed25519.tmp");
    fs::write(&tmp_path, pem.as_bytes())
        .with_context(|| format!("Failed to write tmp host key to {:?}", tmp_path))?;
    fs::rename(&tmp_path, &key_path)
        .with_context(|| format!("Failed to rename host key into place at {:?}", key_path))?;

    // Also write public key alongside for ease of inspection
    let pub_path = ssh_dir.join("host_key_ed25519.pub");
    if let Ok(pub_str) = key.public_key().to_openssh() {
        let _ = fs::write(&pub_path, pub_str.as_bytes());
    }

    Ok(key)
}
