//! Distributed build worker client implementation for DBS.
//!
//! Connects to a central PostgreSQL queue, claims pending package compilation
//! jobs using atomic `FOR UPDATE SKIP LOCKED` transactions, hermetically builds
//! them using Mock with remote repository feedback (`--addrepo`), streams periodic
//! heartbeats, and uploads compiled RPM artifacts and build logs to the coordinator.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eyre::Result;
use log::error;
use tokio::sync::Semaphore;

use crate::db;
use crate::models::Package;
use crate::runner::{BuildOutput, MockRunner};

/// Configuration options for a DBS cluster worker node.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// PostgreSQL connection URL (e.g. `postgres://dbs:pass@sohoai:5432/dbs`).
    pub db_url: Option<String>,

    /// Coordinator HTTP base URL (e.g. `http://sohoai:8080`).
    pub coordinator_url: String,

    /// Unique numeric worker ID for telemetry attribution (defaults to 100 + PID % 900).
    pub worker_id: i32,

    /// Human-readable node name (defaults to system hostname).
    pub node_name: String,

    /// Target Mock chroot profile name (e.g. `tacos-stable-x86_64`).
    pub root_name: String,

    /// Directory containing Mock `.cfg` profile files.
    pub config_dir: Option<PathBuf>,

    /// Number of concurrent compilation worker slots on this node.
    pub concurrency: usize,

    /// Number of SMP compilation threads passed to Mock inside the chroot.
    pub smp_cpus: Option<usize>,

    /// Local scratch / working directory for downloaded SRPMs and build artifacts.
    pub work_dir: PathBuf,

    /// Heartbeat interval in seconds (default: 10).
    pub heartbeat_interval_secs: u64,

    /// Poll interval in seconds when no pending jobs are available in queue (default: 3).
    pub poll_interval_secs: u64,

    /// Keep temporary worker build files instead of deleting after upload.
    pub keep_build_dir: bool,

    /// Disable running test suites in Mock (%check phase).
    pub nocheck: bool,

    /// Specific package names configured to skip %check test execution.
    pub nocheck_packages: Vec<String>,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        let hostname = std::env::var("HOSTNAME")
            .unwrap_or_else(|_| "dbs-worker-node".to_string());
        let pid = std::process::id();
        let default_worker_id = 100 + (pid % 800) as i32;

        Self {
            db_url: None,
            coordinator_url: "http://127.0.0.1:8080".to_string(),
            worker_id: default_worker_id,
            node_name: hostname,
            root_name: "tacos-stable-x86_64".to_string(),
            config_dir: None,
            concurrency: 4,
            smp_cpus: None,
            work_dir: std::env::temp_dir().join("dbs-worker"),
            heartbeat_interval_secs: 10,
            poll_interval_secs: 3,
            keep_build_dir: false,
            nocheck: false,
            nocheck_packages: Vec::new(),
        }
    }
}

/// Runs the distributed worker service loop.
///
/// Registers the worker in the PostgreSQL database, spawns periodic heartbeat
/// telemetry in the background, and continuously claims and compiles packages
/// until terminated via SIGINT/Ctrl+C.
pub async fn run_worker(config: WorkerConfig) -> Result<()> {
    println!("===========================================================");
    println!(" DBS Distributed Build Worker Node");
    println!(" Worker ID:        W{}", config.worker_id);
    println!(" Node Hostname:    {}", config.node_name);
    println!(" Coordinator URL:  {}", config.coordinator_url);
    println!(" Target Chroot:    {}", config.root_name);
    println!(" Concurrency:      {} build slot(s)", config.concurrency);
    if let Some(smp) = config.smp_cpus {
        println!(" SMP CPUs:         {} cores per build", smp);
    }
    println!(" Scratch Dir:      {}", config.work_dir.display());
    println!("===========================================================");

    // 1. Establish initial DB connection to verify connectivity and register
    let mut conn = db::establish_connection_with_url(config.db_url.as_deref())?;
    db::register_cluster_worker(
        &mut conn,
        config.worker_id,
        &config.node_name,
        config.concurrency as i32,
    )?;
    println!("✓ Successfully registered with cluster queue in PostgreSQL");

    // 2. Setup termination signal handler
    let running = Arc::new(AtomicBool::new(true));
    let r_sig = running.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        println!("\nShutdown signal received. Finishing in-flight builds...");
        r_sig.store(false, Ordering::SeqCst);
    });

    // 3. Spawn background heartbeat loop
    let hb_db_url = config.db_url.clone();
    let hb_worker_id = config.worker_id;
    let hb_interval = Duration::from_secs(config.heartbeat_interval_secs);
    let hb_running = running.clone();

    tokio::spawn(async move {
        while hb_running.load(Ordering::SeqCst) {
            tokio::time::sleep(hb_interval).await;
            if let Ok(mut hb_conn) = db::establish_connection_with_url(hb_db_url.as_deref()) {
                let _ = db::update_worker_heartbeat(&mut hb_conn, hb_worker_id, None);
            }
        }
    });

    // 4. Initialize HTTP client for downloading SRPMs and uploading artifacts
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    // 5. Worker execution slots via Tokio Semaphore
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let cfg = Arc::new(config);

    while running.load(Ordering::SeqCst) {
        // Claim permit for available worker slot
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };

        let worker_cfg = cfg.clone();
        let client = http_client.clone();

        tokio::spawn(async move {
            let _permit = permit;

            // Attempt to claim next pending package from PostgreSQL queue
            let claimed_pkg = {
                let mut conn = match db::establish_connection_with_url(worker_cfg.db_url.as_deref()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Worker W{}: Database connection error: {}", worker_cfg.worker_id, e);
                        tokio::time::sleep(Duration::from_secs(worker_cfg.poll_interval_secs)).await;
                        return;
                    }
                };

                match db::claim_next_pending_package(&mut conn, worker_cfg.worker_id, None) {
                    Ok(Some(pkg)) => pkg,
                    Ok(None) => {
                        // Queue empty, wait for jobs
                        tokio::time::sleep(Duration::from_secs(worker_cfg.poll_interval_secs)).await;
                        return;
                    }
                    Err(e) => {
                        error!("Worker W{}: Error claiming job: {}", worker_cfg.worker_id, e);
                        tokio::time::sleep(Duration::from_secs(worker_cfg.poll_interval_secs)).await;
                        return;
                    }
                }
            };

            println!(
                "▶ [W{}] Claimed compilation job: {} ({}-{})",
                worker_cfg.worker_id, claimed_pkg.name, claimed_pkg.version, claimed_pkg.release
            );

            // Execute the build for this package
            let res = execute_worker_package_build(&worker_cfg, &claimed_pkg, &client).await;
            match res {
                Ok(output) => {
                    if output.success {
                        println!(
                            "✓ [W{}] Successfully compiled {} in {:.1}s ({} artifact(s))",
                            worker_cfg.worker_id,
                            claimed_pkg.name,
                            output.duration_seconds,
                            output.artifacts.len()
                        );
                    } else {
                        println!(
                            "✗ [W{}] Build failed for {} after {:.1}s: {}",
                            worker_cfg.worker_id,
                            claimed_pkg.name,
                            output.duration_seconds,
                            output.error_summary.as_deref().unwrap_or("Unknown error")
                        );
                    }
                }
                Err(e) => {
                    error!("Worker W{}: Execution error on {}: {}", worker_cfg.worker_id, claimed_pkg.name, e);
                }
            }
        });
    }

    // Graceful unregister
    if let Ok(mut conn) = db::establish_connection_with_url(cfg.db_url.as_deref()) {
        let _ = db::unregister_cluster_worker(&mut conn, cfg.worker_id);
    }
    println!("✓ Worker W{} shut down cleanly.", cfg.worker_id);

    Ok(())
}

/// Executes the hermetic build, artifact upload, and database recording for a claimed package.
async fn execute_worker_package_build(
    cfg: &WorkerConfig,
    pkg: &Package,
    client: &reqwest::Client,
) -> Result<BuildOutput> {
    let start_time = Instant::now();
    let pkg_work_dir = cfg.work_dir.join(&pkg.name);
    let pkg_results_dir = pkg_work_dir.join("results");
    fs::create_dir_all(&pkg_results_dir)?;

    let srpm_filename = pkg.sourcerpm.clone().unwrap_or_else(|| {
        format!("{}-{}-{}.src.rpm", pkg.name, pkg.version, pkg.release)
    });
    let local_srpm_path = pkg_work_dir.join(&srpm_filename);

    // 1. Download the SRPM from coordinator staging
    let srpm_url = format!("{}/staging/srpms/{}", cfg.coordinator_url.trim_end_matches('/'), srpm_filename);
    println!("  • [W{}] Fetching SRPM from {}...", cfg.worker_id, srpm_url);

    let resp = client.get(&srpm_url).send().await;
    match resp {
        Ok(res) if res.status().is_success() => {
            let bytes = res.bytes().await?;
            fs::write(&local_srpm_path, bytes)?;
        }
        _ => {
            // If SRPM download fails, check if we have a spec file on disk or fail
            let err_msg = format!("Failed to download SRPM from coordinator at {}", srpm_url);
            let log_path = pkg_results_dir.join("worker.log");
            let _ = fs::write(&log_path, &err_msg);

            let out = BuildOutput {
                target_name: pkg.name.clone(),
                success: false,
                duration_seconds: start_time.elapsed().as_secs_f64(),
                log_path,
                artifacts: Vec::new(),
                error_summary: Some(err_msg),
                error_diagnostic: None,
            };

            if let Ok(mut conn) = db::establish_connection_with_url(cfg.db_url.as_deref()) {
                let _ = db::record_build_result(&mut conn, &pkg.name, &out);
            }
            return Ok(out);
        }
    }

    // 2. Setup MockRunner with dynamic remote repository feedback
    let mut runner = MockRunner::resolve(&cfg.root_name, cfg.config_dir.clone())?;
    if let Some(smp) = cfg.smp_cpus {
        runner = runner.with_smp_cpus(smp);
    }
    runner = runner.with_nocheck(cfg.nocheck);
    runner = runner.with_nocheck_packages(cfg.nocheck_packages.clone());

    // Inject the coordinator's staging repository over HTTP so Layer K+1 packages
    // find dependencies compiled in Layer K
    let staging_remote_repo = format!("{}/staging/rpms/x86_64", cfg.coordinator_url.trim_end_matches('/'));
    runner = runner.with_additional_repo(staging_remote_repo);

    // 3. Execute compilation in Mock hermetic chroot
    let runner_clone = runner.clone();
    let target_srpm = local_srpm_path.clone();
    let res_dir = pkg_results_dir.clone();
    let worker_slot_id = cfg.worker_id as usize;

    let build_res = tokio::task::spawn_blocking(move || {
        runner_clone.build_with_worker(&target_srpm, &res_dir, worker_slot_id)
    })
    .await;

    let out = match build_res {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let log_path = pkg_results_dir.join("worker.log");
            let err_msg = format!("Mock execution error: {}", e);
            let _ = fs::write(&log_path, &err_msg);
            BuildOutput {
                target_name: pkg.name.clone(),
                success: false,
                duration_seconds: start_time.elapsed().as_secs_f64(),
                log_path,
                artifacts: Vec::new(),
                error_summary: Some(err_msg),
                error_diagnostic: None,
            }
        }
        Err(join_err) => {
            let log_path = pkg_results_dir.join("worker.log");
            let err_msg = format!("Worker task panicked: {}", join_err);
            let _ = fs::write(&log_path, &err_msg);
            BuildOutput {
                target_name: pkg.name.clone(),
                success: false,
                duration_seconds: start_time.elapsed().as_secs_f64(),
                log_path,
                artifacts: Vec::new(),
                error_summary: Some(err_msg),
                error_diagnostic: None,
            }
        }
    };

    // 4. Upload artifacts and build log to coordinator
    if out.success {
        for art in &out.artifacts {
            if let Some(filename) = art.file_name().and_then(|f| f.to_str()) {
                let upload_url = format!(
                    "{}/upload/staging/rpms/x86_64/{}",
                    cfg.coordinator_url.trim_end_matches('/'),
                    filename
                );
                if let Ok(file_bytes) = fs::read(art) {
                    let _ = client.put(&upload_url).body(file_bytes).send().await;
                }
            }
        }
    }

    // Always upload build log
    if out.log_path.exists()
        && let Ok(log_bytes) = fs::read(&out.log_path) {
            let upload_log_url = format!(
                "{}/upload/staging/logs/{}.log",
                cfg.coordinator_url.trim_end_matches('/'),
                pkg.name
            );
            let _ = client.put(&upload_log_url).body(log_bytes).send().await;
        }

    // 5. Update database status
    if let Ok(mut conn) = db::establish_connection_with_url(cfg.db_url.as_deref()) {
        let _ = db::record_build_result(&mut conn, &pkg.name, &out);
    }

    // 6. Clean up temporary files unless instructed to keep
    if !cfg.keep_build_dir {
        let _ = fs::remove_dir_all(&pkg_work_dir);
    }

    Ok(out)
}
