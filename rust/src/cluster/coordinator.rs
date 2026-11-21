//! Distributed build coordinator service and layer orchestration for DBS.
//!
//! Prepares hermetic source RPMs (SRPMs) for scheduled DAG compilation layers,
//! exposes them via an embedded HTTP staging server, enqueues compilation jobs
//! into PostgreSQL, and tracks real-time progress across distributed workers.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use diesel::prelude::*;
use eyre::Result;
use log::warn;

use crate::db;
use crate::models::Package;
use crate::runner::{ensure_sources_present, resolve_sources_dir};
use crate::types::BuildStatus;

/// Coordinator runtime configuration settings.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Host address to bind embedded HTTP coordinator server (default: `0.0.0.0`).
    pub host: String,

    /// Port to bind embedded HTTP coordinator server (default: `8080`).
    pub port: u16,

    /// Public/LAN URL advertised to worker nodes (e.g. `http://192.168.1.100:8080`).
    pub public_url: String,

    /// Staging directory containing built RPMs, SRPMs, and logs.
    pub staging_dir: PathBuf,

    /// Target distribution name (e.g. `tacos-stable-x86_64`).
    pub distro_name: String,

    /// Target architecture (e.g. `x86_64`).
    pub arch: String,

    /// Mock chroot profile name.
    pub mock_root: String,

    /// Optional directory containing Mock `.cfg` profile files.
    pub mock_config_dir: Option<PathBuf>,

    /// Lookaside cache directory for source archive resolution.
    pub lookaside_dir: Option<PathBuf>,

    /// Number of local worker slots running on the coordinator host itself (0 to disable local builds).
    pub local_workers: usize,

    /// Database connection URL.
    pub db_url: Option<String>,
}

/// Spawns the embedded HTTP coordinator service in a background Tokio task.
pub fn spawn_coordinator_http(
    staging_dir: PathBuf,
    host: String,
    port: u16,
) -> tokio::task::JoinHandle<Result<()>> {
    tokio::spawn(async move {
        crate::distro::run_http_server(staging_dir, &host, port).await
    })
}

/// Prepares and generates hermetic `.src.rpm` files for a set of layer packages.
///
/// Places generated SRPMs in `<staging_dir>/srpms/<name>-<version>-<release>.src.rpm`
/// and enqueues the package records as `PENDING` in PostgreSQL.
pub fn prepare_layer_srpms(
    spec_paths: &[PathBuf],
    config: &CoordinatorConfig,
    conn: &mut diesel::pg::PgConnection,
) -> Result<Vec<String>> {
    let srpms_dir = config.staging_dir.join("srpms");
    fs::create_dir_all(&srpms_dir)?;

    let mut queued_package_names = Vec::new();

    for spec_path in spec_paths {
        let _pkg_stem = spec_path.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");

        // Parse spec metadata
        let meta = match crate::distgit::spec::parse_spec_file(spec_path) {
            Ok(m) => m,
            Err(e) => {
                warn!("Coordinator: Failed to parse spec {}: {}", spec_path.display(), e);
                continue;
            }
        };

        let srpm_name = format!("{}-{}-{}.src.rpm", meta.name, meta.version, meta.release);
        let dest_srpm = srpms_dir.join(&srpm_name);

        // If SRPM already exists in staging, reuse it; otherwise generate it
        if !dest_srpm.exists() {
            if spec_path.to_string_lossy().ends_with(".src.rpm") {
                let _ = fs::copy(spec_path, &dest_srpm);
            } else {
                let sources_dir = resolve_sources_dir(spec_path);
                ensure_sources_present(
                    spec_path,
                    &sources_dir,
                    &meta.name,
                    config.lookaside_dir.as_deref(),
                );

                println!("  • [Coordinator] Building SRPM for {} -> {}", meta.name, srpm_name);
                let mut srpm_cmd = Command::new("mock");
                srpm_cmd.arg(format!("-r={}", config.mock_root));
                srpm_cmd.arg(format!("--resultdir={}", srpms_dir.display()));
                if let Some(cfg_dir) = &config.mock_config_dir {
                    srpm_cmd.arg(format!("--configdir={}", cfg_dir.display()));
                }
                srpm_cmd.arg("--buildsrpm");
                srpm_cmd.arg("--spec").arg(spec_path);
                srpm_cmd.arg("--sources").arg(&sources_dir);

                let output = srpm_cmd.output();
                match output {
                    Ok(out) if out.status.success() => {
                        // Success, SRPM generated
                    }
                    Ok(out) => {
                        let err_str = String::from_utf8_lossy(&out.stderr);
                        warn!("Warning: Failed to generate SRPM for {} via Mock: {}", meta.name, err_str);
                        continue;
                    }
                    Err(e) => {
                        warn!("Warning: Failed to run mock --buildsrpm for {}: {}", meta.name, e);
                        continue;
                    }
                }
            }
        }

        // Register or reset package in DB to PENDING
        let _ = db::record_build_start(conn, &meta.name, None, None, Some(&meta));
        let _ = diesel::update(crate::schema::package::table.filter(crate::schema::package::name.eq(&meta.name)))
            .set((
                crate::schema::package::build_status.eq(BuildStatus::PENDING),
                crate::schema::package::sourcerpm.eq(Some(&srpm_name)),
                crate::schema::package::worker_id.eq(None::<i32>),
                crate::schema::package::error_summary.eq(None::<String>),
            ))
            .execute(conn);

        queued_package_names.push(meta.name);
    }

    Ok(queued_package_names)
}

/// Coordinates the execution of a compilation layer across the cluster.
///
/// Monitors PostgreSQL status until all packages in `layer_packages` transition
/// out of `PENDING` and `BUILDING` into `SUCCESS` or `FAILED`.
pub async fn coordinate_layer_build(
    layer_idx: usize,
    layer_packages: &[String],
    config: &CoordinatorConfig,
) -> Result<()> {
    if layer_packages.is_empty() {
        return Ok(());
    }

    let target_set: HashSet<String> = layer_packages.iter().cloned().collect();
    let total_packages = target_set.len();
    let start_time = Instant::now();

    println!(
        "\n▶ [Layer {}] Monitoring cluster compilation across {} package(s)...",
        layer_idx, total_packages
    );

    let poll_interval = Duration::from_secs(3);
    let mut last_reclaim = Instant::now();

    loop {
        tokio::time::sleep(poll_interval).await;

        let mut conn = match db::establish_connection_with_url(config.db_url.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                warn!("Coordinator: DB poll connection error: {}", e);
                continue;
            }
        };

        // Every 30 seconds, reclaim any stale builds whose worker heartbeat expired (> 300s)
        if last_reclaim.elapsed() > Duration::from_secs(30) {
            let _ = db::reclaim_stale_builds(&mut conn, 300);
            last_reclaim = Instant::now();
        }

        // Query status of layer packages
        let pkgs = match crate::schema::package::table
            .filter(crate::schema::package::name.eq_any(layer_packages))
            .load::<Package>(&mut conn)
        {
            Ok(p) => p,
            Err(e) => {
                warn!("Coordinator: Failed to query layer packages: {}", e);
                continue;
            }
        };

        let mut pending = 0;
        let mut building = 0;
        let mut success = 0;
        let mut failed = 0;
        let mut active_workers = Vec::new();

        for p in &pkgs {
            match p.build_status {
                BuildStatus::PENDING => pending += 1,
                BuildStatus::BUILDING => {
                    building += 1;
                    if let Some(w) = p.worker_id {
                        active_workers.push(format!("W{}", w));
                    }
                }
                BuildStatus::SUCCESS => success += 1,
                BuildStatus::FAILED => failed += 1,
                BuildStatus::SKIPPED => success += 1,
            }
        }

        active_workers.sort();
        active_workers.dedup();

        let workers_str = if active_workers.is_empty() {
            String::new()
        } else {
            format!(" [{}]", active_workers.join(", "))
        };

        println!(
            "  ⏳ [Layer {}] Progress: {}/{} done ({} building{}, {} pending, {} failed)",
            layer_idx,
            success + failed,
            total_packages,
            building,
            workers_str,
            pending,
            failed
        );

        if pending == 0 && building == 0 {
            let elapsed = start_time.elapsed().as_secs_f64();
            println!(
                "✓ [Layer {}] Layer completed in {:.1}s: {} success, {} failed",
                layer_idx, elapsed, success, failed
            );
            break;
        }
    }

    Ok(())
}
