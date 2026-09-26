//! Smart build gate for skipping redundant package compilations.
//!
//! Inspects RPM spec files and existing build artifacts (via database,
//! staging directory, and distro repositories) using native RPM version
//! comparison (`rpmvercmp`) to only build packages when a newer version or
//! release bump is detected.

use std::fs;
use std::path::{Path, PathBuf};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use eyre::{eyre, Result};
use librpm::package::PackageHeader;
use librpm::verify::VerifyOptions;
use librpm::version::Version;

use crate::models::{Package, PackageArtifact};
use crate::schema::{package, package_artifact};
use crate::types::BuildStatus;

/// Information about an existing build detected on disk or in the database.
#[derive(Debug, Clone)]
pub struct ExistingBuild {
    /// Canonical package name.
    pub name: String,
    /// Upstream version of existing build.
    pub version: String,
    /// Distribution release of existing build.
    pub release: String,
    /// Filesystem path to the existing binary RPM artifact.
    pub artifact_path: Option<PathBuf>,
    /// Origin where the existing build was verified ("database", "staging", "distro_repo").
    pub source: String,
}

/// Target metadata extracted before compilation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetMetadata {
    pub name: String,
    pub version: String,
    pub release: String,
    pub epoch: Option<i32>,
}

/// Normalizes a parsed spec release string to match the target environment's distribution tag.
pub fn normalize_spec_release(raw_spec: &str, expanded_release: &str, dist_tag: &str) -> String {
    let clean_tag = if dist_tag.starts_with('.') {
        dist_tag.to_string()
    } else {
        format!(".{}", dist_tag)
    };

    // 1. If unexpanded macro is present in the release string
    if expanded_release.contains("%{?dist}") || expanded_release.contains("%{dist}") {
        return expanded_release
            .replace("%{?dist}", &clean_tag)
            .replace("%{dist}", &clean_tag);
    }

    // 2. Query host %{?dist} from librpm macro context if initialized
    let _lock = crate::distgit::spec::RPM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = librpm::macro_context::MacroContext::default();
    if let Ok(host_dist) = ctx.expand("%{?dist}") {
        if !host_dist.is_empty() && expanded_release.contains(&host_dist) {
            return expanded_release.replace(&host_dist, &clean_tag);
        }
    }

    // 3. Fallback: check if the spec file preamble defined Release using %dist
    let has_dist_macro = raw_spec.lines().any(|l| {
        let t = l.trim();
        t.to_lowercase().starts_with("release:") && (t.contains("%{?dist}") || t.contains("%{dist}") || t.contains("%dist"))
    });

    if has_dist_macro {
        if expanded_release.ends_with(&clean_tag) {
            return expanded_release.to_string();
        }
        if let Some((base, _)) = expanded_release.rsplit_once('.') {
            return format!("{}{}", base, clean_tag);
        } else {
            return format!("{}{}", expanded_release, clean_tag);
        }
    }

    expanded_release.to_string()
}

/// Extracts metadata (Name, Version, Release, Epoch) from a target path (.spec or .src.rpm).
pub fn extract_target_metadata(target: &Path, dist_tag: Option<&str>) -> Result<TargetMetadata> {
    if !target.exists() {
        return Err(eyre!("Target path does not exist: {}", target.display()));
    }

    let target_str = target.to_string_lossy();
    if target_str.ends_with(".src.rpm") || target_str.ends_with(".rpm") {
        let hdr = PackageHeader::from_file(target, Some(&VerifyOptions::skip_verification()))
            .map_err(|e| eyre!("Failed to read RPM header from {}: {:?}", target.display(), e))?;
        Ok(TargetMetadata {
            name: hdr.name().to_string(),
            version: hdr.version().to_string(),
            release: hdr.release().to_string(),
            epoch: hdr.epoch(),
        })
    } else if target_str.ends_with(".spec") {
        let meta = crate::distgit::spec::parse_spec_file(target)?;
        let release = if let Some(tag) = dist_tag {
            let raw_content = fs::read_to_string(target).unwrap_or_default();
            normalize_spec_release(&raw_content, &meta.release, tag)
        } else {
            meta.release
        };
        Ok(TargetMetadata {
            name: meta.name,
            version: meta.version,
            release,
            epoch: Some(meta.epoch),
        })
    } else {
        Err(eyre!("Unsupported target format for metadata inspection: {}", target.display()))
    }
}

/// Checks if an identical or newer version of the package has already been compiled.
pub fn check_package_already_built(
    target_path: &Path,
    distro_dest: Option<&Path>,
    staging_dir: Option<&Path>,
    mut db_conn: Option<&mut PgConnection>,
    dist_tag: Option<&str>,
) -> Result<Option<ExistingBuild>> {
    let target_meta = match extract_target_metadata(target_path, dist_tag) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };

    let target_epoch_str = target_meta.epoch.map(|e| e.to_string());
    let target_ver = Version::new(
        target_epoch_str.as_deref(),
        &target_meta.version,
        Some(&target_meta.release),
    );

    // 1. Check PostgreSQL Database if available
    if let Some(conn) = db_conn.as_deref_mut() {
        if let Ok(pkgs) = package::table
            .filter(package::name.eq(&target_meta.name))
            .filter(package::build_status.eq(BuildStatus::SUCCESS))
            .load::<Package>(conn)
        {
            for p in pkgs {
                // Check artifacts recorded in database
                if let Ok(arts) = package_artifact::table
                    .filter(package_artifact::id_package.eq(Some(p.id)))
                    .filter(package_artifact::is_source.eq(false))
                    .load::<PackageArtifact>(conn)
                {
                    for art in arts {
                        let path = PathBuf::from(&art.rpm_path);
                        if path.exists() {
                            if let Ok(hdr) = PackageHeader::from_file(&path, Some(&VerifyOptions::skip_verification())) {
                                if hdr.name() == target_meta.name {
                                    let art_epoch_str = hdr.epoch().map(|e| e.to_string());
                                    if let Some(art_ver) = Version::new(art_epoch_str.as_deref(), hdr.version(), Some(hdr.release())) {
                                        if let Some(t_ver) = &target_ver {
                                            if art_ver >= *t_ver {
                                                return Ok(Some(ExistingBuild {
                                                    name: target_meta.name,
                                                    version: hdr.version().to_string(),
                                                    release: hdr.release().to_string(),
                                                    artifact_path: Some(path),
                                                    source: "database".to_string(),
                                                }));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Also check if package record has matching version/release
                if p.version != "0.0.0" {
                    if let Some(p_ver) = Version::new(None, &p.version, Some(&p.release)) {
                        if let Some(t_ver) = &target_ver {
                            if p_ver >= *t_ver {
                                return Ok(Some(ExistingBuild {
                                    name: target_meta.name,
                                    version: p.version,
                                    release: p.release,
                                    artifact_path: p.build_log_path.map(PathBuf::from),
                                    source: "database".to_string(),
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Check Staging Directory
    if let Some(stg) = staging_dir {
        if stg.exists() {
            let mut search_dirs = Vec::new();
            if let Ok(entries) = fs::read_dir(stg) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        if name.starts_with("worker-") && name.ends_with(&format!("-{}", target_meta.name)) {
                            search_dirs.push(path);
                        }
                    }
                }
            }
            search_dirs.push(stg.join("rpms").join("x86_64"));

            for dir in search_dirs {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rpm") {
                            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            if !fname.ends_with(".src.rpm") && fname.starts_with(&format!("{}-", target_meta.name)) {
                                if let Ok(hdr) = PackageHeader::from_file(&path, Some(&VerifyOptions::skip_verification())) {
                                    if hdr.name() == target_meta.name {
                                        let h_epoch = hdr.epoch().map(|e| e.to_string());
                                        if let Some(existing_ver) = Version::new(h_epoch.as_deref(), hdr.version(), Some(hdr.release())) {
                                            if let Some(t_ver) = &target_ver {
                                                if existing_ver >= *t_ver {
                                                    return Ok(Some(ExistingBuild {
                                                        name: target_meta.name,
                                                        version: hdr.version().to_string(),
                                                        release: hdr.release().to_string(),
                                                        artifact_path: Some(path),
                                                        source: "staging".to_string(),
                                                    }));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Check Target Distribution Directory
    if let Some(dest) = distro_dest {
        if dest.exists() {
            let mut search_dirs = vec![
                dest.to_path_buf(),
                dest.join("x86_64"),
                dest.join("Packages"),
            ];
            if let Ok(entries) = fs::read_dir(dest) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        search_dirs.push(path.join("x86_64"));
                        search_dirs.push(path.join("Packages"));
                    }
                }
            }

            for dir in search_dirs {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rpm") {
                            let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                            if !fname.ends_with(".src.rpm") && fname.starts_with(&format!("{}-", target_meta.name)) {
                                if let Ok(hdr) = PackageHeader::from_file(&path, Some(&VerifyOptions::skip_verification())) {
                                    if hdr.name() == target_meta.name {
                                        let h_epoch = hdr.epoch().map(|e| e.to_string());
                                        if let Some(existing_ver) = Version::new(h_epoch.as_deref(), hdr.version(), Some(hdr.release())) {
                                            if let Some(t_ver) = &target_ver {
                                                if existing_ver >= *t_ver {
                                                    return Ok(Some(ExistingBuild {
                                                        name: target_meta.name,
                                                        version: hdr.version().to_string(),
                                                        release: hdr.release().to_string(),
                                                        artifact_path: Some(path),
                                                        source: "distro_repo".to_string(),
                                                    }));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_extract_target_metadata_from_spec() {
        let dir = tempdir().unwrap();
        let spec_file = dir.path().join("mypkg.spec");
        fs::write(
            &spec_file,
            r#"Name:           mypkg
Version:        2.5.0
Release:        3%{?dist}
Summary:        Test package
License:        MIT

%description
A test package.
"#,
        )
        .unwrap();

        let meta = extract_target_metadata(&spec_file, Some(".tcrs")).unwrap();
        assert_eq!(meta.name, "mypkg");
        assert_eq!(meta.version, "2.5.0");
        assert_eq!(meta.release, "3.tcrs");
    }

    #[test]
    fn test_rpm_version_ordering() {
        let v1 = Version::new(None, "7.2", Some("1.tcrs")).unwrap();
        let v2 = Version::new(None, "7.2", Some("2.tcrs")).unwrap();
        let v3 = Version::new(None, "7.3", Some("1.tcrs")).unwrap();
        let v1_dup = Version::new(None, "7.2", Some("1.tcrs")).unwrap();

        assert_eq!(v1, v1_dup);
        assert!(v2 > v1);
        assert!(v3 > v2);
        assert!(v1 >= v1_dup);
    }
}
