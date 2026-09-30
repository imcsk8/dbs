//! Build execution runners for DBS.
//!
//! Standardizes on **Mock** as the isolated, hermetic chroot build engine,
//! coupled with Rust's high-efficiency asynchronous concurrency system (`tokio::sync::Semaphore`)
//! for worker orchestration, dynamic local repository feedback, and sequential chain builds.
pub mod gate;

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use eyre::{eyre, Result};
use tokio::sync::Semaphore;

static REPO_LOCK: Mutex<()> = Mutex::new(());

/// Maximum number of retry attempts when Mock encounters build root lock contention.
const MAX_LOCK_RETRIES: usize = 5;

/// Delay in seconds between lock contention retry attempts.
const LOCK_RETRY_DELAY_SECS: u64 = 3;

/// A RAII lease guard that holds an exclusive worker ID from the active concurrency pool.
///
/// When the lease is dropped (whether normally, on error, or during thread panic unwinding),
/// the leased `worker_id` is automatically returned to the available worker pool.
#[derive(Debug)]
pub struct WorkerLease {
    pool: Arc<Mutex<VecDeque<usize>>>,
    worker_id: usize,
}

impl WorkerLease {
    /// Creates a new worker lease with the specified ID and owning pool.
    pub fn new(pool: Arc<Mutex<VecDeque<usize>>>, worker_id: usize) -> Self {
        Self { pool, worker_id }
    }

    /// Returns the unique worker ID associated with this active lease.
    pub fn id(&self) -> usize {
        self.worker_id
    }
}

impl Drop for WorkerLease {
    fn drop(&mut self) {
        match self.pool.lock() {
            Ok(mut p) => p.push_back(self.worker_id),
            Err(poisoned) => poisoned.into_inner().push_back(self.worker_id),
        }
    }
}

/// Core implementation for executing a Mock command with configurable retries and delay on lock contention.
pub(crate) fn execute_mock_with_lock_retry_impl(
    cmd: &mut Command,
    pkg_name: &str,
    worker_id: usize,
    max_retries: usize,
    retry_delay: Duration,
) -> std::io::Result<std::process::Output> {
    for attempt in 1..=max_retries {
        let output = cmd.output()?;

        if output.status.success() {
            return Ok(output);
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let is_locked = stderr.contains("locked by another process")
            || stdout.contains("locked by another process")
            || stderr.contains("Build root is locked")
            || stdout.contains("Build root is locked");

        if is_locked && attempt < max_retries {
            eprintln!(
                "⚠️  [{}] Worker {} chroot is locked by another process (attempt {}/{}). Retrying in {:.1}s...",
                pkg_name,
                worker_id,
                attempt,
                max_retries,
                retry_delay.as_secs_f32()
            );
            std::thread::sleep(retry_delay);
            continue;
        }

        return Ok(output);
    }
    cmd.output()
}

/// Executes a Mock command, automatically retrying with backoff if the build root is locked by another process.
fn execute_mock_with_lock_retry(
    cmd: &mut Command,
    pkg_name: &str,
    worker_id: usize,
) -> std::io::Result<std::process::Output> {
    execute_mock_with_lock_retry_impl(
        cmd,
        pkg_name,
        worker_id,
        MAX_LOCK_RETRIES,
        Duration::from_secs(LOCK_RETRY_DELAY_SECS),
    )
}

/// Detailed diagnostic context extracted from build failure logs.
#[derive(Debug, Clone)]
pub struct ErrorDiagnostic {
    /// Concise summary line (e.g. "error: unknown type name 'foo_t' [%build]").
    pub summary: String,
    /// Detected failure phase (e.g. "%build", "%check", "%install", "%files", "%prep", "builddep", "buildsrpm").
    pub phase: Option<String>,
    /// Relevant log snippet (up to 25 lines) showing the failure context.
    pub context_lines: Vec<String>,
    /// Path to the source log file containing the error.
    pub source_log: PathBuf,
}

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
    /// Detailed error diagnostic for auto-summary display.
    pub error_diagnostic: Option<ErrorDiagnostic>,
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
    /// Optional database connection URL to record build lifecycle states.
    pub db_url: Option<String>,
    /// Optional number of SMP compilation threads passed to Mock (%_smp_mflags, %_smp_build_ncpus).
    pub smp_cpus: Option<usize>,
    /// Whether to disable test execution in Mock and rpmbuild (`--nocheck`).
    pub nocheck: bool,
    /// Specific package names configured to skip %check test execution.
    pub nocheck_packages: Vec<String>,
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
            db_url: None,
            smp_cpus: None,
            nocheck: false,
            nocheck_packages: Vec::new(),
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

    /// Sets database URL for recording build lifecycle events.
    pub fn with_db_url(mut self, db_url: impl Into<String>) -> Self {
        self.db_url = Some(db_url.into());
        self
    }

    /// Sets the number of SMP compilation threads passed to Mock inside the chroot.
    pub fn with_smp_cpus(mut self, smp_cpus: usize) -> Self {
        self.smp_cpus = Some(smp_cpus);
        self
    }

    /// Enables or disables skipping the %check test suite phase in Mock and rpmbuild (`--nocheck`).
    pub fn with_nocheck(mut self, nocheck: bool) -> Self {
        self.nocheck = nocheck;
        self
    }

    /// Configures the specific list of package names that skip %check execution.
    pub fn with_nocheck_packages<I, S>(mut self, packages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.nocheck_packages = packages.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a single package name to the %check skip exemption list.
    pub fn add_nocheck_package(mut self, package: impl Into<String>) -> Self {
        self.nocheck_packages.push(package.into());
        self
    }

    /// Determines whether a package target or stem should skip %check tests.
    pub fn is_nocheck_package(&self, pkg_name: &str) -> bool {
        let clean = extract_nocheck_modifier(pkg_name).0;
        self.nocheck_packages.iter().any(|p| {
            let p_clean = extract_nocheck_modifier(p).0;
            p_clean.eq_ignore_ascii_case(clean)
        })
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

        // Record build started state in database
        if let Some(db_url) = &self.db_url
            && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                let spec_meta = crate::distgit::spec::parse_spec_file(input_path).ok();
                let _ = crate::db::record_build_start(
                    &mut conn,
                    pkg_stem,
                    Some(worker_id as i32),
                    Some(&log_path.display().to_string()),
                    spec_meta.as_ref(),
                );
            }

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

            if let Some(smp) = self.smp_cpus {
                srpm_cmd.arg("-D").arg(format!("_smp_mflags -j{}", smp));
                srpm_cmd.arg("-D").arg(format!("_smp_build_ncpus {}", smp));
                srpm_cmd.arg("-D").arg(format!("_smp_ncpus_max {}", smp));
            }

            srpm_cmd.arg("--buildsrpm");
            srpm_cmd.arg("--spec").arg(input_path);
            srpm_cmd.arg(format!("--sources={}", sources_dir.display()));
            srpm_cmd.arg("--symlink-dereference");

            let srpm_output = match execute_mock_with_lock_retry(&mut srpm_cmd, pkg_stem, worker_id) {
                Ok(out) => out,
                Err(e) => return Err(eyre!("Failed to execute mock --buildsrpm for {}: {}", pkg_stem, e)),
            };
            if !srpm_output.status.success() {
                let duration_seconds = start_time.elapsed().as_secs_f64();
                let runner_log_path = pkg_result_dir.join("dbs-runner.log");
                let runner_log_content = format!(
                    "=== DBS STAGE 1: mock --buildsrpm FAILED ===\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
                    String::from_utf8_lossy(&srpm_output.stdout),
                    String::from_utf8_lossy(&srpm_output.stderr)
                );
                let _ = fs::write(&runner_log_path, &runner_log_content);

                // If Mock did not generate build.log, write runner output to build.log so log_path exists
                if !log_path.exists() {
                    let _ = fs::write(&log_path, &runner_log_content);
                }

                let (error_summary, error_diagnostic) = extract_error_diagnostic(
                    &pkg_result_dir,
                    &log_path,
                    &String::from_utf8_lossy(&srpm_output.stdout),
                    &String::from_utf8_lossy(&srpm_output.stderr),
                );

                let out = BuildOutput {
                    target_name: pkg_stem.to_string(),
                    success: false,
                    duration_seconds,
                    log_path,
                    artifacts: Vec::new(),
                    error_summary: Some(error_summary),
                    error_diagnostic,
                };
                if let Some(db_url) = &self.db_url
                    && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                        let _ = crate::db::record_build_result(&mut conn, pkg_stem, &out);
                    }
                return Ok(out);
            }

            match find_srpm_in_dir(&pkg_result_dir) {
                Some(f) => f,
                None => {
                    let out = BuildOutput {
                        target_name: pkg_stem.to_string(),
                        success: false,
                        duration_seconds: start_time.elapsed().as_secs_f64(),
                        log_path,
                        artifacts: Vec::new(),
                        error_summary: Some("Mock --buildsrpm succeeded but no .src.rpm was created".to_string()),
                        error_diagnostic: None,
                    };
                    if let Some(db_url) = &self.db_url
                        && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                            let _ = crate::db::record_build_result(&mut conn, pkg_stem, &out);
                        }
                    return Ok(out);
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

        if let Some(repo) = &self.local_repo_dir
            && repo.exists() {
                let abs_repo = fs::canonicalize(repo).unwrap_or_else(|_| repo.clone());
                rebuild_cmd.arg(format!("--addrepo=file://{}", abs_repo.display()));
            }

        let staging_repo = result_dir.join("rpms").join("x86_64");
        if staging_repo.exists() && self.local_repo_dir.as_ref() != Some(&staging_repo) {
            let abs_staging = fs::canonicalize(&staging_repo).unwrap_or(staging_repo);
            rebuild_cmd.arg(format!("--addrepo=file://{}", abs_staging.display()));
        }

        if let Some(smp) = self.smp_cpus {
            rebuild_cmd.arg("-D").arg(format!("_smp_mflags -j{}", smp));
            rebuild_cmd.arg("-D").arg(format!("_smp_build_ncpus {}", smp));
            rebuild_cmd.arg("-D").arg(format!("_smp_ncpus_max {}", smp));
        }

        let should_skip_check = self.nocheck || self.is_nocheck_package(pkg_stem);
        if should_skip_check {
            rebuild_cmd.arg("--nocheck");
            if !self.nocheck {
                println!("  [worker-{}] Skipping %check test suite for '{}' (matched nocheck_packages)", worker_id, pkg_stem);
            }
        }

        rebuild_cmd.arg("--rebuild").arg(&target_srpm);

        let rebuild_output = match execute_mock_with_lock_retry(&mut rebuild_cmd, pkg_stem, worker_id) {
            Ok(out) => out,
            Err(e) => return Err(eyre!("Failed to execute mock --rebuild for {}: {}", pkg_stem, e)),
        };
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let success = rebuild_output.status.success();

        // Write DBS wrapper stdout/stderr to dbs-runner.log for separate diagnosis
        let runner_log_path = pkg_result_dir.join("dbs-runner.log");
        let runner_log_content = format!(
            "=== DBS STAGE 2: mock --rebuild ===\n\nSTDOUT:\n{}\n\nSTDERR:\n{}",
            String::from_utf8_lossy(&rebuild_output.stdout),
            String::from_utf8_lossy(&rebuild_output.stderr)
        );
        let _ = fs::write(&runner_log_path, &runner_log_content);

        // Preserve Mock's detailed build.log if generated; otherwise fallback to runner log
        if !log_path.exists() {
            let _ = fs::write(&log_path, &runner_log_content);
        }

        let artifacts = collect_artifacts(&pkg_result_dir);
        let (error_summary, error_diagnostic) = if !success {
            let stdout_str = String::from_utf8_lossy(&rebuild_output.stdout);
            let stderr_str = String::from_utf8_lossy(&rebuild_output.stderr);
            let (s, d) = extract_error_diagnostic(&pkg_result_dir, &log_path, &stdout_str, &stderr_str);
            (Some(s), d)
        } else {
            (None, None)
        };

        let out = BuildOutput {
            target_name: pkg_stem.to_string(),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
            error_diagnostic,
        };

        if let Some(db_url) = &self.db_url
            && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                let _ = crate::db::record_build_result(&mut conn, pkg_stem, &out);
            }

        Ok(out)
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

        if let Some(db_url) = &self.db_url
            && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                for target in targets {
                    let pkg_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
                    let spec_meta = crate::distgit::spec::parse_spec_file(target).ok();
                    let _ = crate::db::record_build_start(
                        &mut conn,
                        pkg_stem,
                        Some(1),
                        Some(&log_path.display().to_string()),
                        spec_meta.as_ref(),
                    );
                }
            }

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
                if let Some(smp) = self.smp_cpus {
                    srpm_cmd.arg("-D").arg(format!("_smp_mflags -j{}", smp));
                    srpm_cmd.arg("-D").arg(format!("_smp_build_ncpus {}", smp));
                    srpm_cmd.arg("-D").arg(format!("_smp_ncpus_max {}", smp));
                }
                srpm_cmd.arg("--buildsrpm");
                srpm_cmd.arg("--spec").arg(target);
                srpm_cmd.arg(format!("--sources={}", sources_dir.display()));
                srpm_cmd.arg("--symlink-dereference");

                let srpm_output = match execute_mock_with_lock_retry(&mut srpm_cmd, pkg_stem, 1) {
                    Ok(out) => out,
                    Err(e) => return Err(eyre!("Failed to execute mock --buildsrpm for chain build ({}): {}", target.display(), e)),
                };
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

        if let Some(smp) = self.smp_cpus {
            cmd.arg("-D").arg(format!("_smp_mflags -j{}", smp));
            cmd.arg("-D").arg(format!("_smp_build_ncpus {}", smp));
            cmd.arg("-D").arg(format!("_smp_ncpus_max {}", smp));
        }

        let any_nocheck = self.nocheck || targets.iter().any(|t| {
            let stem = t.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            self.is_nocheck_package(stem)
        });
        if any_nocheck {
            cmd.arg("--nocheck");
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

        let output = match execute_mock_with_lock_retry(&mut cmd, "chain-build", 1) {
            Ok(out) => out,
            Err(e) => return Err(eyre!("Failed to execute mock --chain: {}", e)),
        };
        let duration_seconds = start_time.elapsed().as_secs_f64();
        let success = output.status.success();

        let log_content = format!(
            "STDOUT:\n{}\n\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = fs::write(&log_path, log_content);

        let artifacts = collect_artifacts(&local_repo);
        let (error_summary, error_diagnostic) = if !success {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let (s, d) = extract_error_diagnostic(result_dir, &log_path, &stdout_str, &stderr_str);
            (Some(s), d)
        } else {
            (None, None)
        };

        let out = BuildOutput {
            target_name: format!("chain-{}pkgs", srpms.len()),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
            error_diagnostic,
        };

        if let Some(db_url) = &self.db_url
            && let Ok(mut conn) = crate::db::establish_connection_with_url(Some(db_url)) {
                for target in targets {
                    let pkg_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("package");
                    let _ = crate::db::record_build_result(&mut conn, pkg_stem, &out);
                }
            }

        Ok(out)
    }

    /// Builds multiple packages in parallel using Tokio worker concurrency and dynamic repository feedback.
    pub async fn build_parallel(
        self: Arc<Self>,
        targets: Vec<PathBuf>,
        result_dir: PathBuf,
        concurrency: usize,
        dynamic_repo: bool,
    ) -> Vec<Result<BuildOutput>> {
        let effective_concurrency = concurrency.max(1);
        let semaphore = Arc::new(Semaphore::new(effective_concurrency));
        let worker_pool = Arc::new(Mutex::new((1..=effective_concurrency).collect::<VecDeque<usize>>()));
        let local_rpms_dir = result_dir.join("rpms").join("x86_64");
        let _ = fs::create_dir_all(&local_rpms_dir);

        if dynamic_repo {
            update_local_repo(&local_rpms_dir);
        }

        let mut tasks = Vec::new();

        for target in targets {
            let sem = semaphore.clone();
            let pool = worker_pool.clone();
            let runner = self.clone();
            let res_dir = result_dir.clone();
            let repo_dir = local_rpms_dir.clone();

            let permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(e) => {
                    tasks.push(tokio::task::spawn_blocking(move || {
                        Err(eyre!("Failed to acquire worker semaphore permit: {}", e))
                    }));
                    continue;
                }
            };

            let worker_id = {
                let mut guard = match pool.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                guard.pop_front().unwrap_or(1)
            };

            let lease = WorkerLease::new(pool, worker_id);

            let task = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let _lease = lease;
                let worker_id = _lease.id();

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
pub struct RpmbuildRunner {
    /// Whether to disable test execution in rpmbuild (`--nocheck`).
    pub nocheck: bool,
    /// Specific package names configured to skip %check test execution.
    pub nocheck_packages: Vec<String>,
}

impl RpmbuildRunner {
    /// Enables or disables skipping the %check phase in rpmbuild (`--nocheck`).
    pub fn with_nocheck(mut self, nocheck: bool) -> Self {
        self.nocheck = nocheck;
        self
    }

    /// Configures the specific list of package names that skip %check execution.
    pub fn with_nocheck_packages<I, S>(mut self, packages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.nocheck_packages = packages.into_iter().map(Into::into).collect();
        self
    }

    /// Determines whether a package target or stem should skip %check tests.
    pub fn is_nocheck_package(&self, pkg_name: &str) -> bool {
        let clean = extract_nocheck_modifier(pkg_name).0;
        self.nocheck_packages.iter().any(|p| {
            let p_clean = extract_nocheck_modifier(p).0;
            p_clean.eq_ignore_ascii_case(clean)
        })
    }
}

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
        if self.nocheck || self.is_nocheck_package(pkg_stem) {
            cmd.arg("--nocheck");
        }
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
        let (error_summary, error_diagnostic) = if !success {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let (s, d) = extract_error_diagnostic(result_dir, &log_path, &stdout_str, &stderr_str);
            (Some(s), d)
        } else {
            (None, None)
        };

        Ok(BuildOutput {
            target_name: pkg_stem.to_string(),
            success,
            duration_seconds,
            log_path,
            artifacts,
            error_summary,
            error_diagnostic,
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

    if let Ok(st) = update_res
        && st.success() {
            return;
        }

    let _ = Command::new("createrepo_c")
        .arg("--quiet")
        .arg(repo_dir)
        .status();
}

/// Reads up to `max_bytes` from the tail of a file, returning its lines.
fn read_tail_lines(path: &Path, max_bytes: u64) -> std::io::Result<Vec<String>> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    let file_len = metadata.len();
    let read_start = file_len.saturating_sub(max_bytes);

    match file.seek(SeekFrom::Start(read_start)) {
        Ok(_) => (),
        Err(e) => return Err(e),
    };

    let mut buffer = String::new();
    match file.read_to_string(&mut buffer) {
        Ok(_) => (),
        Err(e) => return Err(e),
    };

    Ok(buffer.lines().map(|s| s.to_string()).collect())
}

/// Extracts a concise, actionable error summary line from Mock's `build.log` or runner output.
fn extract_error_summary(log_path: &Path, wrapper_stdout: &str, wrapper_stderr: &str) -> String {
    if log_path.exists()
        && let Ok(lines) = read_tail_lines(log_path, 1024 * 1024) {
            let mut bad_exit_phase = None;
            let mut specific_cause = None;

            for line in lines.iter().rev() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Detect rpmbuild phase failure, e.g. (%check), (%build), (%install)
                if bad_exit_phase.is_none() && trimmed.contains("error: Bad exit status") {
                    if let Some(phase_start) = trimmed.rfind("(%") {
                        bad_exit_phase = Some(trimmed[phase_start..].trim_end_matches('.').to_string());
                    } else {
                        bad_exit_phase = Some(trimmed.to_string());
                    }
                    continue;
                }

                // Detect specific test failure, compiler error, or packaging error
                if trimmed.contains(": fatal error: ")
                    || trimmed.contains(": error: ")
                    || (trimmed.starts_with("error: ") && !trimmed.contains("error: Bad exit status"))
                    || trimmed.starts_with("FAIL: ")
                    || trimmed.starts_with("FAILED: ")
                {
                    specific_cause = Some(trimmed.to_string());
                    if bad_exit_phase.is_some() {
                        break;
                    }
                } else if specific_cause.is_none()
                    && ((trimmed.contains("make: *** [") && trimmed.contains("Error"))
                        || (trimmed.contains("make[") && trimmed.contains("Error "))
                        || trimmed.contains("ninja: build stopped:")
                        || trimmed.starts_with("CMake Error at"))
                {
                    specific_cause = Some(trimmed.to_string());
                }
            }

            match (bad_exit_phase, specific_cause) {
                (Some(phase), Some(cause)) => return format!("{} [{}]", cause, phase),
                (Some(phase), None) => return format!("rpmbuild failed at {}", phase),
                (None, Some(cause)) => return cause,
                (None, None) => {}
            }
        }

    // Fall back to Mock CLI wrapper output
    let combined = format!("{}\n{}", wrapper_stdout, wrapper_stderr);
    for line in combined.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("ERROR: Command failed:") {
            return trimmed.to_string();
        }
        if (trimmed.contains("error:") || trimmed.contains("ERROR:") || trimmed.contains("Failed:"))
            && !trimmed.contains("Start:")
            && !trimmed.contains("Finish:")
            && !trimmed.contains("Cleaning up build root")
        {
            return trimmed.to_string();
        }
    }

    "Mock process exited with failure".to_string()
}

/// Extracts both a concise summary and a contextual log excerpt for rapid failure diagnosis.
pub fn extract_error_diagnostic(
    pkg_result_dir: &Path,
    log_path: &Path,
    wrapper_stdout: &str,
    wrapper_stderr: &str,
) -> (String, Option<ErrorDiagnostic>) {
    let summary = extract_error_summary(log_path, wrapper_stdout, wrapper_stderr);
    let mut phase = None;
    if let Some(pos) = summary.rfind("(%")
        && let Some(end) = summary[pos..].find(')') {
            phase = Some(summary[pos + 1..pos + end].to_string());
    } else if let Some(pos) = summary.rfind("[%")
        && let Some(end) = summary[pos..].find(']') {
            phase = Some(summary[pos + 1..pos + end].to_string());
    }

    // 1. First attempt: search build.log for the failure context
    if log_path.exists()
        && let Ok(lines) = read_tail_lines(log_path, 2 * 1024 * 1024)
        && !lines.is_empty() {
            let mut anchor_idx = None;
            for (idx, line) in lines.iter().enumerate().rev() {
                let trimmed = line.trim();
                if trimmed.contains("error: Bad exit status")
                    || trimmed.contains("RPM build errors:")
                    || trimmed.contains("Installed (but unpackaged) file(s) found:")
                    || trimmed.starts_with("File not found:")
                    || trimmed.starts_with("FAIL: ")
                    || trimmed.starts_with("FAILED: ")
                    || trimmed.contains(": fatal error: ")
                    || (trimmed.starts_with("error: ") && !trimmed.contains("error: Bad exit status"))
                    || trimmed.contains("ninja: build stopped:")
                    || (trimmed.contains("make: *** [") && trimmed.contains("Error"))
                    || (trimmed.contains("make[") && trimmed.contains("Error ")) {
                        anchor_idx = Some(idx);
                        break;
                    }
            }

            let excerpt = if let Some(anchor) = anchor_idx {
                let start = anchor.saturating_sub(18);
                let end = (anchor + 6).min(lines.len());
                lines[start..end].to_vec()
            } else {
                let start = lines.len().saturating_sub(20);
                lines[start..].to_vec()
            };

            let diagnostic = ErrorDiagnostic {
                summary: summary.clone(),
                phase: phase.clone(),
                context_lines: excerpt,
                source_log: log_path.to_path_buf(),
            };
            return (summary, Some(diagnostic));
        }

    // 2. Second attempt: check root.log for dependency or environment errors
    let root_log = pkg_result_dir.join("root.log");
    if root_log.exists()
        && let Ok(root_lines) = read_tail_lines(&root_log, 1024 * 1024)
        && !root_lines.is_empty() {
            let mut problem_idx = None;
            for (idx, line) in root_lines.iter().enumerate().rev() {
                let trimmed = line.trim();
                if trimmed.contains("No match for argument:")
                    || trimmed.contains("Problem:")
                    || trimmed.contains("conflicting requests")
                    || trimmed.contains("nothing provides")
                    || trimmed.starts_with("Error: Problem") {
                        problem_idx = Some(idx);
                        break;
                    }
            }

            if let Some(anchor) = problem_idx {
                let start = anchor.saturating_sub(10);
                let end = (anchor + 12).min(root_lines.len());
                let excerpt = root_lines[start..end].to_vec();
                let diag = ErrorDiagnostic {
                    summary: summary.clone(),
                    phase: Some("builddep / dnf".to_string()),
                    context_lines: excerpt,
                    source_log: root_log,
                };
                return (summary, Some(diag));
            }
        }

    // 3. Third attempt: check dbs-runner.log
    let runner_log = pkg_result_dir.join("dbs-runner.log");
    if runner_log.exists()
        && let Ok(runner_lines) = read_tail_lines(&runner_log, 512 * 1024)
        && !runner_lines.is_empty() {
            let start = runner_lines.len().saturating_sub(20);
            let excerpt = runner_lines[start..].to_vec();
            let diag = ErrorDiagnostic {
                summary: summary.clone(),
                phase: phase.or(Some("mock-runner".to_string())),
                context_lines: excerpt,
                source_log: runner_log,
            };
            return (summary, Some(diag));
        }

    // Fall back to wrapper stdout/stderr lines
    let combined = format!("{}\n{}", wrapper_stdout, wrapper_stderr);
    let lines: Vec<String> = combined.lines().map(|s| s.to_string()).collect();
    if !lines.is_empty() {
        let start = lines.len().saturating_sub(20);
        let excerpt = lines[start..].to_vec();
        let diag = ErrorDiagnostic {
            summary: summary.clone(),
            phase: phase.or(Some("runner".to_string())),
            context_lines: excerpt,
            source_log: log_path.to_path_buf(),
        };
        return (summary, Some(diag));
    }

    (summary, None)
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

/// Extracts the clean download URL (without RPM `#/filename` URL fragments)
/// and the destination filename for a source or patch entry.
pub fn parse_source_entry(raw_src: &str) -> (String, String) {
    let trimmed = raw_src.trim();
    let (url_part, raw_filename) = if let Some((url, frag)) = trimmed.split_once('#') {
        let frag_name = frag.trim_start_matches('/').trim();
        let fname = if !frag_name.is_empty() {
            frag_name.to_string()
        } else {
            url.trim().split('/').next_back().unwrap_or(url).to_string()
        };
        (url.trim(), fname)
    } else {
        (trimmed, trimmed.split('/').next_back().unwrap_or(trimmed).to_string())
    };

    // If filename has a query string (e.g. ?foo=bar), take only the part before ?
    let filename = match raw_filename.split_once('?') {
        Some((before, _)) if !before.is_empty() => before.to_string(),
        _ => raw_filename,
    };

    (url_part.to_string(), filename)
}

/// Ensures all source files and patches required by a spec file are present locally.
pub fn ensure_sources_present(
    spec_path: &Path,
    sources_dir: &Path,
    pkg_name: &str,
    lookaside_dir: Option<&Path>,
) {
    let _ = fs::create_dir_all(sources_dir);
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

    if let Some(manifest_path) = sources_manifest
        && let Ok(entries) = crate::lookaside::cas::parse_sources_file(&manifest_path) {
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

    // Always check spec metadata for missing source files
    let meta = match crate::distgit::spec::parse_spec_file(spec_path) {
        Ok(m) => m,
        Err(_) => return,
    };

    // Collect all sources and remote patches
    let mut all_items: Vec<&String> = Vec::new();
    for src in &meta.sources {
        all_items.push(src);
    }
    for patch in &meta.patches {
        if patch.starts_with("http://") || patch.starts_with("https://") {
            all_items.push(patch);
        }
    }

    let mut has_missing = false;
    for item in &all_items {
        let (_, filename) = parse_source_entry(item);
        if !sources_dir.join(&filename).exists() {
            has_missing = true;
            break;
        }
    }

    if has_missing {
        println!("  Attempting spectool download for missing sources of {}...", pkg_name);
        let _ = run_spectool_download(spec_path, sources_dir);

        // Fallback to direct curl download for any remaining missing remote sources or patches
        for item in &all_items {
            let (download_url, filename) = parse_source_entry(item);
            let dest = sources_dir.join(&filename);

            if !dest.exists() && (download_url.starts_with("http://") || download_url.starts_with("https://")) {
                if let Some(parent) = dest.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                println!("  Downloading source URL {} -> {}...", download_url, dest.display());
                let dl_status = Command::new("curl")
                    .arg("-f")
                    .arg("-L")
                    .arg("-s")
                    .arg("-S")
                    .arg("--connect-timeout")
                    .arg("15")
                    .arg("-o")
                    .arg(&dest)
                    .arg(&download_url)
                    .status();
                if let Err(e) = dl_status {
                    eprintln!("  Failed to execute curl for {}: {}", download_url, e);
                }
            }

            if dest.exists() {
                let _ = lookaside_mgr.upload(&dest, pkg_name, Some(spec_path), false);
            }
        }
    }
}

/// Runs `spectool -g -C <sources_dir> <spec_path>` to fetch remote source and patch URLs.
fn run_spectool_download(spec_path: &Path, sources_dir: &Path) -> bool {
    let _ = fs::create_dir_all(sources_dir);
    let abs_sources = fs::canonicalize(sources_dir).unwrap_or_else(|_| sources_dir.to_path_buf());
    let status = Command::new("spectool")
        .arg("--define")
        .arg(format!("_sourcedir {}", abs_sources.display()))
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

/// Discovers a `.spec` file within a directory.
pub fn find_spec_in_dir(dir: &Path) -> Option<PathBuf> {
    if let Some(dir_name) = dir.file_name().and_then(|n| n.to_str()) {
        let expected = dir.join(format!("{}.spec", dir_name));
        if expected.is_file() {
            return Some(expected);
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("spec") {
                return Some(p);
            }
        }
    }
    None
}

/// Extracts a clean package target name/path and whether the `:nocheck` modifier was present.
///
/// For example:
/// - `"git:nocheck"` -> `("git", true)`
/// - `"/srv/dbs/tacos/rpm/git/git.spec:nocheck"` -> `("/srv/dbs/tacos/rpm/git/git.spec", true)`
/// - `"curl"` -> `("curl", false)`
pub fn extract_nocheck_modifier(entry: &str) -> (&str, bool) {
    if let Some(stripped) = entry.strip_suffix(":nocheck") {
        (stripped, true)
    } else {
        (entry, false)
    }
}

/// Resolves a package entry (which can be an explicit .spec / .src.rpm path, a directory,
/// or a package name) to a valid spec or SRPM file path on disk.
/// Automatically handles and ignores `:nocheck` modifiers if present.
pub fn resolve_package_target(entry: &str, distgit_dest: &Path) -> Result<PathBuf> {
    let (clean_entry, _) = extract_nocheck_modifier(entry);
    let p = Path::new(clean_entry);

    // 1. Direct file path check (e.g. "/path/to/pkg.spec" or "specs/pkg.spec" or "pkg.src.rpm")
    if p.is_file() {
        return Ok(p.to_path_buf());
    }

    // 2. Check inside dist-git destination root (e.g. distgit_dest/pkg/pkg.spec or distgit_dest/pkg/*.spec)
    let in_distgit_dir = distgit_dest.join(clean_entry);
    if in_distgit_dir.is_dir()
        && let Some(spec) = find_spec_in_dir(&in_distgit_dir) {
            return Ok(spec);
        }

    // Check distgit_dest/pkg.spec directly
    let in_distgit_spec = distgit_dest.join(format!("{}.spec", clean_entry));
    if in_distgit_spec.is_file() {
        return Ok(in_distgit_spec);
    }

    // 3. Check common relative directories (e.g. specs/pkg/pkg.spec, specs/pkg.spec, data/distgit/pkg/...)
    let specs_dir = Path::new("specs").join(clean_entry);
    if specs_dir.is_dir()
        && let Some(spec) = find_spec_in_dir(&specs_dir) {
            return Ok(spec);
        }
    let specs_file = Path::new("specs").join(format!("{}.spec", clean_entry));
    if specs_file.is_file() {
        return Ok(specs_file);
    }

    let default_distgit = Path::new("data/distgit").join(clean_entry);
    if default_distgit.is_dir()
        && let Some(spec) = find_spec_in_dir(&default_distgit) {
            return Ok(spec);
        }

    // 4. Check if entry + .spec exists relative to current dir
    let local_spec = PathBuf::from(format!("{}.spec", clean_entry));
    if local_spec.is_file() {
        return Ok(local_spec);
    }

    // 5. Direct directory check (e.g. "/path/to/pkg/" or "specs/pkg/")
    if p.is_dir() {
        if let Some(spec) = find_spec_in_dir(p) {
            return Ok(spec);
        }
        return Err(eyre!("Directory '{}' does not contain any .spec file", p.display()));
    }

    // 6. Check if clean_entry is a subpackage or capability provided by any local .spec
    let local_providers = crate::comps::scan_local_spec_providers(distgit_dest);
    if let Some(spec) = local_providers.get(clean_entry) {
        return Ok(spec.clone());
    }

    // 7. Check if clean_entry is a binary subpackage whose source RPM is known via repoquery
    let src_map = crate::comps::query_source_package_names(&[clean_entry.to_string()], None);
    if let Some(src_name) = src_map.get(clean_entry)
        && src_name != clean_entry {
            let src_dir = distgit_dest.join(src_name);
            if src_dir.is_dir() && let Some(spec) = find_spec_in_dir(&src_dir) {
                return Ok(spec);
            }
            let src_spec = distgit_dest.join(format!("{}.spec", src_name));
            if src_spec.is_file() {
                return Ok(src_spec);
            }
    }

    Err(eyre!(
        "Package or spec file '{}' not found (checked '{}' and '{}')",
        entry,
        p.display(),
        in_distgit_dir.display()
    ))
}

/// Loads package targets from a text file (one package per line).
///
/// Blank lines and comments starting with `#` are ignored.
/// Resolves package names, directories, or direct spec/srpm paths.
pub fn load_packages_from_file(file_path: &Path, distgit_dest: &Path) -> Result<Vec<PathBuf>> {
    let content = fs::read_to_string(file_path)
        .map_err(|e| eyre!("Failed to read packages file {}: {}", file_path.display(), e))?;
    parse_packages_list(&content, distgit_dest, file_path)
}

/// Parses a newline-delimited packages list from a string.
pub fn parse_packages_list(content: &str, distgit_dest: &Path, file_path: &Path) -> Result<Vec<PathBuf>> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();

    for (line_no, raw_line) in content.lines().enumerate() {
        let trimmed = raw_line.trim();
        // Remove trailing comments if present
        let clean = match trimmed.split_once('#') {
            Some((before, _)) => before.trim(),
            None => trimmed,
        };

        if clean.is_empty() {
            continue;
        }

        // Skip XML comments and non-package XML tags (e.g. <!-- ... -->, <packages>, </image>)
        if clean.starts_with('<') && !clean.contains("<package") {
            continue;
        }

        let entry = crate::comps::extract_package_entry(clean);
        if crate::comps::is_comps_target(entry) {
            let resolver = crate::comps::CompsResolver::new();
            let pkgs = match resolver.resolve_comps_target(entry, false) {
                Ok(p) => p,
                Err(e) => {
                    return Err(eyre!(
                        "Line {} in packages file {}: Failed to resolve comps target '{}': {}",
                        line_no + 1,
                        file_path.display(),
                        entry,
                        e
                    ));
                }
            };

            let mut resolved_any = false;
            for pkg in pkgs {
                if let Ok(resolved) = resolve_package_target(&pkg, distgit_dest)
                    && seen.insert(resolved.clone()) {
                        targets.push(resolved);
                        resolved_any = true;
                    }
            }

            if !resolved_any {
                eprintln!(
                    "Notice: Line {} in {}: Comps target '{}' resolved, but no member packages were found locally in {}",
                    line_no + 1,
                    file_path.display(),
                    entry,
                    distgit_dest.display()
                );
            }
            continue;
        }

        match resolve_package_target(entry, distgit_dest) {
            Ok(resolved) => {
                if seen.insert(resolved.clone()) {
                    targets.push(resolved);
                }
            }
            Err(e) => {
                return Err(eyre!(
                    "Line {} in packages file {}: {}",
                    line_no + 1,
                    file_path.display(),
                    e
                ));
            }
        }
    }

    if targets.is_empty() {
        return Err(eyre!(
            "No valid package targets found in {}",
            file_path.display()
        ));
    }

    Ok(targets)
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

    #[test]
    fn test_extract_error_summary_check_failure() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("build.log");
        let mock_log = "
Building strace-7.2...
Running tests...
PASS: test_fork
FAIL: test_vmsplice
PASS: test_socket
error: Bad exit status from /var/tmp/rpm-tmp.abc123 (%check)
";
        fs::write(&log_path, mock_log).unwrap();
        let summary = extract_error_summary(&log_path, "", "ERROR: Command failed: rpmbuild");
        assert_eq!(summary, "FAIL: test_vmsplice [(%check)]");
    }

    #[test]
    fn test_extract_error_summary_compile_failure() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("build.log");
        let mock_log = "
gcc -O2 -c parser.c
parser.c:42:10: fatal error: missing_header.h: No such file or directory
make[1]: *** [Makefile:88: parser.o] Error 1
error: Bad exit status from /var/tmp/rpm-tmp.xyz789 (%build)
";
        fs::write(&log_path, mock_log).unwrap();
        let summary = extract_error_summary(&log_path, "", "ERROR: Exception");
        assert_eq!(
            summary,
            "parser.c:42:10: fatal error: missing_header.h: No such file or directory [(%build)]"
        );
    }

    #[test]
    fn test_extract_error_summary_wrapper_fallback() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("non_existent_build.log");
        let wrapper_stderr = "
INFO: Mock Version: 6.8
ERROR: Command failed: # /usr/bin/systemd-nspawn -D /root dnf install
";
        let summary = extract_error_summary(&log_path, "", wrapper_stderr);
        assert_eq!(
            summary,
            "ERROR: Command failed: # /usr/bin/systemd-nspawn -D /root dnf install"
        );
    }

    #[test]
    fn test_find_spec_in_dir() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("strace");
        fs::create_dir(&sub).unwrap();
        assert!(find_spec_in_dir(&sub).is_none());

        let spec_file = sub.join("strace.spec");
        File::create(&spec_file).unwrap();
        assert_eq!(find_spec_in_dir(&sub), Some(spec_file));
    }

    #[test]
    fn test_resolve_package_target() {
        let dir = tempdir().unwrap();
        let distgit = dir.path().join("distgit");
        let pkg_dir = distgit.join("bash");
        fs::create_dir_all(&pkg_dir).unwrap();
        let bash_spec = pkg_dir.join("bash.spec");
        File::create(&bash_spec).unwrap();

        // 1. Direct path
        assert_eq!(resolve_package_target(bash_spec.to_str().unwrap(), &distgit).unwrap(), bash_spec);

        // 2. Direct directory
        assert_eq!(resolve_package_target(pkg_dir.to_str().unwrap(), &distgit).unwrap(), bash_spec);

        // 3. Package name in distgit dest
        assert_eq!(resolve_package_target("bash", &distgit).unwrap(), bash_spec);

        // 4. Missing package
        assert!(resolve_package_target("nonexistent", &distgit).is_err());
    }

    #[test]
    fn test_parse_packages_list_valid() {
        let dir = tempdir().unwrap();
        let distgit = dir.path().join("distgit");

        let p1 = distgit.join("pkg1");
        fs::create_dir_all(&p1).unwrap();
        let spec1 = p1.join("pkg1.spec");
        File::create(&spec1).unwrap();

        let p2 = distgit.join("pkg2");
        fs::create_dir_all(&p2).unwrap();
        let spec2 = p2.join("pkg2.spec");
        File::create(&spec2).unwrap();

        let manifest = "
# Core build manifest
pkg1
  # Inline comment
pkg2   # trailing comment
pkg1   # duplicate entry should be deduplicated
";
        let fake_file = Path::new("packages.txt");
        let targets = parse_packages_list(manifest, &distgit, fake_file).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0], spec1);
        assert_eq!(targets[1], spec2);
    }

    #[test]
    fn test_parse_packages_list_missing() {
        let dir = tempdir().unwrap();
        let distgit = dir.path().join("distgit");
        let manifest = "
# Manifest with missing package
valid_pkg
missing_pkg
";
        let valid_dir = distgit.join("valid_pkg");
        fs::create_dir_all(&valid_dir).unwrap();
        File::create(valid_dir.join("valid_pkg.spec")).unwrap();

        let fake_file = Path::new("packages.txt");
        let res = parse_packages_list(manifest, &distgit, fake_file);
        assert!(res.is_err());
        let err_str = res.unwrap_err().to_string();
        assert!(err_str.contains("Line 4"));
        assert!(err_str.contains("missing_pkg"));
    }

    #[test]
    fn test_parse_packages_list_empty() {
        let dir = tempdir().unwrap();
        let distgit = dir.path().join("distgit");
        let manifest = "# Only comments\n\n   # Empty lines\n";
        let fake_file = Path::new("packages.txt");
        let res = parse_packages_list(manifest, &distgit, fake_file);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("No valid package targets found"));
    }

    #[test]
    fn test_parse_packages_list_xml_and_comps() {
        let dir = tempdir().unwrap();
        let distgit = dir.path().join("distgit");

        let p1 = distgit.join("bash");
        fs::create_dir_all(&p1).unwrap();
        let spec1 = p1.join("bash.spec");
        File::create(&spec1).unwrap();

        let manifest = r#"
<!-- Kiwi XML elements or plain targets -->
<package name="bash"/>
<package name="@core"/>
"#;
        let fake_file = Path::new("config.xml");
        let targets = parse_packages_list(manifest, &distgit, fake_file).unwrap();
        assert!(!targets.is_empty());
        assert_eq!(targets[0], spec1);
    }

    #[test]
    fn test_parse_source_entry() {
        let (url, fname) = parse_source_entry("https://www.ivarch.com/programs/sources/pv-1.11.0.tar.gz.txt");
        assert_eq!(url, "https://www.ivarch.com/programs/sources/pv-1.11.0.tar.gz.txt");
        assert_eq!(fname, "pv-1.11.0.tar.gz.txt");

        let (url, fname) = parse_source_entry("https://github.com/facebook/zstd/archive/v1.5.6.tar.gz#/zstd-1.5.6.tar.gz");
        assert_eq!(url, "https://github.com/facebook/zstd/archive/v1.5.6.tar.gz");
        assert_eq!(fname, "zstd-1.5.6.tar.gz");

        let (url, fname) = parse_source_entry("https://example.com/download.php?id=123#/my-pkg-1.0.tar.gz");
        assert_eq!(url, "https://example.com/download.php?id=123");
        assert_eq!(fname, "my-pkg-1.0.tar.gz");

        let (url, fname) = parse_source_entry("https://example.com/download/archive.tar.gz?version=1.0");
        assert_eq!(url, "https://example.com/download/archive.tar.gz?version=1.0");
        assert_eq!(fname, "archive.tar.gz");

        let (url, fname) = parse_source_entry("local-patch.patch");
        assert_eq!(url, "local-patch.patch");
        assert_eq!(fname, "local-patch.patch");
    }

    #[test]
    fn test_extract_error_diagnostic_build_failure() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("build.log");
        let content = "\
make[1]: Entering directory '/builddir/build/BUILD/mypkg'
gcc -O2 -g -c mypkg.c -o mypkg.o
mypkg.c:42:10: fatal error: header.h: No such file or directory
   42 | #include <header.h>
      |          ^~~~~~~~~~
compilation terminated.
make[1]: *** [Makefile:120: mypkg.o] Error 1
make: *** [Makefile:80: all] Error 2
error: Bad exit status from /var/tmp/rpm-tmp.XYZ (%build)

RPM build errors:
    Bad exit status from /var/tmp/rpm-tmp.XYZ (%build)
";
        fs::write(&log_path, content).unwrap();

        let (summary, diag) = extract_error_diagnostic(dir.path(), &log_path, "", "");
        assert!(summary.contains("fatal error: header.h"));
        assert!(summary.contains("(%build)"));
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.phase.as_deref(), Some("%build"));
        assert!(d.context_lines.iter().any(|l| l.contains("mypkg.c:42:10: fatal error")));
    }

    #[test]
    fn test_extract_error_diagnostic_root_log_failure() {
        let dir = tempdir().unwrap();
        let log_path = dir.path().join("build.log");
        let root_log = dir.path().join("root.log");
        let root_content = "\
Starting Mock buildroot initialization...
Installed: bash-5.2-1.fc41.x86_64
No match for argument: libsecret-devel >= 0.20
Error: Problem: package cannot be installed
  - nothing provides libsecret-devel >= 0.20 needed by myapp.spec
";
        fs::write(&root_log, root_content).unwrap();

        let (_summary, diag) = extract_error_diagnostic(dir.path(), &log_path, "", "Command failed: dnf5 builddep");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.phase.as_deref(), Some("builddep / dnf"));
        assert!(d.context_lines.iter().any(|l| l.contains("nothing provides libsecret-devel")));
    }

    #[test]
    fn test_worker_lease_lifecycle() {
        let pool = Arc::new(Mutex::new(VecDeque::from([1, 2, 3])));
        {
            let id1 = pool.lock().unwrap().pop_front().unwrap();
            let id2 = pool.lock().unwrap().pop_front().unwrap();
            let lease1 = WorkerLease::new(pool.clone(), id1);
            let lease2 = WorkerLease::new(pool.clone(), id2);
            assert_eq!(lease1.id(), 1);
            assert_eq!(lease2.id(), 2);
            assert_eq!(pool.lock().unwrap().len(), 1);
        }
        let mut guard = pool.lock().unwrap();
        assert_eq!(guard.len(), 3);
        assert_eq!(guard.pop_front(), Some(3));
        assert_eq!(guard.pop_front(), Some(2));
        assert_eq!(guard.pop_front(), Some(1));
    }

    #[test]
    fn test_worker_lease_concurrency_no_collisions() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let concurrency = 4;
        let pool = Arc::new(Mutex::new((1..=concurrency).collect::<VecDeque<usize>>()));
        let active = Arc::new(Mutex::new(HashSet::new()));
        let collision_detected = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::new();
        for _ in 0..20 {
            let pool_clone = pool.clone();
            let active_clone = active.clone();
            let collision_clone = collision_detected.clone();

            handles.push(std::thread::spawn(move || {
                for _ in 0..10 {
                    let worker_id = {
                        let mut guard = pool_clone.lock().unwrap();
                        guard.pop_front()
                    };
                    if let Some(id) = worker_id {
                        let _lease = WorkerLease::new(pool_clone.clone(), id);
                        {
                            let mut act = active_clone.lock().unwrap();
                            if !act.insert(id) {
                                collision_clone.store(true, Ordering::SeqCst);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(1));
                        {
                            let mut act = active_clone.lock().unwrap();
                            act.remove(&id);
                        }
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert!(!collision_detected.load(Ordering::SeqCst), "Collision detected in leased worker IDs");
        assert_eq!(pool.lock().unwrap().len(), concurrency);
    }

    #[test]
    fn test_execute_mock_retry_non_lock_failure() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo 'regular compilation error' >&2; exit 1");
        let start = Instant::now();
        let res = execute_mock_with_lock_retry_impl(
            &mut cmd,
            "testpkg",
            1,
            5,
            Duration::from_millis(50),
        );
        assert!(res.is_ok());
        let output = res.unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("regular compilation error"));
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn test_execute_mock_retry_lock_recovery() {
        let dir = tempdir().unwrap();
        let counter_file = dir.path().join("attempt.txt");
        let script = format!(
            "if [ ! -f '{}' ]; then touch '{}'; echo 'ERROR: Build root is locked by another process' >&2; exit 1; else echo 'Build succeeded'; exit 0; fi",
            counter_file.display(),
            counter_file.display()
        );
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(&script);

        let res = execute_mock_with_lock_retry_impl(
            &mut cmd,
            "testpkg",
            1,
            3,
            Duration::from_millis(20),
        );
        assert!(res.is_ok());
        let output = res.unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Build succeeded"));
    }

    #[test]
    fn test_execute_mock_retry_lock_exhausted() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("echo 'ERROR: Build root is locked by another process' >&2; exit 1");

        let res = execute_mock_with_lock_retry_impl(
            &mut cmd,
            "testpkg",
            1,
            2,
            Duration::from_millis(10),
        );
        assert!(res.is_ok());
        let output = res.unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("locked by another process"));
    }

    #[test]
    fn test_mock_runner_nocheck_builder() {
        let runner = MockRunner::new("test-profile").with_nocheck(true);
        assert!(runner.nocheck);

        let runner_disabled = MockRunner::new("test-profile").with_nocheck(false);
        assert!(!runner_disabled.nocheck);
    }

    #[test]
    fn test_rpmbuild_runner_nocheck_builder() {
        let runner = RpmbuildRunner::default().with_nocheck(true);
        assert!(runner.nocheck);

        let runner_disabled = RpmbuildRunner::default().with_nocheck(false);
        assert!(!runner_disabled.nocheck);
    }

    #[test]
    fn test_extract_nocheck_modifier() {
        assert_eq!(extract_nocheck_modifier("git:nocheck"), ("git", true));
        assert_eq!(extract_nocheck_modifier("cockpit:nocheck"), ("cockpit", true));
        assert_eq!(extract_nocheck_modifier("/path/to/git.spec:nocheck"), ("/path/to/git.spec", true));
        assert_eq!(extract_nocheck_modifier("curl"), ("curl", false));
        assert_eq!(extract_nocheck_modifier(""), ("", false));
    }

    #[test]
    fn test_runner_nocheck_packages_exemption() {
        let runner = MockRunner::new("test-profile")
            .with_nocheck_packages(vec!["cockpit", "git:nocheck"]);

        assert!(runner.is_nocheck_package("cockpit"));
        assert!(runner.is_nocheck_package("git"));
        assert!(runner.is_nocheck_package("GIT"));
        assert!(!runner.is_nocheck_package("curl"));
        assert!(!runner.is_nocheck_package("gcc"));

        let runner_added = runner.add_nocheck_package("gnome-shell");
        assert!(runner_added.is_nocheck_package("gnome-shell"));
    }
}


