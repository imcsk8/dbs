//! High-performance RPM `.spec` file parser written in pure Rust.
//!
//! Extracts essential RPM package metadata, dependencies (Requires and BuildRequires),
//! and referenced upstream source files without requiring external toolchains.

use std::collections::HashMap;
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

static RPM_INIT: std::sync::Once = std::sync::Once::new();
pub static RPM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Ensures that librpm is initialized once with default system configuration.
pub fn ensure_rpm_initialized() {
    RPM_INIT.call_once(|| {
        let _ = librpm::init();
    });
}

/// Expands RPM macros like `%{name}`, `%{version}`, `%{?dist}`, etc. using librpm's native macro engine.
pub fn expand_macros(input: &str, macros: &HashMap<String, String>) -> String {
    let _lock = RPM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ensure_rpm_initialized();
    let ctx = librpm::macro_context::MacroContext::default();
    for (k, v) in macros {
        let _ = ctx.define(&format!("{} {}", k, v), 0);
    }
    let result = match ctx.expand(input) {
        Ok(expanded) => expanded,
        Err(_) => input.to_string(),
    };
    for k in macros.keys() {
        let _ = ctx.pop(k);
    }
    result
}

/// Parses an RPM `.spec` file at the specified path.
pub fn parse_spec_file(spec_path: &Path) -> Result<SpecMetadata> {
    if !spec_path.exists() {
        return Err(eyre!("Spec file not found at {}", spec_path.display()));
    }

    let _lock = RPM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ensure_rpm_initialized();

    let spec_dir = spec_path
        .parent()
        .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let ctx = librpm::macro_context::MacroContext::default();
    let _ = ctx.define(&format!("_sourcedir {}", spec_dir.display()), 0);

    // 1. Try parsing with librpm::build::Spec for native, high-performance C parsing and expansion
    if let Some(spec_str) = spec_path.to_str() {
        if let Some(spec) = librpm::build::Spec::parse(spec_str, librpm::build::SpecFlags::NONE, None) {
            let hdr = spec.source_header();
            let mut meta = SpecMetadata {
                name: hdr.name().to_string(),
                version: hdr.version().to_string(),
                release: hdr.release().to_string(),
                epoch: hdr.epoch().unwrap_or(0),
                summary: hdr.summary().to_string(),
                license: hdr.license().to_string(),
                spec_path: Some(spec_path.to_path_buf()),
                ..Default::default()
            };

            if let Some(librpm::TagData::Str(url_str)) = hdr.get(librpm::Tag::URL) {
                meta.url = url_str.to_string();
            }

            // Extract preprocessed preamble from Section::NONE
            if let Some(preprocessed) = spec.get_section(librpm::build::Section::NONE) {
                for line in preprocessed.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("%description")
                        || trimmed.starts_with("%prep")
                        || trimmed.starts_with("%build")
                        || trimmed.starts_with("%install")
                        || trimmed.starts_with("%check")
                        || trimmed.starts_with("%clean")
                        || trimmed.starts_with("%files")
                        || trimmed.starts_with("%changelog")
                    {
                        break;
                    }
                    if let Some((k, v)) = trimmed.split_once(':') {
                        let tag = k.trim().to_lowercase();
                        let val = v.trim().to_string();
                        let is_source_tag = tag == "source"
                            || (tag.starts_with("source") && tag["source".len()..].chars().all(|c| c.is_ascii_digit()));
                        let is_patch_tag = tag == "patch"
                            || (tag.starts_with("patch") && tag["patch".len()..].chars().all(|c| c.is_ascii_digit()));

                        if is_source_tag && !val.is_empty() {
                            meta.sources.push(val);
                        } else if is_patch_tag && !val.is_empty() {
                            meta.patches.push(val);
                        } else if tag == "buildrequires" {
                            for req in val.split([',', ' ']) {
                                let cleaned = req.trim();
                                if !cleaned.is_empty()
                                    && !cleaned.starts_with('>')
                                    && !cleaned.starts_with('=')
                                    && !cleaned.starts_with('<')
                                {
                                    meta.build_requires.push(cleaned.to_string());
                                }
                            }
                        } else if tag == "requires" {
                            for req in val.split([',', ' ']) {
                                let cleaned = req.trim();
                                if !cleaned.is_empty()
                                    && !cleaned.starts_with('>')
                                    && !cleaned.starts_with('=')
                                    && !cleaned.starts_with('<')
                                {
                                    meta.requires.push(cleaned.to_string());
                                }
                            }
                        } else if tag == "provides" {
                            for prov in val.split([',', ' ']) {
                                let cleaned = prov.trim();
                                if !cleaned.is_empty()
                                    && !cleaned.starts_with('>')
                                    && !cleaned.starts_with('=')
                                    && !cleaned.starts_with('<')
                                {
                                    meta.provides.push(cleaned.to_string());
                                }
                            }
                        } else if tag == "url" && meta.url.is_empty() {
                            meta.url = val;
                        }
                    }
                }
            }

            // Fallback for sources/patches if Section::NONE did not yield them
            if meta.sources.is_empty() && meta.patches.is_empty() {
                for src in spec.sources() {
                    if src.is_source() {
                        meta.sources.push(src.filename().to_string());
                    } else if src.is_patch() {
                        meta.patches.push(src.filename().to_string());
                    }
                }
            }

            return Ok(meta);
        }
    }

    // 2. Fallback parser if librpm::build::Spec failed to parse the file
    let mut macros: HashMap<String, String> = HashMap::new();
    macros.insert("nil".to_string(), String::new());
    macros.insert("?dist".to_string(), String::new());
    macros.insert("dist".to_string(), String::new());
    macros.insert("_arch".to_string(), "x86_64".to_string());

    let raw_content = match fs::read_to_string(spec_path) {
        Ok(c) => c,
        Err(e) => return Err(eyre!("Failed to read spec file {}: {}", spec_path.display(), e)),
    };

    let mut meta = SpecMetadata::default();

    for line in raw_content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Check for %global or %define
        if trimmed.starts_with("%global") || trimmed.starts_with("%define") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 3 {
                let k = parts[1].split('(').next().unwrap_or(parts[1]).trim();
                let after_directive = trimmed[parts[0].len()..].trim_start();
                let v = after_directive[parts[1].len()..].trim();
                let expanded_v = expand_macros(v, &macros);
                macros.insert(k.to_string(), expanded_v);
            } else if parts.len() == 2 {
                let k = parts[1].split('(').next().unwrap_or(parts[1]).trim();
                macros.insert(k.to_string(), String::new());
            }
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
                "name" if meta.name.is_empty() => {
                    meta.name = expand_macros(&value, &macros);
                    macros.insert("name".to_string(), meta.name.clone());
                }
                "version" if meta.version.is_empty() => {
                    meta.version = expand_macros(&value, &macros);
                    macros.insert("version".to_string(), meta.version.clone());
                }
                "release" if meta.release.is_empty() => {
                    meta.release = expand_macros(&value, &macros);
                    macros.insert("release".to_string(), meta.release.clone());
                }
                "epoch" => {
                    if let Ok(ep) = value.parse::<i32>() {
                        meta.epoch = ep;
                    }
                    macros.insert("epoch".to_string(), value);
                }
                "summary" if meta.summary.is_empty() => {
                    meta.summary = expand_macros(&value, &macros);
                    macros.insert("summary".to_string(), meta.summary.clone());
                }
                "license" if meta.license.is_empty() => {
                    meta.license = expand_macros(&value, &macros);
                    macros.insert("license".to_string(), meta.license.clone());
                }
                "url" if meta.url.is_empty() => {
                    meta.url = expand_macros(&value, &macros);
                    macros.insert("url".to_string(), meta.url.clone());
                }
                "buildrequires" => {
                    for req in value.split([',', ' ']) {
                        let cleaned = req.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.build_requires.push(expand_macros(cleaned, &macros));
                        }
                    }
                }
                "requires" => {
                    for req in value.split([',', ' ']) {
                        let cleaned = req.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.requires.push(expand_macros(cleaned, &macros));
                        }
                    }
                }
                "provides" => {
                    for prov in value.split([',', ' ']) {
                        let cleaned = prov.trim();
                        if !cleaned.is_empty() && !cleaned.starts_with('>') && !cleaned.starts_with('=') && !cleaned.starts_with('<') {
                            meta.provides.push(expand_macros(cleaned, &macros));
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

    for src in &mut meta.sources {
        *src = expand_macros(src, &macros);
    }
    for patch in &mut meta.patches {
        *patch = expand_macros(patch, &macros);
    }

    meta.url = expand_macros(&meta.url, &macros);
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
        assert!(meta.release.starts_with("4"));
        assert_eq!(meta.epoch, 2);
        assert_eq!(meta.summary, "Test package summary");
        assert_eq!(meta.license, "MIT");
        assert_eq!(meta.url, "https://example.org");
        assert!(meta.build_requires.contains(&"gcc".to_string()));
        assert!(meta.build_requires.contains(&"make".to_string()));
        assert!(meta.requires.contains(&"glibc".to_string()));
        assert_eq!(meta.sources, vec!["test-pkg-1.2.3.tar.gz".to_string()]);
    }

    #[test]
    fn test_expand_macros() {
        let mut macros = HashMap::new();
        macros.insert("name".to_string(), "pv".to_string());
        macros.insert("version".to_string(), "1.11.0".to_string());
        macros.insert("dist".to_string(), ".el9".to_string());

        let res1 = expand_macros("https://www.ivarch.com/programs/sources/%{name}-%{version}.tar.gz.txt", &macros);
        assert_eq!(res1, "https://www.ivarch.com/programs/sources/pv-1.11.0.tar.gz.txt");

        let res2 = expand_macros("1%{?dist}", &macros);
        assert_eq!(res2, "1.el9");

        let res3 = expand_macros("1%{!?dist:.eln}", &macros);
        assert_eq!(res3, "1");

        let res4 = expand_macros("https://example.com/%name-%version.tar.xz", &macros);
        assert_eq!(res4, "https://example.com/pv-1.11.0.tar.xz");
    }

    #[test]
    fn test_parse_spec_macro_expansion() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "Summary: A tool for monitoring the progress of data").unwrap();
        writeln!(file, "Name: pv").unwrap();
        writeln!(file, "Version: 1.11.0").unwrap();
        writeln!(file, "Release: 1%{{?dist}}").unwrap();
        writeln!(file, "License: GPL-3.0-or-later").unwrap();
        writeln!(file, "URL: https://www.ivarch.com/programs/%{{name}}.shtml").unwrap();
        writeln!(file, "Source0: https://www.ivarch.com/programs/sources/%{{name}}-%{{version}}.tar.gz").unwrap();
        writeln!(file, "Source1: https://www.ivarch.com/programs/sources/%{{name}}-%{{version}}.tar.gz.txt").unwrap();
        writeln!(file, "Source2: https://www.ivarch.com/personal/public-key.txt").unwrap();
        writeln!(file, "%description").unwrap();
        writeln!(file, "Pipe Viewer").unwrap();

        let meta = parse_spec_file(file.path()).unwrap();
        assert_eq!(meta.name, "pv");
        assert_eq!(meta.version, "1.11.0");
        assert_eq!(meta.url, "https://www.ivarch.com/programs/pv.shtml");
        assert_eq!(
            meta.sources,
            vec![
                "https://www.ivarch.com/programs/sources/pv-1.11.0.tar.gz".to_string(),
                "https://www.ivarch.com/programs/sources/pv-1.11.0.tar.gz.txt".to_string(),
                "https://www.ivarch.com/personal/public-key.txt".to_string(),
            ]
        );
    }

    #[test]
    fn test_librpm_spec_parsing() {
        use librpm::build::{Spec, SpecFlags};

        ensure_rpm_initialized();

        let mut temp_spec = NamedTempFile::new().unwrap();
        writeln!(temp_spec, "Name: test-pkg").unwrap();
        writeln!(temp_spec, "Version: 1.2.3").unwrap();
        writeln!(temp_spec, "Release: 4%{{?dist}}").unwrap();
        writeln!(temp_spec, "Epoch: 2").unwrap();
        writeln!(temp_spec, "Summary: Test package summary").unwrap();
        writeln!(temp_spec, "License: MIT").unwrap();
        writeln!(temp_spec, "URL: https://example.org").unwrap();
        writeln!(temp_spec, "BuildRequires: gcc, make").unwrap();
        writeln!(temp_spec, "Requires: glibc").unwrap();
        writeln!(temp_spec, "Source0: test-pkg-1.2.3.tar.gz").unwrap();
        writeln!(temp_spec, "%description").unwrap();
        writeln!(temp_spec, "This is the body description").unwrap();

        let s1 = Spec::parse(temp_spec.path().to_str().unwrap(), SpecFlags::NONE, None).expect("failed s1");
        assert_eq!(s1.source_header().name(), "test-pkg");
        assert_eq!(s1.source_header().version(), "1.2.3");
        assert!(s1.source_header().release().starts_with("4"));

        if std::path::Path::new("/srv/dbs/tacos/rpm/pv/pv.spec").exists() {
            let meta_pv = parse_spec_file(std::path::Path::new("/srv/dbs/tacos/rpm/pv/pv.spec")).expect("failed to parse pv.spec");
            assert_eq!(meta_pv.name, "pv");
            assert_eq!(meta_pv.version, "1.11.0");
            assert!(meta_pv.sources.iter().any(|s| s.contains("pv-1.11.0.tar.gz")));
        }

        if std::path::Path::new("/srv/dbs/tacos/rpm/systemd/systemd.spec").exists() {
            let meta_sd = parse_spec_file(std::path::Path::new("/srv/dbs/tacos/rpm/systemd/systemd.spec")).expect("failed to parse systemd.spec");
            assert_eq!(meta_sd.name, "systemd");
            assert_eq!(meta_sd.version, "262");
            assert!(!meta_sd.sources.is_empty());
        }
    }
}
