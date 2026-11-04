use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use tar::Archive as TarArchive;
use thiserror::Error;
use zip::ZipArchive;

pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024; // 1 GB
pub const DEFAULT_MAX_ENTRIES: usize = 50_000;
pub const DEFAULT_MAX_RATIO: u64 = 100;

#[derive(Error, Debug)]
pub enum ArchiveSecurityError {
    #[error("I/O error during archive extraction: {0}")]
    Io(#[from] io::Error),
    #[error("Zip extraction error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Security violation: Zip Slip or path traversal attempt detected in path: {0}")]
    ZipSlipAttempt(String),
    #[error("Security violation: Windows reserved device name forbidden in archive: {0}")]
    WindowsReservedName(String),
    #[error("Security violation: Decompression bomb limit exceeded: extracted {actual} bytes exceeds limit of {limit} bytes")]
    TotalBytesExceeded { actual: u64, limit: u64 },
    #[error("Security violation: Decompression bomb limit exceeded: {actual} entries exceeds limit of {limit}")]
    MaxEntriesExceeded { actual: usize, limit: usize },
    #[error("Security violation: Decompression bomb compression ratio of {ratio}:1 exceeds max permitted ratio of {limit}:1")]
    CompressionRatioExceeded { ratio: u64, limit: u64 },
    #[error("Unsupported archive format for file: {0}")]
    UnsupportedFormat(String),
}

/// Security limits configuration for archive extraction.
#[derive(Debug, Clone)]
pub struct ExtractionLimits {
    pub max_total_bytes: u64,
    pub max_entries: usize,
    pub max_ratio: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_ratio: DEFAULT_MAX_RATIO,
        }
    }
}

/// Sanitizes and validates a relative path inside an archive.
/// Rejects path traversal (`..`), absolute paths, Windows drive specifiers, UNC paths,
/// and Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
pub fn validate_and_sanitize_archive_path(raw_path: &str) -> Result<PathBuf, ArchiveSecurityError> {
    let normalized = raw_path.replace('\\', "/");

    // Check for drive specifiers like "C:" or UNC roots like "//"
    if normalized.contains(':') || normalized.starts_with("//") {
        return Err(ArchiveSecurityError::ZipSlipAttempt(raw_path.to_string()));
    }

    let p = Path::new(&normalized);
    let mut clean_components = Vec::new();

    for comp in p.components() {
        match comp {
            Component::Normal(c) => {
                let s = c.to_string_lossy();
                // Check Windows reserved names
                let stem = Path::new(&*s)
                    .file_stem()
                    .unwrap_or(c)
                    .to_string_lossy()
                    .to_uppercase();
                if is_windows_reserved_device_name(&stem) {
                    return Err(ArchiveSecurityError::WindowsReservedName(s.to_string()));
                }
                clean_components.push(s.to_string());
            }
            Component::ParentDir => {
                return Err(ArchiveSecurityError::ZipSlipAttempt(raw_path.to_string()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ArchiveSecurityError::ZipSlipAttempt(raw_path.to_string()));
            }
            Component::CurDir => {}
        }
    }

    if clean_components.is_empty() {
        return Err(ArchiveSecurityError::ZipSlipAttempt(raw_path.to_string()));
    }

    let mut result = PathBuf::new();
    for c in clean_components {
        result.push(c);
    }

    Ok(result)
}

fn is_windows_reserved_device_name(name: &str) -> bool {
    matches!(
        name,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Safely extracts a ZIP archive into a destination directory with full security checks.
pub fn extract_zip_secure<P: AsRef<Path>, D: AsRef<Path>>(
    zip_path: P,
    dest_dir: D,
    limits: &ExtractionLimits,
) -> Result<usize, ArchiveSecurityError> {
    let file = File::open(zip_path)?;
    let compressed_file_size = file.metadata()?.len().max(1);
    let mut archive = ZipArchive::new(file)?;

    if archive.len() > limits.max_entries {
        return Err(ArchiveSecurityError::MaxEntriesExceeded {
            actual: archive.len(),
            limit: limits.max_entries,
        });
    }

    fs::create_dir_all(&dest_dir)?;
    let canonical_dest = fs::canonicalize(&dest_dir)?;

    let mut total_bytes: u64 = 0;
    let count = archive.len();

    for i in 0..count {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        let clean_rel = validate_and_sanitize_archive_path(&name)?;
        let target_path = canonical_dest.join(&clean_rel);

        // Security assertion: must remain strictly within canonical_dest
        if !target_path.starts_with(&canonical_dest) {
            return Err(ArchiveSecurityError::ZipSlipAttempt(name));
        }

        if entry.is_dir() {
            fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let mut outfile = File::create(&target_path)?;
            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                outfile.write_all(&buf[..n])?;
                total_bytes += n as u64;

                if total_bytes > limits.max_total_bytes {
                    return Err(ArchiveSecurityError::TotalBytesExceeded {
                        actual: total_bytes,
                        limit: limits.max_total_bytes,
                    });
                }
            }
        }
    }

    let ratio = total_bytes / compressed_file_size;
    if ratio > limits.max_ratio {
        return Err(ArchiveSecurityError::CompressionRatioExceeded {
            ratio,
            limit: limits.max_ratio,
        });
    }

    Ok(count)
}

/// Safely extracts a TAR or TAR.GZ archive into a destination directory.
pub fn extract_tar_secure<P: AsRef<Path>, D: AsRef<Path>>(
    tar_path: P,
    dest_dir: D,
    is_gzipped: bool,
    limits: &ExtractionLimits,
) -> Result<usize, ArchiveSecurityError> {
    let file = File::open(&tar_path)?;
    let compressed_file_size = file.metadata()?.len().max(1);

    fs::create_dir_all(&dest_dir)?;
    let canonical_dest = fs::canonicalize(&dest_dir)?;

    let mut entry_count = 0;
    let mut total_bytes: u64 = 0;

    let reader: Box<dyn Read> = if is_gzipped {
        Box::new(GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    let mut archive = TarArchive::new(reader);

    for entry in archive.entries()? {
        let mut entry = entry?;
        entry_count += 1;

        if entry_count > limits.max_entries {
            return Err(ArchiveSecurityError::MaxEntriesExceeded {
                actual: entry_count,
                limit: limits.max_entries,
            });
        }

        let path_bytes = entry.path_bytes();
        let name_str = String::from_utf8_lossy(&path_bytes).to_string();

        let clean_rel = validate_and_sanitize_archive_path(&name_str)?;
        let target_path = canonical_dest.join(&clean_rel);

        if !target_path.starts_with(&canonical_dest) {
            return Err(ArchiveSecurityError::ZipSlipAttempt(name_str));
        }

        let header = entry.header();
        if header.entry_type().is_dir() {
            fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let mut outfile = File::create(&target_path)?;
            let mut buf = [0u8; 8192];
            loop {
                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                outfile.write_all(&buf[..n])?;
                total_bytes += n as u64;

                if total_bytes > limits.max_total_bytes {
                    return Err(ArchiveSecurityError::TotalBytesExceeded {
                        actual: total_bytes,
                        limit: limits.max_total_bytes,
                    });
                }
            }
        }
    }

    let ratio = total_bytes / compressed_file_size;
    if ratio > limits.max_ratio {
        return Err(ArchiveSecurityError::CompressionRatioExceeded {
            ratio,
            limit: limits.max_ratio,
        });
    }

    Ok(entry_count)
}
