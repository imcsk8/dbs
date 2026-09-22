//! High-performance RPM `.spec` file parser written in pure Rust.
//!
//! Extracts essential RPM package metadata, dependencies (Requires and BuildRequires),
//! and referenced upstream source files without requiring external toolchains.

use std::fs;
use std::path::Path;
use eyre::{eyre, Result};

/// Extracted metadata from an RPM `.spec` file.
#[derive(Debug, Clone, Default)]
pub struct SpecMetadata {
    /// Package name tag.
    pub name: String,
    /// Upstream software version tag.
    pub version: String,
    /// Distribution release tag.
    pub release: String,
    /// Package epoch.
    pub epoch: i32,
    /// Brief one-line package summary.
    pub summary: String,
    /// Software licensing identifier (e.g. GPL-3.0-or-later, MIT).
    pub license: String,
    /// Project home page or repository URL.
    pub url: String,
    /// Build-time dependency capability requirements (`BuildRequires`).
    pub build_requires: Vec<String>,
    /// Runtime dependency capability requirements (`Requires`).
    pub requires: Vec<String>,
    /// Provided capabilities (`Provides`).
    pub provides: Vec<String>,
    /// Referenced source filenames or URLs (`Source0`, `Source1`, ...).
    pub sources: Vec<String>,
    /// Referenced patch filenames (`Patch0`, `Patch1`, ...).
    pub patches: Vec<String>,
    /// Filesystem path to this `.spec` file on disk.
    pub spec_path: Option<std::path::PathBuf>,
}

/// Parses an RPM `.spec` file at the specified path.
pub fn parse_spec_file(spec_path: &Path) -> Result<SpecMetadata> {
    if !spec_path.exists() {
        return Err(eyre!("Spec file not found at {}", spec_path.display()));
    }

    let content = match fs::read_to_string(spec_path) {
        Ok(c) => c,
        Err(e) => return Err(eyre!("Failed to read spec file {}: {}", spec_path.display(), e)),
    };

    let mut meta = SpecMetadata::default();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Stop processing preamble when body sections begin
        if trimmed.starts_with("%description") || trimmed.starts_with("%prep") || trimmed.starts_with("%build") {
            break;
        }

        // Split key: value
        if let Some((key, val)) = trimmed.split_once(':') {
            let tag = key.trim().to_lowercase();
            let value = val.trim().to_string();

            match tag.as_str() {
                "name" if meta.name.is_empty() => meta.name = value,
                "version" if meta.version.is_empty() => meta.version = value,
                "release" if meta.release.is_empty() => meta.release = value,
                "epoch" => {
                    if let Ok(ep) = value.parse::<i32>() {
                        meta.epoch = ep;
                    }
                }
                "summary" if meta.summary.is_empty() => meta.summary = value,
                "license" if meta.license.is_empty() => meta.license = value,
                "url" if meta.url.is_empty() => meta.url = value,
                "buildrequires" => {
                    for req in value.split([',', ' ']) {
                        let cleaned = req.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.build_requires.push(cleaned.to_string());
                        }
                    }
                }
                "requires" => {
                    for req in value.split([',', ' ']) {
                        let cleaned = req.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.requires.push(cleaned.to_string());
                        }
                    }
                }
                "provides" => {
                    for prov in value.split([',', ' ']) {
                        let cleaned = prov.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.provides.push(cleaned.to_string());
                        }
                    }
                }
                k if k.starts_with("source") => {
                    if !value.is_empty() {
                        meta.sources.push(value);
                    }
                }
                k if k.starts_with("patch") => {
                    if !value.is_empty() {
                        meta.patches.push(value);
                    }
                }
                _ => (),
            }
        }
    }

    meta.spec_path = Some(spec_path.to_path_buf());
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_spec_metadata() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Name: test-pkg").unwrap();
        writeln!(file, "Version: 1.2.3").unwrap();
        writeln!(file, "Release: 4%{{?dist}}").unwrap();
        writeln!(file, "Epoch: 2").unwrap();
        writeln!(file, "Summary: Test package summary").unwrap();
        writeln!(file, "License: MIT").unwrap();
        writeln!(file, "URL: https://example.org").unwrap();
        writeln!(file, "BuildRequires: gcc, make").unwrap();
        writeln!(file, "Requires: glibc").unwrap();
        writeln!(file, "Source0: test-pkg-1.2.3.tar.gz").unwrap();
        writeln!(file, "%description").unwrap();
        writeln!(file, "This is the body description").unwrap();

        let meta = parse_spec_file(file.path()).unwrap();
        assert_eq!(meta.name, "test-pkg");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.release, "4%{?dist}");
        assert_eq!(meta.epoch, 2);
        assert_eq!(meta.summary, "Test package summary");
        assert_eq!(meta.license, "MIT");
        assert_eq!(meta.url, "https://example.org");
        assert!(meta.build_requires.contains(&"gcc".to_string()));
        assert!(meta.build_requires.contains(&"make".to_string()));
        assert!(meta.requires.contains(&"glibc".to_string()));
        assert_eq!(meta.sources, vec!["test-pkg-1.2.3.tar.gz".to_string()]);
    }
}
