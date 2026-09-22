//! Build execution runners for DBS.
//!
//! Standardizes on **Mock** as the isolated, hermetic chroot build engine,
//! coupled with Rust's high-efficiency asynchronous concurrency system (`tokio::sync::Semaphore`)
//! for worker orchestration, dynamic local repository feedback, and sequential chain builds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use eyre::{eyre, Result};
use tokio::sync::Semaphore;

static REPO_LOCK: Mutex<()> = Mutex::new(());

/// Results and output artifacts from a package build execution.
#[derive(Debug, Clone)]
pub struct BuildOutput {
    /// Package or target identifier.
    pub target_name: String,
    /// Whether compilation completed successfully.
    pub success: bool,
    /// Total duration of build execution in seconds.
    pub duration_seconds: f64,
    /// Path to primary build log.
    pub log_path: PathBuf,
    /// List of generated RPM and SRPM file paths.
    pub artifacts: Vec<PathBuf>,
    /// Error summary snippet if the build failed.
    pub error_summary: Option<String>,
}

/// Extensible trait implemented by package build runners in DBS.
pub trait BuildRunner: Send + Sync {
    /// Human-readable runner name.
    fn name(&self) -> &'static str;

    /// Executes the compilation of a `.spec` or `.src.rpm` file into binary RPMs.
    fn build(&self, input_path: &Path, result_dir: &Path) -> Result<BuildOutput>;
}

/// Enterprise-grade Mock runner executing hermetic chroot builds.
#[derive(Debug, Clone)]
pub struct MockRunner {
    /// Mock chroot configuration profile name (e.g. `tacos-rolling-x86_64`, `fedora-rawhide-x86_64`, `centos-stream-10-x86_64`).
    pub root_name: String,
    /// Directory containing Mock `.cfg` profile files.
    pub config_dir: Option<PathBuf>,
    /// Unique extension tag for worker isolation (`--uniqueext`).
    pub unique_ext: Option<String>,
    /// Local RPM repository directory to inject via `--addrepo=file://...`.
    pub local_repo_dir: Option<PathBuf>,
    /// Path to local lookaside cache for instant BTRFS CoW source staging.
    pub lookaside_dir: Option<PathBuf>,
}

impl MockRunner {
    /// Creates a new MockRunner with a specific root profile name.
    pub fn new(root_name: impl Into<String>) -> Self {
        Self {
            root_name: root_name.into(),
            config_dir: None,
            unique_ext: None,
            local_repo_dir: None,
            lookaside_dir: None,
        }
    }

    /// Creates a MockRunner by automatically resolving a root profile name or .cfg file path.
    pub fn resolve(root_spec: &str, explicit_config_dir: Option<PathBuf>) -> Result<Self> {
        let resolved = crate::chroot::ChrootResolver::resolve(root_spec, explicit_config_dir.as_deref())?;
        let mut runner = Self::new(resolved.profile_name);
        if let Some(cfg_dir) = resolved.config_dir {
            runner = runner.with_config_dir(cfg_dir);
        }
        Ok(runner)
    }

    /// Sets the path to custom Mock configuration directory.
    pub fn with_config_dir(mut self, path: PathBuf) -> Self {
        self.config_dir = Some(path);
        self
    }

    /// Sets worker unique extension.
    pub fn with_unique_ext(mut self, ext: impl Into<String>) -> Self {
        self.unique_ext = Some(ext.into());
        self
    }

    /// Sets local repo directory for dynamic `--addrepo` injection.
    pub fn with_local_repo(mut self, repo_dir: PathBuf) -> Self {
        self.local_repo_dir = Some(repo_dir);
        self
    }

    /// Sets local lookaside cache directory for instant BTRFS CoW source staging.
    pub fn with_lookaside_dir(mut self, lookaside_dir: PathBuf) -> Self {
        self.lookaside_dir = Some(lookaside_dir);
        self
    }

    /// Builds a single package with worker isolation, automatic source acquisition, and two-stage Mock compilation.
    pub fn build_with_worker(
        &self,
        input_path: &Path,
        result_dir: &Path,
        worker_id: usize,
    ) -> Result<BuildOutput> {
        let pkg_stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("package");
        let pkg_result_dir = result_dir.join(format!("worker-{}-{}", worker_id, pkg_stem));
        fs::create_dir_all(&pkg_result_dir)?;

        let log_path = pkg_result_dir.join("build.log");
        let start_time = Instant::now();

        let is_srpm = input_path.to_string_lossy().ends_with(".src.rpm");

        // Stage 1: If input is a .spec file, ensure sources are present and compile SRPM
        let target_srpm = if is_srpm {
            input_path.to_path_buf()
        } else {
            let sources_dir = resolve_sources_dir(input_path);
            ensure_sources_present(input_path, &sources_dir, pkg_stem, self.lookaside_dir.as_deref());

            let mut srpm_cmd = Command::new("mock");
            srpm_cmd.arg(format!("-r={}", self.root_name));
            srpm_cmd.arg(format!("--resultdir={}", pkg_result_dir.display()));
            srpm_cmd.arg(format!("--uniqueext=w{}", worker_id));

            if let Some(cfg) = &self.config_dir {
                srpm_cmd.arg(format!("--configdir={}", cfg.display()));
            }

            srpm_cmd.arg("--buildsrpm");
            srpm_cmd.arg("--spec").arg(input_path);
            srpm_cmd.arg(format!("--sources={}", sources_dir.display()));
            srpm_cmd.arg("--symlink-dereference");

            let srpm_output = srpm_cmd.output()?;
            if !srpm_output.status.success() {
                let duration_seconds = start_time.elapsed().as_secs_f64();
                let log_content = format!(
                    "=== STAGE 1: mock --buildsrpm FAILED ===\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
                    String::from_utf8_lossy(&srpm_output.stdout),
                    String::from_utf8_lossy(&srpm_output.stderr)
                );
                let _ = fs::write(&log_path, log_content);

                let stderr_str = String::from_utf8_lossy(&srpm_output.stderr);
                let stdout_str = String::from_utf8_lossy(&srpm_output.stdout);
                let combined = format!("{}\n{}", stdout_str, stderr_str);
                let err_line = combined
                    .lines()
                    .find(|l| l.contains("error:") || l.contains("Bad file:") || l.contains("ERROR:"))
                    .unwrap_or("Mock --buildsrpm failed to produce SRPM")
                    .to_string();

                return Ok(BuildOutput {
                    target_name: pkg_stem.to_string(),
                    success: false,
                    duration_seconds,
                    log_path,
                    artifacts: Vec::new(),
                    error_summary: Some(err_line),
                });
            }

            match find_srpm_in_dir(&pkg_result_dir) {
                Some(f) => f,
                None => {
                    return Ok(BuildOutput {
                        target_name: pkg_stem.to_string(),
                        success: false,
                        duration_seconds: start_time.elapsed().as_secs_f64(),
                        log_path,
                        artifacts: Vec::new(),
                        error_summary: Some("Mock --buildsrpm succeeded but no .src.rpm was created".to_string()),
                    });
                }
            }
        };

        // Stage 2: Rebuild the SRPM inside the isolated Mock chroot
        let mut rebuild_cmd = Command::new("mock");
        rebuild_cmd.arg(format!("-r={}", self.root_name));
        rebuild_cmd.arg(format!("--resultdir={}", pkg_result_dir.display()));
        rebuild_cmd.arg(format!("--uniqueext=w{}", worker_id));

        if let Some(cfg) = &self.config_dir {
            rebuild_cmd.arg(format!("--configdir={}", cfg.display()));
        }

        if let Some(repo) = &self.local_repo_dir {
            if repo.exists() {
                let abs_repo = fs::canonicalize(repo).unwrap_or_else(|_| repo.clone());
                rebuild_cmd.arg(format!("--addrepo=file://{}", abs_repo.display()));
            }
        }

        rebuild_cmd.arg("--rebuild").arg(&target_srpm);

        let rebuild_output = rebuild_cmd.output()?;
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let success = rebuild_output.status.success();

        let log_content = format!(
            "=== STAGE 2: mock --rebuild ===\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            String::from_utf8_lossy(&rebuild_output.stdout),
            String::from_utf8_lossy(&rebuild_output.stderr)
        );
        let _ = fs::write(&log_path, log_content);

        let artifacts = collect_artifacts(&pkg_result_dir);
        let error_summary = if !success {
            let stderr_str = String::from_utf8_lossy(&rebuild_output.stderr);
            let stdout_str = String::from_utf8_lossy(&rebuild_output.stdout);
            let combined = format!("{}\n{}", stdout_str, stderr_str);
            let err_line = combined
                .lines()
                .find(|l| l.contains("error:") || l.contains("ERROR:") || l.contains("Failed:"))
                .unwrap_or("Mock --rebuild process exited with failure")
                .to_string();
            Some(err_line)
        } else {
            None
        };

        Ok(BuildOutput {
            target_name: pkg_stem.to_string(),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
        })
    }

    /// Executes sequential chain build for multiple interdependent packages (`mock --chain`).
    pub fn build_chain(
        &self,
        targets: &[PathBuf],
        result_dir: &Path,
        continue_on_error: bool,
    ) -> Result<BuildOutput> {
        fs::create_dir_all(result_dir)?;
        let log_path = result_dir.join("chain.log");
        let start_time = Instant::now();

        // Convert any .spec targets to .src.rpm first
        let mut srpms = Vec::new();
        let chain_srpms_dir = result_dir.join("chain_srpms");
        fs::create_dir_all(&chain_srpms_dir)?;

        for target in targets {
            if target.to_string_lossy().ends_with(".src.rpm") {
                srpms.push(target.clone());
            } else {
                let pkg_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                let sources_dir = resolve_sources_dir(target);
                ensure_sources_present(target, &sources_dir, pkg_stem, self.lookaside_dir.as_deref());

                let pkg_srpm_dir = chain_srpms_dir.join(pkg_stem);
                fs::create_dir_all(&pkg_srpm_dir)?;

                let mut srpm_cmd = Command::new("mock");
                srpm_cmd.arg(format!("-r={}", self.root_name));
                srpm_cmd.arg(format!("--resultdir={}", pkg_srpm_dir.display()));
                if let Some(cfg) = &self.config_dir {
                    srpm_cmd.arg(format!("--configdir={}", cfg.display()));
                }
                srpm_cmd.arg("--buildsrpm");
                srpm_cmd.arg("--spec").arg(target);
                srpm_cmd.arg(format!("--sources={}", sources_dir.display()));
                srpm_cmd.arg("--symlink-dereference");

                let srpm_output = srpm_cmd.output()?;
                if srpm_output.status.success() {
                    if let Some(srpm) = find_srpm_in_dir(&pkg_srpm_dir) {
                        srpms.push(srpm);
                    } else {
                        return Err(eyre!("Mock buildsrpm succeeded for {} but no .src.rpm was created", target.display()));
                    }
                } else {
                    let err = String::from_utf8_lossy(&srpm_output.stderr);
                    return Err(eyre!("Failed to build SRPM for chain build ({}): {}", target.display(), err));
                }
            }
        }

        let mut cmd = Command::new("mock");
        cmd.arg(format!("-r={}", self.root_name));
        cmd.arg("--chain");

        if let Some(cfg) = &self.config_dir {
            cmd.arg(format!("--configdir={}", cfg.display()));
        }

        for srpm in &srpms {
            cmd.arg(srpm);
        }

        if continue_on_error {
            cmd.arg("-c");
        }

        let local_repo = self
            .local_repo_dir
            .clone()
            .unwrap_or_else(|| result_dir.join("localrepo"));
        fs::create_dir_all(&local_repo)?;
        cmd.arg(format!("--localrepo={}", local_repo.display()));

        let output = cmd.output()?;
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let success = output.status.success();

        let log_content = format!(
            "STDOUT:\n{}\n\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::write(&log_path, log_content);

        let artifacts = collect_artifacts(&local_repo);
        let error_summary = if !success {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            Some(stderr_str.lines().last().unwrap_or("Mock chain build failed").to_string())
        } else {
            None
        };

        Ok(BuildOutput {
            target_name: format!("chain-{}pkgs", srpms.len()),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
        })
    }

    /// Builds multiple packages in parallel using Tokio worker concurrency and dynamic repository feedback.
    pub async fn build_parallel(
        self: Arc<Self>,
        targets: Vec<PathBuf>,
        result_dir: PathBuf,
        concurrency: usize,
        dynamic_repo: bool,
    ) -> Vec<Result<BuildOutput>> {
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let local_rpms_dir = result_dir.join("rpms").join("x86_64");
        let _ = fs::create_dir_all(&local_rpms_dir);

        if dynamic_repo {
            update_local_repo(&local_rpms_dir);
        }

        let mut tasks = Vec::new();

        for (idx, target) in targets.into_iter().enumerate() {
            let sem = semaphore.clone();
            let runner = self.clone();
            let res_dir = result_dir.clone();
            let worker_id = (idx % concurrency) + 1;
            let repo_dir = local_rpms_dir.clone();

            let task = tokio::task::spawn_blocking(move || {
                let _permit = sem.acquire_many(1);
                let out = runner.build_with_worker(&target, &res_dir, worker_id)?;

                // If build succeeded, stage artifacts and update local repo for other workers
                if out.success && dynamic_repo {
                    for art in &out.artifacts {
                        let filename = art.file_name().unwrap_or_default();
                        let dest = repo_dir.join(filename);
                        let _ = fs::copy(art, &dest);
                    }
                    update_local_repo(&repo_dir);
                }

                Ok(out)
            });

            tasks.push(task);
        }

        let mut results = Vec::new();
        for task in tasks {
            match task.await {
                Ok(res) => results.push(res),
                Err(e) => results.push(Err(eyre!("Worker thread panicked: {}", e))),
            }
        }

        results
    }
}

impl BuildRunner for MockRunner {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn build(&self, input_path: &Path, result_dir: &Path) -> Result<BuildOutput> {
        self.build_with_worker(input_path, result_dir, 1)
    }
}

/// Host-level rpmbuild runner executing `rpmbuild -ba` directly (for local debugging).
#[derive(Debug, Clone, Default)]
pub struct RpmbuildRunner;

impl BuildRunner for RpmbuildRunner {
    fn name(&self) -> &'static str {
        "rpmbuild"
    }

    fn build(&self, input_path: &Path, result_dir: &Path) -> Result<BuildOutput> {
        fs::create_dir_all(result_dir)?;
        let pkg_stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("package");
        let sources_dir = resolve_sources_dir(input_path);
        ensure_sources_present(input_path, &sources_dir, pkg_stem, None);
        let log_path = result_dir.join("rpmbuild.log");
        let start_time = Instant::now();

        let mut cmd = Command::new("rpmbuild");
        cmd.arg("-ba");
        cmd.arg(format!("--define=_topdir {}", result_dir.display()));
        cmd.arg(input_path);

        let output = cmd.output()?;
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let success = output.status.success();

        let log_content = format!(
            "STDOUT:\n{}\n\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::write(&log_path, log_content);

        let artifacts = collect_artifacts(result_dir);
        let error_summary = if !success {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            Some(stderr_str.lines().last().unwrap_or("rpmbuild exited with failure").to_string())
        } else {
            None
        };

        Ok(BuildOutput {
            target_name: pkg_stem.to_string(),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
        })
    }
}

/// Updates the repodata metadata for a local RPM repository directory using `createrepo_c`.
pub fn update_local_repo(repo_dir: &Path) {
    if !repo_dir.exists() {
        return;
    }

    let _guard = match REPO_LOCK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };

    let update_res = Command::new("createrepo_c")
        .arg("--update")
        .arg("--quiet")
        .arg(repo_dir)
        .status();

    if let Ok(st) = update_res {
        if st.success() {
            return;
        }
    }

    let _ = Command::new("createrepo_c")
        .arg("--quiet")
        .arg(repo_dir)
        .status();
}

/// Recursively discovers all generated `.rpm` and `.src.rpm` files within a directory.
fn collect_artifacts(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() {
        return files;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                files.extend(collect_artifacts(&p));
            } else if p.extension().and_then(|e| e.to_str()) == Some("rpm") {
                files.push(p);
            }
        }
    }
    files
}

/// Locates the appropriate sources directory for an RPM .spec file.
pub fn resolve_sources_dir(spec_path: &Path) -> PathBuf {
    if let Some(parent) = spec_path.parent() {
        let sources_sub = parent.join("SOURCES");
        if sources_sub.is_dir() {
            return sources_sub;
        }
        if let Some(grandparent) = parent.parent() {
            let gp_sources = grandparent.join("SOURCES");
            if gp_sources.is_dir() {
                return gp_sources;
            }
        }
        parent.to_path_buf()
    } else {
        PathBuf::from(".")
    }
}

/// Discovers an SRPM file (`*.src.rpm`) within a directory.
pub fn find_srpm_in_dir(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.to_string_lossy().ends_with(".src.rpm") {
                return Some(p);
            }
        }
    }
    None
}

/// Ensures all source files and patches required by a spec file are present locally.
pub fn ensure_sources_present(
    spec_path: &Path,
    sources_dir: &Path,
    pkg_name: &str,
    lookaside_dir: Option<&Path>,
) {
    let spec_dir = spec_path.parent().unwrap_or_else(|| Path::new("."));
    let lookaside_mgr = crate::lookaside::LookasideManager::resolve_default(lookaside_dir);

    // Check if a dist-git `sources` manifest exists in spec_dir or sources_dir
    let sources_manifest = if spec_dir.join("sources").is_file() {
        Some(spec_dir.join("sources"))
    } else if sources_dir.join("sources").is_file() {
        Some(sources_dir.join("sources"))
    } else {
        None
    };

    if let Some(manifest_path) = sources_manifest {
        if let Ok(entries) = crate::lookaside::cas::parse_sources_file(&manifest_path) {
            for entry in entries {
                let dest = sources_dir.join(&entry.filename);
                if !dest.exists() {
                    println!("  Sourcing archive '{}' for {} from lookaside...", entry.filename, pkg_name);
                    match lookaside_mgr.get_or_fetch(pkg_name, &entry.filename, &entry.hash, &dest) {
                        Ok(mode) => {
                            println!("  ✓ Staged '{}' via {}", entry.filename, mode);
                        }
                        Err(e) => {
                            println!("  Lookaside fetch failed ({}), trying spectool fallback...", e);
                            let _ = run_spectool_download(spec_path, sources_dir);
                        }
                    }
                } else {
                    // Archive exists locally; ensure it is mirrored in the lookaside cache
                    let cas_path = lookaside_mgr.cas_path_for_hash(&entry.hash);
                    if !cas_path.exists() {
                        let _ = lookaside_mgr.upload(&dest, pkg_name, Some(spec_path), false);
                    }
                }
            }
        }
    }

    // Always check spec metadata for missing source files
    let meta = match crate::distgit::spec::parse_spec_file(spec_path) {
        Ok(m) => m,
        Err(_) => return,
    };

    let mut has_missing = false;
    for src in &meta.sources {
        let filename = Path::new(src).file_name().and_then(|s| s.to_str()).unwrap_or(src);
        if !sources_dir.join(filename).exists() {
            has_missing = true;
            break;
        }
    }

    if has_missing {
        println!("  Attempting spectool download for missing sources of {}...", pkg_name);
        let ok = run_spectool_download(spec_path, sources_dir);
        if ok {
            for src in &meta.sources {
                let filename = Path::new(src).file_name().and_then(|s| s.to_str()).unwrap_or(src);
                let dest = sources_dir.join(filename);
                if dest.exists() {
                    let _ = lookaside_mgr.upload(&dest, pkg_name, Some(spec_path), false);
                }
            }
        } else {
            // Direct curl download for URL sources
            for src in &meta.sources {
                if src.starts_with("http://") || src.starts_with("https://") {
                    if let Some(fname) = src.split('/').last() {
                        let dest = sources_dir.join(fname);
                        if !dest.exists() {
                            println!("  Downloading source URL {} -> {}...", src, dest.display());
                            let dl_status = Command::new("curl")
                                .arg("-f")
                                .arg("-L")
                                .arg("-s")
                                .arg("-S")
                                .arg("--connect-timeout")
                                .arg("15")
                                .arg("-o")
                                .arg(&dest)
                                .arg(src)
                                .status();
                            if let Ok(st) = dl_status {
                                if st.success() && dest.exists() {
                                    let _ = lookaside_mgr.upload(&dest, pkg_name, Some(spec_path), false);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Runs `spectool -g -C <sources_dir> <spec_path>` to fetch remote source and patch URLs.
fn run_spectool_download(spec_path: &Path, sources_dir: &Path) -> bool {
    let status = Command::new("spectool")
        .arg("-g")
        .arg("-C")
        .arg(sources_dir)
        .arg(spec_path)
        .status();

    match status {
        Ok(st) => st.success(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_sources_dir() {
        let dir = tempdir().unwrap();
        let spec_path = dir.path().join("pkg.spec");
        File::create(&spec_path).unwrap();

        // Without SOURCES dir, resolves to parent
        assert_eq!(resolve_sources_dir(&spec_path), dir.path());

        // With SOURCES subdir
        let sources_dir = dir.path().join("SOURCES");
        fs::create_dir(&sources_dir).unwrap();
        assert_eq!(resolve_sources_dir(&spec_path), sources_dir);
    }

    #[test]
    fn test_find_srpm_in_dir() {
        let dir = tempdir().unwrap();
        assert!(find_srpm_in_dir(dir.path()).is_none());

        let srpm = dir.path().join("test-1.0-1.fc42.src.rpm");
        File::create(&srpm).unwrap();
        assert_eq!(find_srpm_in_dir(dir.path()), Some(srpm));
    }
}
