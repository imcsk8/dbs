//! Content-Addressable Storage (CAS) engine and SHA-512 cryptographic verification.
//!
//! Stores source archives deduplicated by SHA-512 hash under `.cas/sha512/xx/<hash>`
//! and exposes dist-git URI schemas (`pkgs/<pkg>/<file>/sha512/<hash>/<file>`)
//! using zero-cost BTRFS reflinks.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use eyre::{eyre, Result};
use sha2::{Digest, Sha512};

/// Computes the cryptographic SHA-512 hex string of a file using streaming reads.
pub fn compute_sha512(path: &Path) -> Result<String> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return Err(eyre!("Failed to open file for SHA512 computation ({}): {}", path.display(), e)),
    };

    let mut hasher = Sha512::new();
    let mut buffer = [0u8; 65536];

    loop {
        let count = match file.read(&mut buffer) {
            Ok(c) => c,
            Err(e) => return Err(eyre!("Read error during SHA512 hashing: {}", e)),
        };
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// A parsed entry from a dist-git `sources` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistgitSourceEntry {
    pub filename: String,
    pub hash: String,
}

/// Parses entries from a dist-git `sources` manifest file.
pub fn parse_sources_file(path: &Path) -> Result<Vec<DistgitSourceEntry>> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return Err(eyre!("Failed to read sources manifest ({}): {}", path.display(), e)),
    };

    let mut entries = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Format 1: Fedora standard - SHA512 (filename) = hash
        if let Some(m) = trimmed.strip_prefix("SHA512 (") {
            if let Some((fname, h)) = m.split_once(") = ") {
                entries.push(DistgitSourceEntry {
                    filename: fname.trim().to_string(),
                    hash: h.trim().to_string(),
                });
                continue;
            }
        }

        // Format 2: Legacy dist-git - hash  filename
        if let Some((h, fname)) = trimmed.split_once([' ', '\t']) {
            let h_trim = h.trim();
            let f_trim = fname.trim();
            if !h_trim.is_empty() && !f_trim.is_empty() {
                entries.push(DistgitSourceEntry {
                    filename: f_trim.to_string(),
                    hash: h_trim.to_string(),
                });
            }
        }
    }

    Ok(entries)
}

/// Updates or creates a dist-git `sources` manifest file with a new file hash entry.
pub fn update_sources_manifest(dir: &Path, filename: &str, hash: &str) -> Result<PathBuf> {
    let manifest_path = dir.join("sources");

    let mut existing_entries = if manifest_path.exists() {
        match parse_sources_file(&manifest_path) {
            Ok(e) => e,
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    // Remove existing entry with same filename if present
    existing_entries.retain(|e| e.filename != filename);
    existing_entries.push(DistgitSourceEntry {
        filename: filename.to_string(),
        hash: hash.to_string(),
    });

    let mut content = String::new();
    for entry in &existing_entries {
        content.push_str(&format!("SHA512 ({}) = {}\n", entry.filename, entry.hash));
    }

    match fs::write(&manifest_path, content) {
        Ok(()) => Ok(manifest_path),
        Err(e) => Err(eyre!("Failed to write updated sources manifest ({}): {}", manifest_path.display(), e)),
    }
}
