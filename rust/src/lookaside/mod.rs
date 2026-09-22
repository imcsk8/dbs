//! Lookaside cache management for TacOS and dist-git distributions.
//!
//! Provides Content-Addressable Storage (CAS) for binary source archives,
//! high-efficiency BTRFS CoW cloning, standard Fedora/TacOS HTTP URL directory
//! exposure, and automated retrieval/synchronization.

pub mod cas;
pub mod reflink;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use eyre::{eyre, Result};
use tokio::sync::Semaphore;

use self::cas::{compute_sha512, parse_sources_file, update_sources_manifest};
use self::reflink::{is_btrfs, reflink_or_copy, ReflinkMode};

/// Default lookaside cache filesystem path for TacOS DBS builds.
pub const DEFAULT_LOOKASIDE_PATH: &str = "/srv/dbs/lookaside";
/// Fallback local lookaside directory within the project workspace.
pub const LOCAL_LOOKASIDE_FALLBACK: &str = "data/lookaside";

/// Primary Lookaside Cache Manager.
#[derive(Debug, Clone)]
pub struct LookasideManager {
    /// Root path of the lookaside cache repository.
    pub root: PathBuf,
}

/// Metadata and status report of an uploaded source archive.
#[derive(Debug, Clone)]
pub struct UploadResult {
    pub package: String,
    pub filename: String,
    pub hash: String,
    pub size_bytes: u64,
    pub cas_path: PathBuf,
    pub pkgs_path: PathBuf,
    pub reflink_mode: ReflinkMode,
    pub manifest_updated: Option<PathBuf>,
}

/// Comprehensive status and storage metrics of the lookaside repository.
#[derive(Debug, Clone)]
pub struct LookasideStatus {
    pub root: PathBuf,
    pub is_btrfs: bool,
    pub total_cas_objects: usize,
    pub total_cas_bytes: u64,
    pub total_packages: usize,
    pub total_package_entries: usize,
}

/// Report summarizing garbage collection outcomes.
#[derive(Debug, Clone)]
pub struct GcReport {
    pub scanned_objects: usize,
    pub orphaned_objects: Vec<PathBuf>,
    pub reclaimed_bytes: u64,
    pub dry_run: bool,
}

/// Report summarizing synchronization of dist-git repositories with lookaside.
#[derive(Debug, Clone)]
pub struct SyncReport {
    pub total_sources_found: usize,
    pub already_cached: usize,
    pub downloaded: usize,
    pub failed: usize,
    pub failures: Vec<String>,
}

impl LookasideManager {
    /// Instantiates a LookasideManager for the specified root directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolves the effective lookaside cache directory by checking:
    /// 1. User-supplied explicit path
    /// 2. `DBS_LOOKASIDE_DIR` environment variable
    /// 3. Standard system path `/srv/dbs/lookaside` (if existing or parent exists)
    /// 4. Local workspace fallback `data/lookaside`
    pub fn resolve_default(explicit: Option<&Path>) -> Self {
        if let Some(p) = explicit {
            return Self::new(p.to_path_buf());
        }

        if let Ok(env_path) = std::env::var("DBS_LOOKASIDE_DIR") {
            if !env_path.trim().is_empty() {
                return Self::new(PathBuf::from(env_path));
            }
        }

        let sys_path = Path::new(DEFAULT_LOOKASIDE_PATH);
        if sys_path.exists() {
            return Self::new(sys_path.to_path_buf());
        }

        // If /srv/dbs exists, use /srv/dbs/lookaside
        if let Some(parent) = sys_path.parent() {
            if parent.exists() {
                return Self::new(sys_path.to_path_buf());
            }
        }

        Self::new(PathBuf::from(LOCAL_LOOKASIDE_FALLBACK))
    }

    /// Initializes repository directory structures (.cas/sha512 and pkgs).
    pub fn init(&self) -> Result<()> {
        let cas_dir = self.cas_dir();
        let pkgs_dir = self.pkgs_dir();

        match fs::create_dir_all(&cas_dir) {
            Ok(()) => {}
            Err(e) => return Err(eyre!("Failed to create CAS directory ({}): {}", cas_dir.display(), e)),
        };

        match fs::create_dir_all(&pkgs_dir) {
            Ok(()) => {}
            Err(e) => return Err(eyre!("Failed to create pkgs directory ({}): {}", pkgs_dir.display(), e)),
        };

        Ok(())
    }

    /// Directory storing content-addressable objects indexed by SHA-512.
    pub fn cas_dir(&self) -> PathBuf {
        self.root.join(".cas").join("sha512")
    }

    /// Directory exposing standard Fedora/TacOS HTTP URL hierarchy.
    pub fn pkgs_dir(&self) -> PathBuf {
        self.root.join("pkgs")
    }

    /// Checks if the underlying storage filesystem is BTRFS.
    pub fn is_btrfs_fs(&self) -> bool {
        is_btrfs(&self.root)
    }

    /// Resolves the exact CAS path for a given SHA-512 hash.
    pub fn cas_path_for_hash(&self, hash: &str) -> PathBuf {
        let prefix = if hash.len() >= 2 { &hash[0..2] } else { "00" };
        self.cas_dir().join(prefix).join(hash)
    }

    /// Resolves the public dist-git URL path for a package file.
    pub fn pkgs_path_for_file(&self, pkg_name: &str, filename: &str, hash: &str) -> PathBuf {
        self.pkgs_dir()
            .join(pkg_name)
            .join(filename)
            .join("sha512")
            .join(hash)
            .join(filename)
    }

    /// Uploads and registers a source archive into the lookaside cache.
    ///
    /// Deduplicates storage via CAS, exposes the file under `pkgs/`,
    /// and optionally updates the package's dist-git `sources` manifest.
    pub fn upload(
        &self,
        file_path: &Path,
        pkg_name: &str,
        spec_path: Option<&Path>,
        update_manifest: bool,
    ) -> Result<UploadResult> {
        self.init()?;

        if !file_path.exists() {
            return Err(eyre!("Source archive file does not exist: {}", file_path.display()));
        }

        let filename = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| eyre!("Invalid source filename: {}", file_path.display()))?
            .to_string();

        let size_bytes = fs::metadata(file_path)?.len();
        let hash = compute_sha512(file_path)?;

        // 1. Store in CAS
        let cas_target = self.cas_path_for_hash(&hash);
        if let Some(p) = cas_target.parent() {
            fs::create_dir_all(p)?;
        }

        let reflink_mode = if !cas_target.exists() {
            match reflink_or_copy(file_path, &cas_target) {
                Ok(mode) => mode,
                Err(e) => return Err(eyre!("Failed to store file in CAS: {}", e)),
            }
        } else {
            ReflinkMode::Reflink
        };

        // 2. Expose in pkgs/ hierarchy using reflink
        let pkgs_target = self.pkgs_path_for_file(pkg_name, &filename, &hash);
        if let Some(p) = pkgs_target.parent() {
            fs::create_dir_all(p)?;
        }

        if !pkgs_target.exists() {
            let _ = reflink_or_copy(&cas_target, &pkgs_target);
        }

        // 3. Update dist-git sources manifest if requested
        let manifest_updated = if update_manifest {
            let target_dir = if let Some(spec) = spec_path {
                spec.parent().unwrap_or_else(|| Path::new("."))
            } else {
                file_path.parent().unwrap_or_else(|| Path::new("."))
            };
            let updated = update_sources_manifest(target_dir, &filename, &hash)?;
            Some(updated)
        } else {
            None
        };

        Ok(UploadResult {
            package: pkg_name.to_string(),
            filename,
            hash,
            size_bytes,
            cas_path: cas_target,
            pkgs_path: pkgs_target,
            reflink_mode,
            manifest_updated,
        })
    }

    /// Retrieves a source file by hash, checking local lookaside first (via reflink),
    /// and downloading from upstream caches on demand.
    pub fn get_or_fetch(
        &self,
        pkg_name: &str,
        filename: &str,
        hash: &str,
        dest_path: &Path,
    ) -> Result<ReflinkMode> {
        let _ = self.init();

        // 1. Check local CAS
        let cas_path = self.cas_path_for_hash(hash);
        if cas_path.exists() {
            match reflink_or_copy(&cas_path, dest_path) {
                Ok(mode) => return Ok(mode),
                Err(e) => return Err(eyre!("Failed to link from CAS ({}): {}", cas_path.display(), e)),
            }
        }

        // 2. Check local pkgs hierarchy
        let pkgs_path = self.pkgs_path_for_file(pkg_name, filename, hash);
        if pkgs_path.exists() {
            match reflink_or_copy(&pkgs_path, dest_path) {
                Ok(mode) => return Ok(mode),
                Err(e) => return Err(eyre!("Failed to link from pkgs hierarchy: {}", e)),
            }
        }

        // 3. Not in local lookaside: fetch from upstream remote lookaside directly into CAS
        if let Some(p) = cas_path.parent() {
            fs::create_dir_all(p)?;
        }

        let downloaded = self.download_from_remote(pkg_name, filename, hash, &cas_path)?;
        if !downloaded {
            return Err(eyre!(
                "Source archive '{}' (sha512: {}) could not be found in local lookaside or upstream mirrors",
                filename,
                hash
            ));
        }

        // Verify SHA512 of downloaded file
        let computed = compute_sha512(&cas_path)?;
        if !computed.eq_ignore_ascii_case(hash) {
            let _ = fs::remove_file(&cas_path);
            return Err(eyre!(
                "Hash mismatch for downloaded source '{}': expected {}, got {}",
                filename,
                hash,
                computed
            ));
        }

        // Expose in pkgs hierarchy
        if let Some(p) = pkgs_path.parent() {
            let _ = fs::create_dir_all(p);
        }
        let _ = reflink_or_copy(&cas_path, &pkgs_path);

        // Finally stage to destination using reflink
        match reflink_or_copy(&cas_path, dest_path) {
            Ok(mode) => Ok(mode),
            Err(e) => Err(eyre!("Failed to stage downloaded source to destination: {}", e)),
        }
    }

    /// Downloads a source archive from TacOS, Fedora Rawhide, or CentOS lookaside URLs.
    fn download_from_remote(
        &self,
        pkg_name: &str,
        filename: &str,
        hash: &str,
        dest: &Path,
    ) -> Result<bool> {
        let urls = [
            format!("https://repos.tacos.org.mx/sources/{}/{}", pkg_name, filename),
            format!(
                "https://src.fedoraproject.org/repo/pkgs/{}/{}/sha512/{}/{}",
                pkg_name, filename, hash, filename
            ),
            format!(
                "https://sources.stream.centos.org/sources/rpms/{}/{}/sha512/{}/{}",
                pkg_name, filename, hash, filename
            ),
        ];

        for url in &urls {
            let status = Command::new("curl")
                .arg("-f")
                .arg("-L")
                .arg("-s")
                .arg("-S")
                .arg("--connect-timeout")
                .arg("10")
                .arg("-o")
                .arg(dest)
                .arg(url)
                .status();

            if let Ok(st) = status {
                if st.success() && dest.exists() {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Gathers status and capacity statistics for the lookaside repository.
    pub fn status(&self) -> Result<LookasideStatus> {
        let is_btrfs = self.is_btrfs_fs();
        let mut total_cas_objects = 0;
        let mut total_cas_bytes = 0;
        let mut total_packages = 0;
        let mut total_package_entries = 0;

        let cas_dir = self.cas_dir();
        if cas_dir.exists() {
            if let Ok(subdirs) = fs::read_dir(&cas_dir) {
                for sub in subdirs.flatten() {
                    let p = sub.path();
                    if p.is_dir() {
                        if let Ok(files) = fs::read_dir(&p) {
                            for f in files.flatten() {
                                if let Ok(meta) = f.metadata() {
                                    if meta.is_file() {
                                        total_cas_objects += 1;
                                        total_cas_bytes += meta.len();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let pkgs_dir = self.pkgs_dir();
        if pkgs_dir.exists() {
            if let Ok(entries) = fs::read_dir(&pkgs_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        total_packages += 1;
                        total_package_entries += count_files_recursive(&entry.path());
                    }
                }
            }
        }

        Ok(LookasideStatus {
            root: self.root.clone(),
            is_btrfs,
            total_cas_objects,
            total_cas_bytes,
            total_packages,
            total_package_entries,
        })
    }

    /// Performs garbage collection, identifying and optionally removing CAS objects
    /// that are no longer referenced by any package in the lookaside or dist-git tree.
    pub fn gc(&self, dry_run: bool, distgit_dir: Option<&Path>) -> Result<GcReport> {
        let mut referenced_hashes = std::collections::HashSet::new();

        // 1. Scan pkgs hierarchy for hashes
        let pkgs_dir = self.pkgs_dir();
        if pkgs_dir.exists() {
            collect_hashes_from_dir(&pkgs_dir, &mut referenced_hashes);
        }

        // 2. Scan dist-git sources files if provided
        if let Some(distgit) = distgit_dir {
            if distgit.exists() {
                collect_hashes_from_sources_manifests(distgit, &mut referenced_hashes);
            }
        }

        let mut scanned = 0;
        let mut orphaned = Vec::new();
        let mut reclaimed_bytes = 0;

        let cas_dir = self.cas_dir();
        if cas_dir.exists() {
            if let Ok(subdirs) = fs::read_dir(&cas_dir) {
                for sub in subdirs.flatten() {
                    let p = sub.path();
                    if p.is_dir() {
                        if let Ok(files) = fs::read_dir(&p) {
                            for f in files.flatten() {
                                let file_path = f.path();
                                if file_path.is_file() {
                                    scanned += 1;
                                    let hash_name = file_path
                                        .file_name()
                                        .and_then(|s| s.to_str())
                                        .unwrap_or_default();

                                    if !referenced_hashes.contains(hash_name) {
                                        if let Ok(m) = file_path.metadata() {
                                            reclaimed_bytes += m.len();
                                        }
                                        orphaned.push(file_path.clone());
                                        if !dry_run {
                                            let _ = fs::remove_file(&file_path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(GcReport {
            scanned_objects: scanned,
            orphaned_objects: orphaned,
            reclaimed_bytes,
            dry_run,
        })
    }

    /// Synchronizes lookaside cache by scanning dist-git repositories for missing source tarballs.
    pub async fn sync_dir(&self, target_dir: &Path, concurrency: usize) -> Result<SyncReport> {
        self.init()?;

        let mut sources_to_fetch = Vec::new();
        collect_distgit_sources(target_dir, &mut sources_to_fetch);

        let total_found = sources_to_fetch.len();
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let manager = Arc::new(self.clone());

        let mut tasks = Vec::new();

        for item in sources_to_fetch {
            let sem = semaphore.clone();
            let mgr = manager.clone();

            let task = tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let cas_target = mgr.cas_path_for_hash(&item.hash);
                let pkg_dest = item.sources_dir.join(&item.filename);

                if cas_target.exists() {
                    if !pkg_dest.exists() {
                        let _ = reflink_or_copy(&cas_target, &pkg_dest);
                    }
                    return Ok((item.pkg_name, item.filename, false)); // already cached
                }

                // Temporary fetch destination
                let temp_dest = mgr.root.join(".tmp").join(&item.hash);
                if let Some(p) = temp_dest.parent() {
                    let _ = fs::create_dir_all(p);
                }

                let fetch_res = tokio::task::spawn_blocking({
                    let mgr_clone = mgr.clone();
                    let p_name = item.pkg_name.clone();
                    let f_name = item.filename.clone();
                    let h_name = item.hash.clone();
                    let tmp = temp_dest.clone();
                    move || mgr_clone.download_from_remote(&p_name, &f_name, &h_name, &tmp)
                })
                .await;

                let ok = match fetch_res {
                    Ok(Ok(success)) => success,
                    _ => false,
                };

                if ok && temp_dest.exists() {
                    let _ = fs::create_dir_all(cas_target.parent().unwrap());
                    let _ = fs::rename(&temp_dest, &cas_target);
                    let pkgs_target = mgr.pkgs_path_for_file(&item.pkg_name, &item.filename, &item.hash);
                    if let Some(p) = pkgs_target.parent() {
                        let _ = fs::create_dir_all(p);
                    }
                    let _ = reflink_or_copy(&cas_target, &pkgs_target);

                    // Also stage into package sources_dir using BTRFS reflink
                    if !pkg_dest.exists() {
                        let _ = reflink_or_copy(&cas_target, &pkg_dest);
                    }

                    Ok((item.pkg_name, item.filename, true))
                } else {
                    let _ = fs::remove_file(&temp_dest);
                    Err(format!("{}: {}", item.pkg_name, item.filename))
                }
            });

            tasks.push(task);
        }

        let mut already_cached = 0;
        let mut downloaded = 0;
        let mut failed = 0;
        let mut failures = Vec::new();

        for t in tasks {
            match t.await {
                Ok(Ok((_, _, was_downloaded))) => {
                    if was_downloaded {
                        downloaded += 1;
                    } else {
                        already_cached += 1;
                    }
                }
                Ok(Err(fail)) => {
                    failed += 1;
                    failures.push(fail);
                }
                Err(e) => {
                    failed += 1;
                    failures.push(format!("Task failure: {}", e));
                }
            }
        }

        Ok(SyncReport {
            total_sources_found: total_found,
            already_cached,
            downloaded,
            failed,
            failures,
        })
    }
}

/// A target source archive identified in dist-git with its local package sources directory.
#[derive(Debug, Clone)]
pub struct DistgitSourceItem {
    pub pkg_name: String,
    pub filename: String,
    pub hash: String,
    pub sources_dir: PathBuf,
}

/// Recursively discovers all `sources` files under a dist-git directory tree.
fn collect_distgit_sources(dir: &Path, out: &mut Vec<DistgitSourceItem>) {
    if !dir.exists() {
        return;
    }

    if dir.is_file() {
        if dir.file_name().and_then(|s| s.to_str()) == Some("sources") {
            let parent = dir.parent().unwrap_or_else(|| Path::new("."));
            let pkg_name = parent
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("pkg")
                .to_string();
            let sources_dir = if parent.join("SOURCES").is_dir() {
                parent.join("SOURCES")
            } else {
                parent.to_path_buf()
            };

            if let Ok(entries) = parse_sources_file(dir) {
                for e in entries {
                    out.push(DistgitSourceItem {
                        pkg_name: pkg_name.clone(),
                        filename: e.filename,
                        hash: e.hash,
                        sources_dir: sources_dir.clone(),
                    });
                }
            }
        }
        return;
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let sources_file = p.join("sources");
                if sources_file.is_file() {
                    let pkg_name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("pkg")
                        .to_string();
                    let sources_dir = if p.join("SOURCES").is_dir() {
                        p.join("SOURCES")
                    } else {
                        p.clone()
                    };

                    if let Ok(manifest_entries) = parse_sources_file(&sources_file) {
                        for e in manifest_entries {
                            out.push(DistgitSourceItem {
                                pkg_name: pkg_name.clone(),
                                filename: e.filename,
                                hash: e.hash,
                                sources_dir: sources_dir.clone(),
                            });
                        }
                    }
                } else {
                    collect_distgit_sources(&p, out);
                }
            }
        }
    }
}

/// Collects hashes from `sources` files within a dist-git repository.
fn collect_hashes_from_sources_manifests(dir: &Path, out: &mut std::collections::HashSet<String>) {
    let mut sources = Vec::new();
    collect_distgit_sources(dir, &mut sources);
    for item in sources {
        out.insert(item.hash);
    }
}

/// Recursively scans a directory hierarchy and collects directories named with valid sha512 hashes.
fn collect_hashes_from_dir(dir: &Path, out: &mut std::collections::HashSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                if name.len() == 128 && name.chars().all(|c| c.is_ascii_hexdigit()) {
                    out.insert(name.to_string());
                } else {
                    collect_hashes_from_dir(&p, out);
                }
            }
        }
    }
}

/// Counts total regular files recursively in a directory tree.
fn count_files_recursive(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += count_files_recursive(&p);
            } else if p.is_file() {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cas_hash_and_storage() {
        let dir = tempdir().unwrap();
        let mgr = LookasideManager::new(dir.path().join("lookaside"));
        mgr.init().unwrap();

        let src_file = dir.path().join("test-1.0.tar.gz");
        fs::write(&src_file, b"content-of-tarball-archive").unwrap();

        let res = mgr.upload(&src_file, "mypkg", None, false).unwrap();
        assert_eq!(res.package, "mypkg");
        assert_eq!(res.filename, "test-1.0.tar.gz");
        assert_eq!(res.hash.len(), 128); // SHA-512 length
        assert!(res.cas_path.exists());
        assert!(res.pkgs_path.exists());

        let status = mgr.status().unwrap();
        assert_eq!(status.total_cas_objects, 1);
        assert_eq!(status.total_packages, 1);

        // Fetch to new destination
        let dest = dir.path().join("downloaded.tar.gz");
        let fetch_res = mgr.get_or_fetch("mypkg", "test-1.0.tar.gz", &res.hash, &dest);
        assert!(fetch_res.is_ok());
        assert!(dest.exists());
        let content = fs::read(&dest).unwrap();
        assert_eq!(content, b"content-of-tarball-archive");
    }

    #[test]
    fn test_sources_manifest_update() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("zstd");
        fs::create_dir_all(&pkg_dir).unwrap();

        let updated = update_sources_manifest(&pkg_dir, "zstd-1.5.7.tar.gz", "abcdef123456").unwrap();
        assert!(updated.exists());

        let entries = parse_sources_file(&updated).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "zstd-1.5.7.tar.gz");
        assert_eq!(entries[0].hash, "abcdef123456");

        // Add second file
        update_sources_manifest(&pkg_dir, "patch-1.patch", "987654fedcba").unwrap();
        let entries2 = parse_sources_file(&updated).unwrap();
        assert_eq!(entries2.len(), 2);
    }
}
