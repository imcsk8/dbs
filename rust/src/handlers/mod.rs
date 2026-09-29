//! Command execution handlers for the Distribution Build System (DBS) CLI.
//!
//! Implements dispatch workflows for exploring, cloning, inspecting, building,
//! DAG resolution, Mock chroot management, repository publishing, and database lifecycle.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use eyre::{eyre, Result};

use crate::cli::{
    self, BuildArgs, ChrootArgs, ChrootCommands, ConfigArgs, ConfigCommands, DagArgs, DbArgs,
    DbCommands, DistgitArgs, DistgitCommands, DistroArgs, DistroCommands, ExploreArgs,
    LookasideArgs, LookasideCommands, MonitorArgs, OsArgs, PkgArgs, RetryArgs, ShellArgs,
    CleanArgs,
};
use crate::chroot;
use crate::config::DbsConfig;
use crate::dag::DependencyGraph;
use crate::db;
use crate::distgit::provider::DistroConfig;
use crate::distgit::spec::{parse_spec_file, SpecMetadata};
use crate::distgit::DistGitClient;
use crate::distro::{
    generate_nginx_config, get_distro_status, init_distro, publish_distro, run_http_server,
    DistroInitOptions, DistroPublishOptions,
};
use crate::lookaside::LookasideManager;
use crate::runner::{self, BuildOutput, BuildRunner, MockRunner, RpmbuildRunner};
use crate::tui::{run_explorer, run_monitor};

/// Dispatches the `explore` subcommand to discover remote packages in dist-git.
pub async fn handle_explore(args: ExploreArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let config = match DistroConfig::from_preset(&args.distro) {
        Some(cfg) => cfg,
        None => {
            println!("Unknown distribution preset: '{}'. Available presets:", args.distro);
            for p in DistroConfig::all_presets() {
                println!("  * {} (branch: {}, api: {})", p.name, p.dist_git_branch, p.api_type);
            }
            return Ok(());
        }
    };

    let effective_api_key = args.api_key.or_else(|| dbs_cfg.distgit.api_key.clone());
    let client = DistGitClient::new(config.clone()).with_api_key(effective_api_key);
    let query_limit = if args.all { None } else { Some(args.limit) };

    if args.interactive {
        let initial_limit = query_limit.or(Some(100));
        let initial_projects = client.explore(args.search.as_deref(), initial_limit).await?;
        let target_rpm_dir = dbs_cfg.distgit.dest.clone();
        run_explorer(config, target_rpm_dir, initial_projects, args.search.as_deref()).await?;
        return Ok(());
    }

    println!("===========================================================");
    println!(" DBS Remote Package Explorer");
    println!(" Target:    {} ({})", config.name, config.version);
    println!(" API Type:  {}", config.api_type);
    if let Some(pattern) = &args.search {
        println!(" Query:     '{}'", pattern);
    }
    println!("===========================================================");

    let projects = client.explore(args.search.as_deref(), query_limit).await?;

    if projects.is_empty() {
        println!("No projects or packages found matching query.");
    } else {
        println!("Found {} package project(s):", projects.len());
        for p in &projects {
            println!("\n  * Package:  {}", p.name);
            println!("    Git URL:  {}", p.git_url);
            if let Some(web) = &p.web_url {
                println!("    Web URL:  {}", web);
            }
            if let Some(desc) = &p.description {
                let trimmed = desc.trim();
                if !trimmed.is_empty() {
                    println!("    Summary:  {}", trimmed);
                }
            }
        }
    }
    println!("===========================================================");

    Ok(())
}

/// Dispatches the `monitor` / `top` subcommand for live interactive system telemetry.
pub async fn handle_monitor(args: MonitorArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    run_monitor(dbs_cfg.clone(), args.distro).await?;
    Ok(())
}

/// Dispatches the `distgit` subcommand for cloning, pulling, syncing, and inspecting.
pub async fn handle_distgit(args: DistgitArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        DistgitCommands::Clone {
            distro,
            dest,
            rename_as,
            rename_spec,
            new_origin,
            new_top_origin,
            api_key,
            packages,
        } => {
            let distro = distro.unwrap_or_else(|| dbs_cfg.distgit.distro.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let config = DistroConfig::from_preset(&distro)
                .ok_or_else(|| eyre!("Unknown distribution preset '{}'", distro))?;
            let effective_api_key = api_key.or_else(|| dbs_cfg.distgit.api_key.clone());
            let client = DistGitClient::new(config).with_api_key(effective_api_key);
            let effective_top_origin = new_top_origin.or_else(|| dbs_cfg.distgit.new_top_origin.clone());

            println!("Cloning {} package(s) from {} to {}...", packages.len(), distro, dest.display());
            for item in packages {
                let (source_pkg, target_name) = if let Some((src, tgt)) = item.split_once(':') {
                    (src.trim(), tgt.trim())
                } else if let Some(rename) = &rename_as {
                    (item.as_str(), rename.as_str())
                } else {
                    (item.as_str(), item.as_str())
                };

                if target_name != source_pkg {
                    println!("Cloning upstream '{}' as '{}'...", source_pkg, target_name);
                }

                let resolved_origin = if let Some(orig) = &new_origin {
                    Some(orig.clone())
                } else if let Some(top) = &effective_top_origin {
                    if top.contains("{package}") {
                        Some(top.replace("{package}", target_name))
                    } else {
                        Some(format!("{}/{}", top.trim_end_matches('/'), target_name))
                    }
                } else {
                    None
                };

                match client.clone_or_pull_as(source_pkg, target_name, &dest, rename_spec, resolved_origin.as_deref()) {
                    Ok(status) => {
                        println!("✓ Cloned {} -> {} (branch: {}, commit: {:.8})", source_pkg, status.package_name, status.branch, status.commit_hash);
                        println!("  Spec: {} ({}-{})", status.spec_path.display(), status.spec_meta.version, status.spec_meta.release);
                        if let Some(orig) = &resolved_origin {
                            println!("  Git Remote 'origin':   {}", orig);
                            println!("  Git Remote 'upstream': {}", client.config().git_url_for_package(source_pkg));
                        }
                    }
                    Err(e) => eprintln!("✗ Failed to clone {}: {}", source_pkg, e),
                }
            }
        }

        DistgitCommands::Pull { dest, packages } => {
            let dest = dest.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let pkgs_to_pull = if packages.is_empty() {
                let mut found = Vec::new();
                if let Ok(entries) = fs::read_dir(&dest) {
                    for entry in entries.flatten() {
                        if entry.path().join(".git").exists()
                            && let Some(name) = entry.file_name().to_str() {
                                found.push(name.to_string());
                            }
                    }
                }
                found
            } else {
                packages
            };

            if pkgs_to_pull.is_empty() {
                println!("No dist-git repositories found in {}", dest.display());
                return Ok(());
            }

            println!("Pulling updates for {} repository(ies)...", pkgs_to_pull.len());
            // Default to Fedora Rawhide config for pulling existing repos
            let config = DistroConfig::fedora_rawhide();
            let client = DistGitClient::new(config);

            for pkg in pkgs_to_pull {
                match client.clone_or_pull(&pkg, &dest) {
                    Ok(status) => {
                        println!("✓ Updated {} (branch: {}, commit: {:.8})", pkg, status.branch, status.commit_hash);
                    }
                    Err(e) => eprintln!("✗ Failed to update {}: {}", pkg, e),
                }
            }
        }

        DistgitCommands::Sync {
            distro,
            dest,
            concurrency,
            sources,
            search,
            limit,
            all,
            record_db,
            lookaside_dir,
            new_top_origin,
            api_key,
        } => {
            let distro = distro.unwrap_or_else(|| dbs_cfg.distgit.distro.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let concurrency = concurrency.unwrap_or(dbs_cfg.distgit.concurrency);
            let lookaside_dir = lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
            let record_db = record_db || dbs_cfg.database.record_db;
            let effective_top_origin = new_top_origin.or_else(|| dbs_cfg.distgit.new_top_origin.clone());
            let effective_api_key = api_key.or_else(|| dbs_cfg.distgit.api_key.clone());

            let config = DistroConfig::from_preset(&distro)
                .ok_or_else(|| eyre!("Unknown distribution preset '{}'", distro))?;
            let client = Arc::new(DistGitClient::new(config).with_api_key(effective_api_key));
            let lookaside_mgr = LookasideManager::resolve_default(lookaside_dir.as_deref());

            let query_limit = if all { None } else { Some(limit) };
            if all {
                println!("Discovering all packages in {} across all pages...", distro);
            } else {
                println!("Discovering packages in {} matching query '{:?}' (limit: {})...", distro, search, limit);
            }
            let projects = client.explore(search.as_deref(), query_limit).await?;
            if projects.is_empty() {
                println!("No packages found to synchronize.");
                return Ok(());
            }

            let mut db_conn = if record_db {
                match db::establish_connection_with_url(dbs_cfg.database.url.as_deref()) {
                    Ok(conn) => {
                        println!("✓ Connected to database for package catalog synchronization.");
                        Some(conn)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to connect to database: {}. Continuing without DB recording.", e);
                        None
                    }
                }
            } else {
                None
            };

            let pkg_names: Vec<String> = projects.into_iter().map(|p| p.name).collect();
            let total = pkg_names.len();
            println!("Synchronizing {} repository(ies) with {} workers into {}...", total, concurrency, dest.display());
            if let Some(top) = &effective_top_origin {
                println!("Automatically configuring new origin remotes based on: {}/<package>", top.trim_end_matches('/'));
            }

            let mut rx = client.clone().sync_batch_stream(pkg_names, dest.clone(), concurrency, effective_top_origin.clone()).await;
            let mut success_count = 0;

            while let Some((idx, total_pkgs, res)) = rx.recv().await {
                match res {
                    Ok(status) => {
                        success_count += 1;
                        println!("[{}/{}] ✓ {} -> {} (commit: {:.8})", idx, total_pkgs, status.package_name, status.spec_meta.name, status.commit_hash);
                        if let Some(top) = &effective_top_origin {
                            let origin_url = if top.contains("{package}") {
                                top.replace("{package}", &status.package_name)
                            } else {
                                format!("{}/{}", top.trim_end_matches('/'), status.package_name)
                            };
                            println!("    * Configured origin remote: {}", origin_url);
                        }

                        if let Some(conn) = &mut db_conn {
                            match db::record_synced_package(conn, &status.spec_meta, &distro, &status.branch, &status.commit_hash) {
                                Ok(p) => println!("    * Persisted to DB package catalog [ID: {}]", p.id),
                                Err(e) => eprintln!("    ✗ DB persistence error for {}: {}", status.package_name, e),
                            }
                        }

                        if sources {
                            println!("  Sourcing archives into lookaside for {}...", status.package_name);
                            match lookaside_mgr.sync_dir(&status.local_path, 2).await {
                                Ok(report) => {
                                    println!("    * Lookaside: {} cached, {} downloaded, {} failed", report.already_cached, report.downloaded, report.failed);
                                }
                                Err(e) => eprintln!("    ✗ Lookaside sync failed: {}", e),
                            }
                        }
                    }
                    Err(e) => eprintln!("[{}/{}] ✗ Sync error: {}", idx, total_pkgs, e),
                }
            }

            println!("Completed: {}/{} repositories synchronized successfully.", success_count, total);
        }

        DistgitCommands::Inspect { path } => {
            let spec_path = if path.is_dir() {
                let mut found = None;
                if let Ok(entries) = fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if entry.path().extension().and_then(|e| e.to_str()) == Some("spec") {
                            found = Some(entry.path());
                            break;
                        }
                    }
                }
                found.ok_or_else(|| eyre!("No .spec file found in directory {}", path.display()))?
            } else {
                path
            };

            let meta = parse_spec_file(&spec_path)?;
            print_spec_summary(&spec_path, &meta);
        }
    }

    Ok(())
}

/// Dispatches the `build` subcommand.
pub async fn handle_build(mut args: BuildArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    // Intercept 'dbs build clean [package]' or 'dbs build delete [package]'
    if let Some(first) = args.targets.first() {
        let first_str = first.to_string_lossy();
        if first_str == "clean" || first_str == "delete" {
            let pkg = args.targets.get(1).map(|p| p.to_string_lossy().to_string());
            let record_db = args.record_db || dbs_cfg.database.record_db;
            let clean_args = CleanArgs {
                package: pkg,
                all: args.targets.len() <= 1,
                staging_only: false,
                repo_only: false,
                clean_chroot: false,
                mock_root: args.mock_root,
                mock_config_dir: args.mock_config_dir,
                staging_dir: args.output_dir,
                repo_dir: None,
                arch: None,
                no_repo_update: false,
                no_db: !record_db,
            };
            return handle_clean(clean_args, dbs_cfg).await;
        }
    }

    let output_dir = args.output_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let concurrency = args.concurrency.unwrap_or(dbs_cfg.distgit.concurrency);
    let mock_root = args.mock_root.or_else(|| Some(dbs_cfg.chroot.profile.clone()));
    let lookaside_dir = args.lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
    let record_db = args.record_db || dbs_cfg.database.record_db;
    let distgit_dest = &dbs_cfg.distgit.dest;

    // If --packages was provided, load targets from the file
    if let Some(pkg_file) = &args.packages {
        println!("Loading package build targets from file: {}", pkg_file.display());
        let file_targets = runner::load_packages_from_file(pkg_file, distgit_dest)?;
        println!("✓ Loaded {} package(s) from {}", file_targets.len(), pkg_file.display());

        let mut combined = file_targets;
        for t in args.targets {
            let resolved = runner::resolve_package_target(&t.to_string_lossy(), distgit_dest)
                .unwrap_or(t);
            if !combined.contains(&resolved) {
                combined.push(resolved);
            }
        }
        args.targets = combined;
    } else {
        let mut resolved_targets = Vec::with_capacity(args.targets.len());
        for t in args.targets {
            let target_str = t.to_string_lossy();
            let resolved = match runner::resolve_package_target(&target_str, distgit_dest) {
                Ok(path) => path,
                Err(_) => {
                    if !target_str.ends_with(".spec") && !target_str.ends_with(".src.rpm") && !target_str.ends_with(".rpm") {
                        if let Some(config) = DistroConfig::from_preset(&dbs_cfg.distgit.distro) {
                            println!("Target '{}' not found locally in {}. Auto-cloning from {}...", target_str, distgit_dest.display(), dbs_cfg.distgit.distro);
                            let client = DistGitClient::new(config).with_api_key(dbs_cfg.distgit.api_key.clone());
                            let resolved_origin = dbs_cfg.distgit.new_top_origin.as_ref().map(|top| {
                                if top.contains("{package}") {
                                    top.replace("{package}", &target_str)
                                } else {
                                    format!("{}/{}", top.trim_end_matches('/'), target_str)
                                }
                            });
                            match client.clone_or_pull_as(&target_str, &target_str, distgit_dest, false, resolved_origin.as_deref()) {
                                Ok(status) => {
                                    println!("✓ Cloned {} -> {} (branch: {}, commit: {:.8})", target_str, status.package_name, status.branch, status.commit_hash);
                                    status.spec_path
                                }
                                Err(e) => {
                                    eprintln!("Warning: Failed to auto-clone '{}' from distgit: {}", target_str, e);
                                    t
                                }
                            }
                        } else {
                            t
                        }
                    } else {
                        t
                    }
                }
            };
            resolved_targets.push(resolved);
        }
        args.targets = resolved_targets;
    }

    if args.targets.is_empty() {
        return Err(eyre!("No package targets specified to build. Provide target paths or a valid --packages file."));
    }

    println!("===========================================================");
    println!(" DBS Package Build Orchestrator");
    println!(" Runner:       {}", args.runner);
    println!(" Staging:      {}", output_dir.display());
    println!(" Targets:      {}", args.targets.len());
    println!(" Concurrency:  {} workers", concurrency);
    println!(" Dynamic Repo: {}", args.dynamic_repo);
    if record_db {
        println!(" Record to DB: enabled");
    }
    if args.force {
        println!(" Force Build:  enabled (-f/--force specified)");
    } else if args.skip_existing && dbs_cfg.build.skip_existing {
        println!(" Skip Built:   enabled (only building new versions)");
    }
    println!("===========================================================");

    let mut db_conn = if record_db {
        match db::establish_connection_with_url(dbs_cfg.database.url.as_deref()) {
            Ok(conn) => {
                println!("✓ Connected to database for build metrics recording.");
                Some(conn)
            }
            Err(e) => {
                eprintln!("Warning: Failed to connect to database: {}. Continuing without DB recording.", e);
                None
            }
        }
    } else {
        None
    };

    let should_check_existing = !args.force && (args.skip_existing && dbs_cfg.build.skip_existing);
    if should_check_existing {
        let mut targets_to_build = Vec::new();
        let mut skipped_targets = Vec::new();

        let dist_tag = ".tcrs";

        for target in &args.targets {
            match runner::gate::check_package_already_built(
                target,
                Some(&dbs_cfg.distro.dest),
                Some(&output_dir),
                db_conn.as_mut(),
                Some(dist_tag),
            ) {
                Ok(Some(existing)) => {
                    println!("✓ Package {}-{}-{} is already built and up-to-date ({}). Skipping.",
                        existing.name, existing.version, existing.release, existing.source);
                    if let Some(art) = &existing.artifact_path {
                        println!("  Artifact: {}", art.display());
                    }
                    skipped_targets.push((target.clone(), existing));
                }
                Ok(None) => {
                    targets_to_build.push(target.clone());
                }
                Err(_) => {
                    targets_to_build.push(target.clone());
                }
            }
        }

        if targets_to_build.is_empty() {
            println!("\nAll {} target package(s) are already built and up-to-date. Nothing to build.", args.targets.len());
            println!("Use '--force' or '-f' to rebuild existing packages.");
            return Ok(());
        }

        if !skipped_targets.is_empty() {
            println!("\nBuilding {} remaining package(s) (skipped {} already built)...",
                targets_to_build.len(), skipped_targets.len());
        }

        args.targets = targets_to_build;
    }

    if args.fetch_sources {
        println!("Checking and synchronizing sources into lookaside cache for targets...");
        for target in &args.targets {
            if !target.to_string_lossy().ends_with(".src.rpm") {
                let pkg_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                let sources_dir = runner::resolve_sources_dir(target);
                runner::ensure_sources_present(target, &sources_dir, pkg_stem, lookaside_dir.as_deref());
            }
        }
    }

    match args.runner.to_lowercase().as_str() {
        "mock" => {
            let chroot_spec = mock_root.unwrap_or_else(|| "tacos-rolling-x86_64".to_string());
            let mut runner = MockRunner::resolve(&chroot_spec, args.mock_config_dir.or_else(|| dbs_cfg.chroot.config_dir.clone()))?;
            let smp_effective = args.smp.or_else(|| {
                if args.targets.len() == 1 && args.concurrency.is_some() {
                    args.concurrency
                } else {
                    Some(dbs_cfg.chroot.smp_cpus)
                }
            });
            if let Some(smp) = smp_effective {
                runner = runner.with_smp_cpus(smp);
            }
            if let Some(l_dir) = lookaside_dir {
                runner = runner.with_lookaside_dir(l_dir);
            }
            if record_db
                && let Some(url) = dbs_cfg.database.url.as_ref() {
                    runner = runner.with_db_url(url.clone());
                }
            println!(" Chroot Profile: {}", runner.root_name);
            if let Some(cfg) = &runner.config_dir {
                println!(" Chroot Config:  {}", cfg.display());
            }
            if let Some(ld) = &runner.lookaside_dir {
                println!(" Lookaside Dir:  {}", ld.display());
            }
            if let Some(smp) = runner.smp_cpus {
                println!(" SMP Concurrency: {} cores (-j{})", smp, smp);
            }

            if args.chain {
                println!("Executing sequential Mock chain build for {} package(s)...", args.targets.len());
                let out = runner.build_chain(&args.targets, &output_dir, args.continue_on_error)?;
                print_build_output(&out);
                if let Some(conn) = &mut db_conn {
                    for target in &args.targets {
                        let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                        let _ = db::record_build_result(conn, name, &out);
                    }
                }
            } else if args.targets.len() == 1 || concurrency == 1 {
                for target in &args.targets {
                    println!("\nBuilding {} in Mock...", target.display());
                    let out = runner.build(target, &output_dir)?;
                    print_build_output(&out);
                    if let Some(conn) = &mut db_conn {
                        let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                        let _ = db::record_build_result(conn, name, &out);
                    }
                }
            } else {
                println!("Launching {} parallel Mock workers with dynamic local repo feedback...", concurrency);
                let runner = Arc::new(runner);
                let results = runner
                    .build_parallel(args.targets.clone(), output_dir.clone(), concurrency, args.dynamic_repo)
                    .await;

                for (idx, res) in results.into_iter().enumerate() {
                    match res {
                        Ok(out) => {
                            print_build_output(&out);
                            if let Some(conn) = &mut db_conn
                                && let Some(target) = args.targets.get(idx) {
                                    let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                    let _ = db::record_build_result(conn, name, &out);
                                }
                        }
                        Err(e) => eprintln!("✗ Worker build error: {}", e),
                    }
                }
            }
        }

        "rpmbuild" => {
            let runner = RpmbuildRunner;
            for target in &args.targets {
                let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                let log_path = output_dir.join("rpmbuild.log");
                if let Some(conn) = &mut db_conn {
                    let spec_meta = parse_spec_file(target).ok();
                    let _ = db::record_build_start(
                        conn,
                        name,
                        Some(1),
                        Some(&log_path.display().to_string()),
                        spec_meta.as_ref(),
                    );
                }
                println!("\nBuilding {} on host with rpmbuild...", target.display());
                let out = runner.build(target, &output_dir)?;
                print_build_output(&out);
                if let Some(conn) = &mut db_conn {
                    let _ = db::record_build_result(conn, name, &out);
                }
            }
        }

        other => {
            return Err(eyre!("Unsupported runner engine '{}'. Supported runners: mock, rpmbuild", other));
        }
    }

    Ok(())
}

/// Dispatches the legacy/deprecated `os` subcommand (forwards to `distro`).
pub async fn handle_os(args: OsArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    eprintln!("Notice: 'dbs os' is deprecated. Please use 'dbs distro' instead.\n");
    match args.command {
        cli::os::OsCommands::List => {
            handle_distro(
                DistroArgs {
                    action: DistroCommands::List,
                },
                dbs_cfg,
            )
            .await?;
        }
        cli::os::OsCommands::Add(add_args) => {
            handle_distro(
                DistroArgs {
                    action: DistroCommands::Add(add_args),
                },
                dbs_cfg,
            )
            .await?;
        }
        cli::os::OsCommands::Delete { id } => {
            handle_distro(
                DistroArgs {
                    action: DistroCommands::Delete { id },
                },
                dbs_cfg,
            )
            .await?;
        }
        cli::os::OsCommands::AddPackage(pkg_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::add_package(&mut conn, &pkg_args)?;
        }
        cli::os::OsCommands::Update(_) => {
            println!("Distribution update action is not yet implemented.");
        }
        cli::os::OsCommands::Build { .. } => {
            return Err(eyre!(
                "'dbs os build' is deprecated. To compile a distribution, use 'dbs distro build' or 'dbs build <targets>'."
            ));
        }
    }
    Ok(())
}

/// Dispatches the `pkg` subcommand.
pub async fn handle_pkg(args: PkgArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        cli::pkg::PkgCommands::List(list_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::pkg::list(&mut conn, &list_args)?;
        }
        cli::pkg::PkgCommands::Failed(mut list_args) => {
            list_args.failed = true;
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::pkg::list(&mut conn, &list_args)?;
        }
        cli::pkg::PkgCommands::Add(add_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::pkg::add(&mut conn, &add_args)?;
        }
        cli::pkg::PkgCommands::Delete { id } => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::pkg::delete(&mut conn, id)?;
        }
        cli::pkg::PkgCommands::Update(_) => {
            println!("Package update action is not yet implemented.");
        }
        cli::pkg::PkgCommands::Build(build_args) => {
            println!("Package build action for: {:?}", build_args);
        }
        cli::pkg::PkgCommands::Clean(clean_pkg_args) => {
            let pkg_name = if let Some(n) = clean_pkg_args.name {
                Some(n)
            } else if let Some(id) = clean_pkg_args.id {
                let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
                use diesel::prelude::*;
                match crate::schema::package::table
                    .filter(crate::schema::package::id.eq(id))
                    .load::<crate::models::Package>(&mut conn)
                {
                    Ok(pkgs) if !pkgs.is_empty() => Some(pkgs[0].name.clone()),
                    _ => return Err(eyre!("Package ID {} not found in database.", id)),
                }
            } else {
                None
            };
            let clean_args = CleanArgs {
                package: pkg_name,
                all: clean_pkg_args.all,
                staging_only: clean_pkg_args.staging_only,
                repo_only: clean_pkg_args.repo_only,
                clean_chroot: false,
                mock_root: None,
                mock_config_dir: None,
                staging_dir: None,
                repo_dir: None,
                arch: None,
                no_repo_update: false,
                no_db: false,
            };
            return handle_clean(clean_args, dbs_cfg).await;
        }
    }
    Ok(())
}

/// Displays parsed spec metadata in a formatted box.
fn print_spec_summary(spec_path: &Path, meta: &SpecMetadata) {
    println!("===========================================================");
    println!(" RPM Spec File Inspection: {}", spec_path.display());
    println!("===========================================================");
    println!("  Name:          {}", meta.name);
    println!("  EVR:           {}:{}-{}", meta.epoch, meta.version, meta.release);
    println!("  Summary:       {}", meta.summary);
    println!("  License:       {}", meta.license);
    println!("  URL:           {}", meta.url);
    println!("  Sources:       {}", meta.sources.len());
    for s in &meta.sources {
        println!("    * Source:    {}", s);
    }
    println!("  Patches:       {}", meta.patches.len());
    for p in &meta.patches {
        println!("    * Patch:     {}", p);
    }
    println!("  BuildRequires: {}", meta.build_requires.len());
    for br in &meta.build_requires {
        println!("    * BR:        {}", br);
    }
    println!("  Requires:      {}", meta.requires.len());
    for r in &meta.requires {
        println!("    * Req:       {}", r);
    }
    println!("===========================================================");
}

/// Displays build execution summary and output artifacts.
fn print_build_output(out: &BuildOutput) {
    let status_str = if out.success { "SUCCESS" } else { "FAILED" };
    println!("-----------------------------------------------------------");
    println!("  Build Status:  {}", status_str);
    println!("  Duration:      {:.1}s", out.duration_seconds);
    println!("  Log:           {}", out.log_path.display());
    println!("  Artifacts:     {}", out.artifacts.len());
    for art in &out.artifacts {
        println!("    * {}", art.display());
    }
    if let Some(err) = &out.error_summary {
        println!("  Failure:       {}", err);
    }
    println!("-----------------------------------------------------------");

    if !out.success
        && let Some(diag) = &out.error_diagnostic {
            println!("\n╔═══════════════════════════════════════════════════════════════════════════╗");
            println!("║                   DBS BUILD FAILURE DIAGNOSTIC SUMMARY                   ║");
            println!("╚═══════════════════════════════════════════════════════════════════════════╝");
            println!("  Target:     {}", out.target_name);
            if let Some(phase) = &diag.phase {
                println!("  Phase:      {}", phase);
            }
            println!("  Log Source: {}", diag.source_log.display());
            println!("  Root Cause: {}", diag.summary);
            println!("\n  >>> Diagnostic Log Excerpt (last {} lines):", diag.context_lines.len());
            for line in &diag.context_lines {
                println!("  │ {}", line);
            }
            println!("\n  >>> Recommended Next Actions:");
            println!("  • Inspect & debug interactively inside chroot: dbs shell {}", out.target_name);
            println!("  • Retry build after edit:                     dbs retry {}", out.target_name);
            println!("-----------------------------------------------------------\n");
    }
}

/// Dispatches the `dag` subcommand to compute topological build layers and optionally execute builds.
pub async fn handle_dag(args: DagArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let path = args.path.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
    let output_dir = args.output_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let mock_root = args.mock_root.or_else(|| Some(dbs_cfg.chroot.profile.clone()));
    let concurrency = args.concurrency.unwrap_or(dbs_cfg.distgit.concurrency);
    let lookaside_dir = args.lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
    let record_db = args.record_db || dbs_cfg.database.record_db;

    println!("===========================================================");
    println!(" DBS Topological Dependency Graph (DAG) Resolution");
    println!(" Repository/Spec Root: {}", path.display());
    println!("===========================================================");

    let mut graph = DependencyGraph::new();
    let loaded = match graph.load_from_dir(&path) {
        Ok(count) => count,
        Err(e) => return Err(eyre!("Failed loading packages from {}: {}", path.display(), e)),
    };

    if loaded == 0 {
        println!("No package .spec files found in {}", path.display());
        return Ok(());
    }

    let total_edges: usize = graph.dependencies.values().map(|s| s.len()).sum();
    let layers = graph.compute_layers()?;

    println!("  * Total Packages Loaded:       {}", graph.packages.len());
    println!("  * Total Dependency Edges:      {}", total_edges);
    println!("  * Total Compilation Layers:    {}", layers.len());
    println!("===========================================================");

    for layer in &layers {
        println!("\n▶ Layer {} ({} package(s) ready for parallel build):", layer.layer_index, layer.packages.len());
        for pkg in &layer.packages {
            println!("    * {}", pkg);
        }
    }
    println!("===========================================================");

    if let Some(report_path) = &args.report {
        let mut report = String::new();
        report.push_str("# DBS Dependency Graph & Compilation Order\n\n");
        report.push_str(&format!("* **Source Directory:** `{}`\n", path.display()));
        report.push_str(&format!("* **Total Packages:** {}\n", graph.packages.len()));
        report.push_str(&format!("* **Total Dependency Edges:** {}\n", total_edges));
        report.push_str(&format!("* **Compilation Layers:** {}\n\n", layers.len()));

        for layer in &layers {
            report.push_str(&format!("### Layer {} ({} packages)\n\n", layer.layer_index, layer.packages.len()));
            for pkg in &layer.packages {
                report.push_str(&format!("* `{}`\n", pkg));
            }
            report.push('\n');
        }

        if let Some(parent) = report_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::write(report_path, &report) {
            Ok(_) => println!("✓ Dependency graph report generated at {}", report_path.display()),
            Err(e) => eprintln!("✗ Failed to write report: {}", e),
        }
    }

    if args.fetch_sources {
        let lookaside_mgr = LookasideManager::resolve_default(
            lookaside_dir.as_deref());
        println!("\n▶ Synchronizing source archives into lookaside cache ({}) \
            for packages in {}...", lookaside_mgr.root.display(),
            path.display());
        match lookaside_mgr.sync_dir(&path, concurrency).await {
            Ok(report) => {
                if report.total_sources_found > 0 {
                    println!("✓ Lookaside synchronization: {} already cached, {} downloaded, {} failed (total: {})",
                        report.already_cached, report.downloaded, report.failed, report.total_sources_found);
                }
            }
            Err(e) => {
                eprintln!("Warning: Failed to synchronize lookaside sources: {}", e);
            }
        }
    }

    if args.build {
        println!("\nExecuting layered build orchestration with runner '{}'...", args.runner);
        let chroot_spec = mock_root.unwrap_or_else(|| "tacos-rolling-x86_64".to_string());
        let mut mock_runner = MockRunner::resolve(&chroot_spec, args.mock_config_dir.or_else(|| dbs_cfg.chroot.config_dir.clone()))?;
        mock_runner = mock_runner.with_smp_cpus(dbs_cfg.chroot.smp_cpus);
        if let Some(l_dir) = lookaside_dir {
            mock_runner = mock_runner.with_lookaside_dir(l_dir);
        }
        if record_db
            && let Some(url) = dbs_cfg.database.url.as_ref() {
                mock_runner = mock_runner.with_db_url(url.clone());
            }
        println!(" Chroot Profile: {}", mock_runner.root_name);
        if let Some(cfg) = &mock_runner.config_dir {
            println!(" Chroot Config:  {}", cfg.display());
        }
        if let Some(ld) = &mock_runner.lookaside_dir {
            println!(" Lookaside Dir:  {}", ld.display());
        }
        if let Some(smp) = mock_runner.smp_cpus {
            println!(" SMP Concurrency: {} cores (-j{})", smp, smp);
        }
        let runner_arc = Arc::new(mock_runner);

        let mut db_conn = if record_db {
            match db::establish_connection_with_url(dbs_cfg.database.url.as_deref()) {
                Ok(conn) => {
                    println!("✓ Connected to database for DAG build recording.");
                    Some(conn)
                }
                Err(e) => {
                    eprintln!("Warning: Failed to connect to database: {}. Continuing without DB recording.", e);
                    None
                }
            }
        } else {
            None
        };

        for layer in &layers {
            println!("\n===========================================================");
            println!(" Starting Compilation Layer {} ({} package(s))", layer.layer_index, layer.packages.len());
            println!("===========================================================");

            let mut targets = Vec::new();
            for pkg in &layer.packages {
                if let Some(meta) = graph.packages.get(pkg)
                    && let Some(spec) = &meta.spec_path {
                        targets.push(spec.clone());
                    }
            }

            if !targets.is_empty() {
                println!("Building {} package(s) in parallel...", targets.len());
                let results = runner_arc
                    .clone()
                    .build_parallel(targets.clone(), output_dir.clone(), concurrency, args.dynamic_repo)
                    .await;

                for (idx, res) in results.into_iter().enumerate() {
                    match res {
                        Ok(out) => {
                            print_build_output(&out);
                            if let Some(conn) = &mut db_conn
                                && let Some(target) = targets.get(idx) {
                                    let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                    let _ = db::record_build_result(conn, name, &out);
                                }
                        }
                        Err(e) => eprintln!("✗ Worker build error: {}", e),
                    }
                }
            }
        }
    }

    Ok(())
}

/// Dispatches the `chroot` subcommand to list, inspect, check, add, or init Mock configurations.
pub async fn handle_chroot(args: ChrootArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        ChrootCommands::List { dir, all } => {
            let dir = dir.or_else(|| dbs_cfg.chroot.config_dir.clone());
            println!("===========================================================");
            println!(" DBS Discovered Mock Chroot Configurations");
            if let Some(custom) = &dir {
                println!(" Custom Directory: {}", custom.display());
            }
            println!(" System Configs:   {}", if all { "included (/etc/mock)" } else { "excluded (use --all to show)" });
            println!("===========================================================");

            let configs = chroot::ChrootResolver::list_all(dir.as_deref(), all)?;
            if configs.is_empty() {
                println!("No chroot configurations found.");
                println!("Hint: You can import or initialize a config with 'dbs chroot add <path>' or 'dbs chroot init <name>'.");
            } else {
                for c in configs {
                    let status = if c.valid { "✓ VALID" } else { "✗ INVALID" };
                    println!("  * [{}] {}", status, c.name);
                    println!("    Path:         {}", c.path.display());
                    if let Some(arch) = &c.target_arch {
                        println!("    Architecture: {}", arch);
                    }
                    if let Some(pkg_mgr) = &c.package_manager {
                        println!("    Package Mgr:  {}", pkg_mgr);
                    }
                    if let Some(release) = &c.releasever {
                        println!("    Releasever:   {}", release);
                    }
                    if let Some(desc) = &c.description {
                        println!("    Description:  {}", desc);
                    }
                    if !c.includes.is_empty() {
                        println!("    Includes:     {}", c.includes.join(", "));
                    }
                    if let Some(err) = &c.validation_error {
                        println!("    Error:        {}", err);
                    }
                    println!();
                }
            }
            println!("===========================================================");
        }

        ChrootCommands::Inspect { target, dir } => {
            let target = target.unwrap_or_else(|| dbs_cfg.chroot.profile.clone());
            let dir = dir.or_else(|| dbs_cfg.chroot.config_dir.clone());
            let resolved = match chroot::ChrootResolver::resolve(&target, dir.as_deref()) {
                Ok(r) => r,
                Err(e) => return Err(eyre!("Failed to resolve chroot '{}': {}", target, e)),
            };
            let config = chroot::ChrootResolver::parse_config(&resolved.config_path);

            println!("===========================================================");
            println!(" Mock Chroot Configuration Inspection: {}", config.name);
            println!("===========================================================");
            println!("  Profile Name:     {}", config.name);
            println!("  Config File:      {}", config.path.display());
            println!("  Config Directory: {}", config.config_dir.display());
            println!("  Target Arch:      {}", config.target_arch.as_deref().unwrap_or("unspecified"));
            println!("  Package Manager:  {}", config.package_manager.as_deref().unwrap_or("dnf"));
            println!("  Release Version:  {}", config.releasever.as_deref().unwrap_or("unspecified"));
            println!("  Dist Macro:       {}", config.dist.as_deref().unwrap_or("unspecified"));
            println!("  Vendor Macro:     {}", config.vendor.as_deref().unwrap_or("unspecified"));
            println!("  Bootstrap Image:  {}", config.bootstrap_image.as_deref().unwrap_or("none"));
            if let Some(desc) = &config.description {
                println!("  Description:      {}", desc);
            }
            println!("  Template Includes ({}):", config.includes.len());
            for inc in &config.includes {
                let full = config.config_dir.join(inc);
                let exists = if full.is_file() { "found" } else { "MISSING" };
                println!("    * {} [{}] ({})", inc, exists, full.display());
            }
            println!("  Status:           {}", if config.valid { "✓ Valid and complete" } else { "✗ Invalid" });
            println!("  SMP Concurrency:  {}", config.smp_cpus.map(|c| format!("{} cores (-j{})", c, c)).unwrap_or_else(|| "unspecified (dynamic)".to_string()));
            if let Some(err) = &config.validation_error {
                println!("  Validation Error: {}", err);
            }
            println!("===========================================================");
        }

        ChrootCommands::Check { target, dir } => {
            let target = target.unwrap_or_else(|| dbs_cfg.chroot.profile.clone());
            let dir = dir.or_else(|| dbs_cfg.chroot.config_dir.clone());
            println!("===========================================================");
            println!(" Testing Mock Chroot Configuration: {}", target);
            println!("===========================================================");
            let report = match chroot::ChrootResolver::check(&target, dir.as_deref()) {
                Ok(r) => r,
                Err(e) => return Err(eyre!("Failed to check chroot '{}': {}", target, e)),
            };

            println!("  Profile Name:     {}", report.config.name);
            println!("  Config File:      {}", report.config.path.display());
            println!("  Architecture:     {}", report.config.target_arch.as_deref().unwrap_or("unknown"));
            println!("  Package Manager:  {}", report.config.package_manager.as_deref().unwrap_or("unknown"));
            println!("  SMP Concurrency:  {}", report.config.smp_cpus.map(|c| format!("{} cores (-j{})", c, c)).unwrap_or_else(|| "unspecified (dynamic)".to_string()));
            println!("  Templates Valid:  {}", if report.config.valid { "✓ All includes found" } else { "✗ Missing template" });

            if report.mock_verified {
                println!("  Mock Verification: ✓ Success");
                if let Some(root_path) = &report.mock_root_path {
                    println!("  Mock Root Path:    {}", root_path);
                }
            } else {
                println!("  Mock Verification: ✗ Failed");
                if let Some(err) = &report.error_message {
                    println!("  Error Details:     {}", err);
                }
            }
            println!("===========================================================");

            if !report.mock_verified {
                return Err(eyre!("Mock verification failed for chroot '{}'", target));
            }
        }

        ChrootCommands::Add { path, dest } => {
            println!("Importing chroot configuration from {} into {}...", path.display(), dest.display());
            let copied_path = match chroot::ChrootResolver::add(&path, &dest) {
                Ok(p) => p,
                Err(e) => return Err(eyre!("Failed to import chroot from '{}': {}", path.display(), e)),
            };
            println!("✓ Successfully imported configuration: {}", copied_path.display());
            println!("Verifying imported configuration with Mock...");
            match chroot::ChrootResolver::check(copied_path.to_str().unwrap_or_default(), Some(&dest)) {
                Ok(check) => {
                    if check.mock_verified {
                        println!("✓ Chroot verified by Mock successfully.");
                    } else {
                        eprintln!("Warning: Mock check reported an issue: {:?}", check.error_message);
                    }
                }
                Err(e) => eprintln!("Warning: Mock verification encountered an error: {}", e),
            }
        }

        ChrootCommands::Init { name, arch, dest, smp_cpus } => {
            println!("Initializing new Mock chroot configuration '{}' (arch: {}) in {}...", name, arch, dest.display());
            let created_path = match chroot::ChrootResolver::init(&name, &arch, &dest, smp_cpus) {
                Ok(p) => p,
                Err(e) => return Err(eyre!("Failed to initialize chroot '{}': {}", name, e)),
            };
            println!("✓ Successfully initialized chroot configuration: {}", created_path.display());
            println!("  Template created at: {}/templates/{}.tpl", dest.display(), name);
            println!("You can customize this configuration and verify it using 'dbs chroot check {}'", name);
        }

        ChrootCommands::Shell(shell_args) => {
            handle_shell(shell_args, dbs_cfg).await?;
        }
    }

    Ok(())
}

/// Dispatches the `lookaside` subcommand for managing source archive storage and synchronization.
pub async fn handle_lookaside(args: LookasideArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let dir = args.dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
    let mgr = LookasideManager::resolve_default(dir.as_deref());

    match args.command {
        LookasideCommands::Upload { file, pkg, spec, no_sources } => {
            println!("===========================================================");
            println!(" DBS Dist-git Lookaside Uploader");
            println!(" Repository: {}", mgr.root.display());
            println!(" Package:    {}", pkg);
            println!(" File:       {}", file.display());
            println!(" FS Type:    {}", if mgr.is_btrfs_fs() { "BTRFS (FICLONE CoW reflink enabled)" } else { "Standard filesystem" });
            println!("===========================================================");

            let res = mgr.upload(&file, &pkg, spec.as_deref(), !no_sources)?;

            println!("\n✓ Archive successfully registered in lookaside:");
            println!("  * Package:       {}", res.package);
            println!("  * File:          {}", res.filename);
            println!("  * Size:          {:.2} MB", res.size_bytes as f64 / (1024.0 * 1024.0));
            println!("  * SHA-512:       {}", res.hash);
            println!("  * Storage Mode:  {}", res.reflink_mode);
            println!("  * CAS Path:      {}", res.cas_path.display());
            println!("  * Dist-git Path: {}", res.pkgs_path.display());
            if let Some(m) = res.manifest_updated {
                println!("  * Manifest:      Updated {}", m.display());
            }
        }

        LookasideCommands::Get { pkg, file, hash, dest } => {
            println!("Retrieving {} ({:.12}...) into {}...", file, hash, dest.display());
            let target_dest = if dest.is_dir() {
                dest.join(&file)
            } else {
                dest
            };

            let mode = mgr.get_or_fetch(&pkg, &file, &hash, &target_dest)?;
            println!("✓ Staged: {} ({})", target_dest.display(), mode);
        }

        LookasideCommands::Sync { path, concurrency } => {
            println!("===========================================================");
            println!(" DBS Dist-git Lookaside Cache Synchronizer");
            println!(" Lookaside Root: {}", mgr.root.display());
            println!(" Dist-git Path:  {}", path.display());
            println!(" Concurrency:    {} tasks", concurrency);
            println!("===========================================================");

            let report = mgr.sync_dir(&path, concurrency).await?;
            println!("\nSync completed:");
            println!("  * Total Archives Declared: {}", report.total_sources_found);
            println!("  * Already Cached Locally:  {}", report.already_cached);
            println!("  * Successfully Fetched:    {}", report.downloaded);
            println!("  * Failed Downloads:        {}", report.failed);
            for f in &report.failures {
                eprintln!("    ✗ {}", f);
            }
        }

        LookasideCommands::Status => {
            let status = mgr.status()?;
            println!("===========================================================");
            println!(" DBS Dist-git Lookaside Status & Storage Metrics");
            println!("===========================================================");
            println!(" Root Directory:        {}", status.root.display());
            println!(" Filesystem Engine:     {}", if status.is_btrfs { "BTRFS (Reflinks & CoW Compression Active)" } else { "Non-BTRFS (Standard / Ext4)" });
            println!(" CAS Unique Archives:   {}", status.total_cas_objects);
            println!(" CAS Physical Storage:  {:.2} MB ({:.2} GB)", 
                status.total_cas_bytes as f64 / (1024.0 * 1024.0),
                status.total_cas_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
            );
            println!(" Distinct Packages:     {}", status.total_packages);
            println!(" Dist-git Exposed Files:{}", status.total_package_entries);

            if status.is_btrfs {
                println!("\n💡 BTRFS Storage Optimizations:");
                println!("  * CoW Reflinks:        Enabled (0 disk overhead for staged packages)");
                println!("  * Force Compression:   btrfs filesystem defragment -r -czstd:3 {}", status.root.display());
                println!("  * Block Deduplication: duperemove -drh {}", status.root.display());
                println!("  * Subvolume Snapshots: btrfs subvolume snapshot -r {} <snapshot-path>", status.root.display());
            } else {
                println!("\n💡 Storage Recommendation:");
                println!("  Mount a dedicated BTRFS subvolume at /srv/dbs/lookaside with 'compress=zstd:3' to unlock");
                println!("  instant FICLONE zero-disk copies, block deduplication, and subvolume snapshot replication.");
            }
            println!("===========================================================");
        }

        LookasideCommands::Gc { dry_run, distgit } => {
            println!("Running Lookaside Garbage Collection (dry_run: {})...", dry_run);
            let report = mgr.gc(dry_run, distgit.as_deref())?;
            println!("  * Scanned CAS Objects: {}", report.scanned_objects);
            println!("  * Orphaned Objects:    {}", report.orphaned_objects.len());
            println!("  * Reclaimable Space:   {:.2} MB", report.reclaimed_bytes as f64 / (1024.0 * 1024.0));
            for orphan in &report.orphaned_objects {
                println!("    {} {}", if dry_run { "[DRY RUN] Would delete:" } else { "Pruned:" }, orphan.display());
            }
        }
    }

    Ok(())
}

/// Dispatches the `distro` subcommand to manage, build, publish, and serve distribution repositories.
pub async fn handle_distro(args: DistroArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.action {
        DistroCommands::List => {
            println!("===========================================================");
            println!(" DBS Supported Distribution Presets (Static)");
            println!("===========================================================");
            for p in DistroConfig::all_presets() {
                println!("  * Name:         {} ({})", p.name, p.version);
                println!("    Dist-Git:     {}", p.dist_git_url_template);
                println!("    Branch:       {}", p.dist_git_branch);
                println!("    API Type:     {}", p.api_type);
                if let Some(chroot) = p.mock_chroot {
                    println!("    Mock Root:    {}", chroot);
                }
                println!();
            }
            println!("===========================================================");

            match db::establish_connection_with_url(dbs_cfg.database.url.as_deref()) {
                Ok(mut conn) => {
                    let _ = cli::os_actions::list(&mut conn);
                }
                Err(e) => {
                    println!("Note: Database not connected ({})", e);
                }
            }
        }

        DistroCommands::Add(add_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::add(&mut conn, &add_args)?;
        }

        DistroCommands::Delete { id } => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::delete(&mut conn, id)?;
        }

        DistroCommands::Init {
            name,
            arch,
            channel,
            releasever,
            dist,
            dest,
            mock_dir,
            smp_cpus: _,
        } => {
            let dest = dest.unwrap_or_else(|| dbs_cfg.distro.dest.clone());
            let mock_dir = mock_dir
                .or_else(|| dbs_cfg.chroot.config_dir.clone())
                .unwrap_or_else(|| PathBuf::from("mock"));

            println!("===========================================================");
            println!(" DBS Distribution Repository Initialization");
            println!(" Target Distribution: {}", name);
            println!(" Architecture:        {}", arch);
            println!(" Channel:             {}", channel);
            println!(" Base Directory:      {}", dest.display());
            println!("===========================================================");

            let opts = DistroInitOptions {
                name: name.clone(),
                arch,
                channel,
                releasever,
                dist_tag: dist,
                dest_root: dest,
                mock_dir: mock_dir.clone(),
            };

            let path = match init_distro(&opts) {
                Ok(p) => p,
                Err(e) => return Err(eyre!("Failed to initialize distribution: {}", e)),
            };

            println!("✓ Distribution repository structure created at {}", path.display());
            println!("✓ Mock configuration initialized at {}/{}.cfg", mock_dir.display(), name);
            println!("===========================================================");
            println!("Next step: Place .spec files in your dist-git directory and run:");
            println!("  dbs distro build {}", name);
        }

        DistroCommands::Build {
            name,
            path,
            mock_root,
            mock_config_dir,
            dest,
            lookaside_dir,
            concurrency,
            smp,
            staging_dir,
            sign_key,
            record_db,
            stage,
            stages,
            packages,
            break_cycles,
        } => {
            let name = name.unwrap_or_else(|| dbs_cfg.distro.name.clone());
            let path = path.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let mock_root = mock_root
                .or_else(|| dbs_cfg.distro.chroot.clone())
                .unwrap_or_else(|| dbs_cfg.chroot.profile.clone());
            let mock_config_dir = mock_config_dir.or_else(|| dbs_cfg.chroot.config_dir.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distro.dest.clone());
            let lookaside_dir = lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
            let concurrency = concurrency.unwrap_or(dbs_cfg.distro.workers);
            let smp = smp.unwrap_or(dbs_cfg.chroot.smp_cpus);
            let staging_dir = staging_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
            let sign_key = sign_key.or_else(|| dbs_cfg.distro.sign_key.clone());
            let record_db = record_db || dbs_cfg.database.record_db;

            println!("===========================================================");
            println!(" DBS Distribution Build Orchestration");
            println!(" Target Distribution: {}", name);
            println!(" Spec/Dist-Git Root:  {}", path.display());
            println!(" Staging Directory:   {}", staging_dir.display());
            println!(" Concurrency:         {}", concurrency);
            println!(" SMP Concurrency:     {} cores (-j{})", smp, smp);
            println!(" Cycle Breaker:       {}", if break_cycles { "Enabled (Base chroot fallback)" } else { "Strict (Fail on cycle)" });
            println!("===========================================================");

            // 1. Determine execution plan: Stages vs Manifest vs Full Directory
            struct StageTask {
                name: String,
                spec_paths: Option<Vec<PathBuf>>,
            }

            let mut tasks: Vec<StageTask> = Vec::new();

            if let Some(pkg_file) = packages {
                println!("\n▶ Loading package targets from manifest file: {}", pkg_file.display());
                let targets = runner::load_packages_from_file(&pkg_file, &path)?;
                tasks.push(StageTask {
                    name: format!("manifest ({})", pkg_file.display()),
                    spec_paths: Some(targets),
                });
            } else if let Some(selected_stage) = stage {
                let stage_pkgs = match dbs_cfg.distro.stages.get(&selected_stage) {
                    Some(pkgs) => pkgs,
                    None => return Err(eyre!(
                        "Stage '{}' not found in configuration. Available stages: {:?}",
                        selected_stage,
                        dbs_cfg.distro.stages.keys().collect::<Vec<_>>()
                    )),
                };
                let mut spec_paths = Vec::new();
                for target_entry in stage_pkgs {
                    let as_path = Path::new(target_entry);
                    if as_path.is_file() {
                        let sub_targets = runner::load_packages_from_file(as_path, &path)?;
                        spec_paths.extend(sub_targets);
                    } else {
                        match runner::resolve_package_target(target_entry, &path) {
                            Ok(spec) => spec_paths.push(spec),
                            Err(e) => eprintln!("Warning: stage '{}' skipping unresolved target '{}': {}", selected_stage, target_entry, e),
                        }
                    }
                }
                tasks.push(StageTask {
                    name: selected_stage,
                    spec_paths: Some(spec_paths),
                });
            } else if stages || !dbs_cfg.distro.stages.is_empty() {
                let ordered_names: Vec<String> = if let Some(order) = &dbs_cfg.distro.stage_order {
                    order.clone()
                } else {
                    let mut keys: Vec<String> = dbs_cfg.distro.stages.keys().cloned().collect();
                    keys.sort();
                    keys
                };

                for stage_name in ordered_names {
                    if let Some(stage_pkgs) = dbs_cfg.distro.stages.get(&stage_name) {
                        let mut spec_paths = Vec::new();
                        for target_entry in stage_pkgs {
                            let as_path = Path::new(target_entry);
                            if as_path.is_file() {
                                let sub_targets = runner::load_packages_from_file(as_path, &path)?;
                                spec_paths.extend(sub_targets);
                            } else {
                                match runner::resolve_package_target(target_entry, &path) {
                                    Ok(spec) => spec_paths.push(spec),
                                    Err(e) => eprintln!("Warning: stage '{}' skipping unresolved target '{}': {}", stage_name, target_entry, e),
                                }
                            }
                        }
                        if !spec_paths.is_empty() {
                            tasks.push(StageTask {
                                name: stage_name,
                                spec_paths: Some(spec_paths),
                            });
                        }
                    }
                }
            } else {
                tasks.push(StageTask {
                    name: "full-distribution".to_string(),
                    spec_paths: None,
                });
            }

            if tasks.is_empty() {
                return Err(eyre!("No package specifications or stages resolved to build."));
            }

            // 2. Ensure lookaside sources are synchronized for full-distribution builds if needed
            let is_full_dist = tasks.iter().any(|t| t.spec_paths.is_none());
            if is_full_dist {
                let lookaside_mgr = LookasideManager::resolve_default(lookaside_dir.as_deref());
                println!("\n▶ Synchronizing source archives into lookaside cache ({})...", lookaside_mgr.root.display());
                match lookaside_mgr.sync_dir(&path, concurrency).await {
                    Ok(rep) => {
                        println!("✓ Lookaside sources: {} cached, {} downloaded, {} failed (total: {})",
                            rep.already_cached, rep.downloaded, rep.failed, rep.total_sources_found);
                    }
                    Err(e) => eprintln!("Warning: Lookaside sync issue: {}", e),
                }
            }

            if tasks.is_empty() {
                return Err(eyre!("No package specifications or stages resolved to build."));
            }

            // 3. Setup Mock runner engine
            let mut mock_runner = MockRunner::resolve(&mock_root, mock_config_dir)?;
            mock_runner = mock_runner.with_smp_cpus(smp);
            if let Some(ld) = lookaside_dir.clone() {
                mock_runner = mock_runner.with_lookaside_dir(ld);
            }
            if record_db
                && let Some(url) = dbs_cfg.database.url.as_ref() {
                    mock_runner = mock_runner.with_db_url(url.clone());
                }
            let runner_arc = Arc::new(mock_runner);

            let mut db_conn = if record_db {
                db::establish_connection_with_url(dbs_cfg.database.url.as_deref()).ok()
            } else {
                None
            };

            // 4. Execute stages sequentially
            let total_stages = tasks.len();
            for (stage_idx, stage_task) in tasks.into_iter().enumerate() {
                println!("\n===========================================================");
                println!(" Stage [{}/{}] {}", stage_idx + 1, total_stages, stage_task.name);
                println!("===========================================================");

                let mut graph = DependencyGraph::new();
                let loaded = if let Some(specs) = &stage_task.spec_paths {
                    graph.load_package_targets(specs)?
                } else {
                    graph.load_from_dir(&path)?
                };

                if loaded == 0 {
                    println!("No valid package .spec files found in stage '{}'. Skipping.", stage_task.name);
                    continue;
                }

                let (layers, broken_edges) = if break_cycles {
                    graph.compute_layers_with_cycle_breaker()?
                } else {
                    let l = graph.compute_layers()?;
                    (l, Vec::new())
                };

                if !broken_edges.is_empty() {
                    println!("ℹ Resolved {} circular dependency edge(s) via base chroot fallback:", broken_edges.len());
                    for b in &broken_edges {
                        println!("  • Severed cyclic edge: '{}' -> '{}'", b.consumer, b.prerequisite);
                    }
                }
                println!("✓ Scheduled {} packages across {} compilation layers.", graph.packages.len(), layers.len());

                // Execute layered builds with dynamic repo
                for layer in &layers {
                    println!("\n-----------------------------------------------------------");
                    println!(" Stage '{}' - Layer {} ({} package(s))", stage_task.name, layer.layer_index, layer.packages.len());
                    println!("-----------------------------------------------------------");

                    let mut targets = Vec::new();
                    for pkg in &layer.packages {
                        if let Some(meta) = graph.packages.get(pkg)
                            && let Some(spec) = &meta.spec_path {
                                targets.push(spec.clone());
                            }
                    }

                    let mut layer_targets = Vec::new();
                    for target in targets {
                        if dbs_cfg.build.skip_existing
                            && let Ok(Some(existing)) = runner::gate::check_package_already_built(
                                &target,
                                Some(&dbs_cfg.distro.dest),
                                Some(&staging_dir),
                                db_conn.as_mut(),
                                Some(".tcrs"),
                            ) {
                                println!("✓ Layer package {}-{}-{} is already built ({}). Skipping.",
                                    existing.name, existing.version, existing.release, existing.source);
                                continue;
                            }
                        layer_targets.push(target);
                    }

                    if layer_targets.is_empty() {
                        println!("All packages in Layer {} are already built. Skipping layer.", layer.layer_index);
                        continue;
                    }

                    let targets = layer_targets;

                    let results = runner_arc
                        .clone()
                        .build_parallel(targets.clone(), staging_dir.clone(), concurrency, true)
                        .await;

                    for (idx, res) in results.into_iter().enumerate() {
                        match res {
                            Ok(out) => {
                                print_build_output(&out);
                                if let Some(conn) = &mut db_conn
                                    && let Some(target) = targets.get(idx) {
                                        let pkg_name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                        let _ = db::record_build_result(conn, pkg_name, &out);
                                    }
                            }
                            Err(e) => eprintln!("✗ Worker build error: {}", e),
                        }
                    }
                }

                // Automatically publish stage artifacts into distribution repository so subsequent stages have immediate access
                println!("\n▶ Publishing stage '{}' artifacts to repository...", stage_task.name);
                let pub_opts = DistroPublishOptions {
                    name: name.clone(),
                    staging_dir: staging_dir.clone(),
                    dest_root: dest.clone(),
                    arch: dbs_cfg.distro.arch.clone(),
                    base_url: dbs_cfg.distro.base_url.clone(),
                    sign_key: sign_key.clone(),
                    workers: concurrency,
                };
                let _ = publish_distro(&pub_opts);
            }

            // 5. Final distribution repository publication & report
            println!("\n▶ Generating final distribution repository metadata & client configs...");
            let pub_opts = DistroPublishOptions {
                name: name.clone(),
                staging_dir,
                dest_root: dest.clone(),
                arch: dbs_cfg.distro.arch.clone(),
                base_url: dbs_cfg.distro.base_url.clone(),
                sign_key,
                workers: concurrency,
            };

            let report = publish_distro(&pub_opts)?;
            println!("\n===========================================================");
            println!(" Distribution Build & Publication Complete!");
            println!(" Total Stages Executed: {}", total_stages);
            println!(" Binary Repository:   {}", report.binary_repo.display());
            println!(" Total Binary RPMs:   {}", report.binary_count);
            println!(" Total Source RPMs:   {}", report.source_count);
            println!(" Client Repo File:    {}", report.client_repo_file.display());
            println!(" GPG Signed:          {}", if report.gpg_signed { "Yes" } else { "No" });
            println!("===========================================================");
        }

        DistroCommands::Publish {
            name,
            staging_dir,
            dest,
            arch,
            base_url,
            sign_key,
            workers,
        } => {
            let name = name.unwrap_or_else(|| dbs_cfg.distro.name.clone());
            let staging_dir = staging_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distro.dest.clone());
            let arch = arch.unwrap_or_else(|| dbs_cfg.distro.arch.clone());
            let base_url = base_url.unwrap_or_else(|| dbs_cfg.distro.base_url.clone());
            let sign_key = sign_key.or_else(|| dbs_cfg.distro.sign_key.clone());
            let workers = workers.unwrap_or(dbs_cfg.distro.workers);

            println!("===========================================================");
            println!(" DBS Distribution Repository Publishing");
            println!(" Distribution:        {}", name);
            println!(" Staging Directory:   {}", staging_dir.display());
            println!(" Output Directory:    {}/{}", dest.display(), name);
            println!(" Base URL:            {}", base_url);
            println!(" Architecture:        {}", arch);
            println!("===========================================================");

            let opts = DistroPublishOptions {
                name: name.clone(),
                staging_dir,
                dest_root: dest,
                arch,
                base_url,
                sign_key,
                workers,
            };

            let report = publish_distro(&opts)?;
            println!("\n===========================================================");
            println!(" Repository Publication Successful");
            println!(" * Published Binaries:  {}", report.binary_count);
            println!(" * Published Sources:   {}", report.source_count);
            println!(" * Repodata Location:   {}/repodata/", report.binary_repo.display());
            println!(" * Client Config File:  {}", report.client_repo_file.display());
            println!(" * GPG Signed:          {}", if report.gpg_signed { "Yes" } else { "No" });
            println!("===========================================================");
        }

        DistroCommands::Serve {
            path,
            nginx_conf,
            server_name,
            port,
            host,
        } => {
            let path = path.unwrap_or_else(|| dbs_cfg.distro.dest.clone());
            let server_name = server_name.unwrap_or_else(|| dbs_cfg.distro.server_name.clone());
            let port = port.unwrap_or(dbs_cfg.distro.server_port);
            let host = host.unwrap_or_else(|| dbs_cfg.distro.server_host.clone());

            if let Some(out_conf) = nginx_conf {
                println!("Generating Nginx virtual host configuration...");
                let content = generate_nginx_config(&path, &server_name, port);
                if let Some(parent) = out_conf.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                match fs::write(&out_conf, content) {
                    Ok(_) => {
                        println!("✓ Nginx configuration written to {}", out_conf.display());
                        println!("  Apply with: sudo cp {} /etc/nginx/conf.d/ && sudo nginx -t && sudo systemctl reload nginx", out_conf.display());
                    }
                    Err(e) => return Err(eyre!("Failed to write Nginx config: {}", e)),
                }
            } else {
                run_http_server(path, &host, port).await?;
            }
        }

        DistroCommands::Status { name, dest, arch } => {
            let name = name.unwrap_or_else(|| dbs_cfg.distro.name.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distro.dest.clone());
            let arch = arch.unwrap_or_else(|| dbs_cfg.distro.arch.clone());

            println!("===========================================================");
            println!(" DBS Distribution Repository Status: {}", name);
            println!("===========================================================");

            let st = get_distro_status(&name, &dest, &arch)?;
            println!(" Repository Directory: {}", st.root_dir.display());
            println!(" Target Architecture:  {}", st.arch);
            println!(" Binary Packages:      {}", st.binary_count);
            println!(" Source Packages:      {}", st.source_count);
            println!(" Repodata Generated:   {}", if st.repodata_present { "Yes" } else { "No (Run 'dbs distro publish')" });
            println!(" GPG Signed:           {}", if st.gpg_signed { "Yes (repomd.xml.asc verified)" } else { "No" });
            if let Some(cf) = st.client_repo_file {
                println!(" Client Configuration: {}", cf.display());
            }
            println!("===========================================================");
        }
    }

    Ok(())
}

/// Dispatches the `db` subcommand for database bootstrapping, status check, reset, and dumping schema.
pub async fn handle_db(args: DbArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    if let DbCommands::DumpSchema { down } = args.command {
        let sql = db::bootstrap::dump_schema(down);
        print!("{}", sql);
        return Ok(());
    }

    let db_url = args.database_url.as_deref().or(dbs_cfg.database.url.as_deref());
    let mut conn = db::establish_connection_with_url(db_url)?;

    match args.command {
        DbCommands::Bootstrap { force } => {
            println!("===========================================================");
            println!(" DBS Database Bootstrap");
            println!("===========================================================");
            db::bootstrap::bootstrap_database(&mut conn, force)?;
            println!("-----------------------------------------------------------");
            let status = db::bootstrap::get_database_status(&mut conn)?;
            println!(" Database:                {}", status.database_name);
            println!(" PostgreSQL:              {}", status.server_version.lines().next().unwrap_or(""));
            println!(" Registered OS presets:   {}", status.os_count);
            println!(" Supported architectures: {}", status.arch_count);
            println!(" Package managers:        {}", status.pm_count);
            println!(" Initial packages:        {}", status.package_count);
            println!("===========================================================");
        }

        DbCommands::Status => {
            println!("===========================================================");
            println!(" DBS Database Status");
            println!("===========================================================");
            let status = db::bootstrap::get_database_status(&mut conn)?;
            println!(" Database:         {}", status.database_name);
            println!(" PostgreSQL:       {}", status.server_version.lines().next().unwrap_or(""));
            println!(" Schema Status:    {}", if status.is_initialized { "INITIALIZED" } else { "NOT INITIALIZED" });
            println!("-----------------------------------------------------------");
            println!(" Operating Systems: {}", status.os_count);
            println!(" Architectures:     {}", status.arch_count);
            println!(" Package Managers:  {}", status.pm_count);
            println!(" Packages:          {}", status.package_count);
            println!(" Package Artifacts: {}", status.artifact_count);
            println!("===========================================================");
        }

        DbCommands::Reset { force } => {
            if !force {
                return Err(eyre!(
                    "Resetting the database will drop all tables and data. Pass '--force' to proceed."
                ));
            }
            println!("===========================================================");
            println!(" DBS Database Reset");
            println!("===========================================================");
            db::bootstrap::reset_database(&mut conn)?;
            println!("-----------------------------------------------------------");
            let status = db::bootstrap::get_database_status(&mut conn)?;
            println!(" Database:         {}", status.database_name);
            println!(" Initialized:      {}", if status.is_initialized { "YES" } else { "NO" });
            println!(" Operating Systems: {}", status.os_count);
            println!(" Architectures:     {}", status.arch_count);
            println!(" Packages:          {}", status.package_count);
            println!("===========================================================");
        }

        DbCommands::Reconcile { path } => {
            let distgit_dir = path.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            println!("===========================================================");
            println!(" DBS Database Macro Reconciliation");
            println!(" Target Dist-Git Directory: {}", distgit_dir.display());
            println!("===========================================================");
            let updated = db::reconcile_all_package_macros(&mut conn, &distgit_dir)?;
            println!("✓ Successfully reconciled {} package record(s) in PostgreSQL.", updated);
            println!("===========================================================");
        }

        DbCommands::DumpSchema { .. } => unreachable!(),
    }

    Ok(())
}

/// Dispatches the `config` subcommand to view resolved configuration or initialize a new dbs.toml.
pub async fn handle_config(args: ConfigArgs, dbs_cfg: &DbsConfig, loaded_path: Option<&Path>) -> Result<()> {
    match args.command {
        ConfigCommands::Show => {
            println!("===========================================================");
            println!(" DBS Configuration Settings");
            if let Some(p) = loaded_path {
                println!(" Loaded from: {}", p.display());
            } else {
                println!(" Loaded from: Default settings (no file loaded)");
            }
            println!("===========================================================\n");
            let toml_str = toml::to_string_pretty(dbs_cfg)
                .map_err(|e| eyre!("Failed to serialize configuration to TOML: {}", e))?;
            println!("{}", toml_str);
        }
        ConfigCommands::Init { output, force } => {
            if output.exists() && !force {
                return Err(eyre!(
                    "Target configuration file '{}' already exists. Use '--force' to overwrite.",
                    output.display()
                ));
            }
            if let Some(parent) = output.parent()
                && !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            let sample = DbsConfig::sample_toml();
            fs::write(&output, sample)?;
            println!("✓ Successfully generated DBS configuration file at: {}", output.display());
            println!("Edit this file to customize your database, chroot, distgit, and distro options.");
        }
    }

    Ok(())
}

/// Discovers worker unique extension and working directory inside Mock buildroot for a package.
fn discover_package_chroot(
    pkg_name: &str,
    staging_dir: &Path,
    root_profile: &str,
) -> (Option<String>, Option<String>) {
    let mut detected_uniqueext = None;
    let mut detected_cwd = None;

    // 1. Scan staging dir for worker-<id>-<pkg>
    if staging_dir.exists()
        && let Ok(entries) = fs::read_dir(staging_dir) {
            let prefix = "worker-";
            let suffix = format!("-{}", pkg_name);
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with(prefix) && fname.ends_with(&suffix) {
                    let middle = &fname[prefix.len()..fname.len() - suffix.len()];
                    if let Ok(id) = middle.parse::<usize>() {
                        detected_uniqueext = Some(format!("w{}", id));
                        break;
                    }
                }
            }
        }

    // 2. Scan mock basedirs (/srv/dbs/mock, /var/lib/mock) for matching chroot and BUILD subdirectory
    let mock_search_bases = [Path::new("/srv/dbs/mock"), Path::new("/var/lib/mock")];
    for mock_base in &mock_search_bases {
        if mock_base.exists()
            && let Ok(entries) = fs::read_dir(mock_base) {
                let uext_tag = detected_uniqueext.as_deref().unwrap_or("");
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // Match root_profile and optional uniqueext
                    if name.contains(root_profile) && (uext_tag.is_empty() || name.ends_with(uext_tag)) {
                        let build_dir = entry.path().join("root/builddir/build/BUILD");
                        if build_dir.exists()
                            && let Ok(sub_entries) = fs::read_dir(&build_dir) {
                                for sub in sub_entries.flatten() {
                                    let sub_name = sub.file_name().to_string_lossy().to_string();
                                    if sub_name.starts_with(pkg_name) {
                                        detected_cwd = Some(format!("/builddir/build/BUILD/{}", sub_name));
                                        break;
                                    }
                                }
                                if detected_cwd.is_none() {
                                    detected_cwd = Some("/builddir/build/BUILD".to_string());
                                }
                            }
                        if detected_cwd.is_some() {
                            break;
                        }
                    }
                }
            }
        if detected_cwd.is_some() {
            break;
        }
    }

    (detected_uniqueext, detected_cwd)
}

/// Dispatches the `shell` subcommand to drop interactively into a Mock chroot.
pub async fn handle_shell(args: ShellArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let raw_target = &args.package;
    let pkg_name = Path::new(raw_target)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(raw_target)
        .trim_end_matches(".spec")
        .trim_end_matches(".src");

    let staging_dir = args.staging_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let mock_root = args.mock_root
        .or_else(|| dbs_cfg.distro.chroot.clone())
        .unwrap_or_else(|| dbs_cfg.chroot.profile.clone());
    let mock_config_dir = args.mock_config_dir.or_else(|| dbs_cfg.chroot.config_dir.clone());

    let resolved = match chroot::ChrootResolver::resolve(&mock_root, mock_config_dir.as_deref()) {
        Ok(r) => r,
        Err(e) => return Err(eyre!("Failed to resolve chroot configuration '{}': {}", mock_root, e)),
    };
    let root_profile = resolved.profile_name;
    let config_dir = resolved.config_dir;

    let (detected_uext, detected_cwd) = discover_package_chroot(pkg_name, &staging_dir, &root_profile);

    let effective_uniqueext = args.uniqueext.or(detected_uext);
    let effective_cwd = args.cwd.or(detected_cwd).unwrap_or_else(|| "/builddir/build/BUILD".to_string());

    println!("===========================================================");
    println!(" DBS Interactive Mock Chroot Shell");
    println!(" Target Package:    {}", pkg_name);
    println!(" Chroot Profile:    {}", root_profile);
    if let Some(ref uext) = effective_uniqueext {
        println!(" Unique Extension:  {}", uext);
    }
    println!(" Working Directory: {}", effective_cwd);
    if let Some(ref cfg) = config_dir {
        println!(" Mock Config Dir:   {}", cfg.display());
    }
    println!("===========================================================");
    println!("Spawning interactive shell inside Mock buildroot...");
    println!("Hint: type 'exit' or press Ctrl+D to return to host.\n");

    let mut cmd = std::process::Command::new("mock");
    cmd.arg(format!("-r={}", root_profile));
    if let Some(cfg) = &config_dir {
        cmd.arg(format!("--configdir={}", cfg.display()));
    }
    if let Some(uext) = &effective_uniqueext {
        cmd.arg(format!("--uniqueext={}", uext));
    }
    cmd.arg(format!("--cwd={}", effective_cwd));
    cmd.arg("--shell");
    if !args.cmd.is_empty() {
        cmd.args(&args.cmd);
    }

    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                let code = status.code().unwrap_or(-1);
                eprintln!("\nMock shell exited with code: {}", code);
            }
            Ok(())
        }
        Err(e) => Err(eyre!(
            "Failed to execute mock --shell: {}. Ensure 'mock' is installed and current user is in the 'mock' group.",
            e
        )),
    }
}

/// Dispatches the `retry` subcommand to clean previous worker staging artifacts and rebuild a package.
pub async fn handle_retry(args: RetryArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let staging_dir = args.output_dir.clone().unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let distgit_dest = &dbs_cfg.distgit.dest;
    let pkg_input = &args.package;

    // Resolve target path (spec or src.rpm)
    let target = match runner::resolve_package_target(pkg_input, distgit_dest) {
        Ok(path) => path,
        Err(_) => {
            let as_path = PathBuf::from(pkg_input);
            if as_path.exists() {
                as_path
            } else {
                return Err(eyre!(
                    "Could not resolve package '{}' in {} or current directory.",
                    pkg_input,
                    distgit_dest.display()
                ));
            }
        }
    };

    let pkg_stem = target.file_stem().and_then(|s| s.to_str()).unwrap_or("package");

    println!("===========================================================");
    println!(" DBS Package Build Retry");
    println!(" Target Package:    {}", pkg_stem);
    println!(" Resolved Spec:     {}", target.display());
    println!(" Staging Directory: {}", staging_dir.display());
    println!("===========================================================");

    // Clean up previous failed staging worker directory
    if staging_dir.exists()
        && let Ok(entries) = fs::read_dir(&staging_dir) {
            let prefix = "worker-";
            let suffix = format!("-{}", pkg_stem);
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with(prefix) && fname.ends_with(&suffix) {
                    let worker_path = entry.path();
                    println!("▶ Removing previous worker staging directory: {}", worker_path.display());
                    let _ = fs::remove_dir_all(&worker_path);
                }
            }
        }

    // Optionally clean Mock chroot
    let mock_root = args.mock_root.clone()
        .or_else(|| dbs_cfg.distro.chroot.clone())
        .unwrap_or_else(|| dbs_cfg.chroot.profile.clone());

    if args.clean_chroot {
        println!("▶ Cleaning Mock chroot profile: {}...", mock_root);
        let mut clean_cmd = std::process::Command::new("mock");
        clean_cmd.arg(format!("-r={}", mock_root));
        if let Some(cfg) = &args.mock_config_dir.as_ref().or(dbs_cfg.chroot.config_dir.as_ref()) {
            clean_cmd.arg(format!("--configdir={}", cfg.display()));
        }
        clean_cmd.arg("--clean");
        match clean_cmd.status() {
            Ok(s) if s.success() => println!("✓ Mock chroot cleaned successfully."),
            Ok(s) => eprintln!("Warning: mock --clean exited with status: {}", s),
            Err(e) => eprintln!("Warning: failed to execute mock --clean: {}", e),
        }
    }

    // Trigger rebuild with force=true and skip_existing=false
    let build_args = BuildArgs {
        runner: "mock".to_string(),
        mock_root: args.mock_root,
        mock_config_dir: args.mock_config_dir,
        output_dir: args.output_dir,
        concurrency: args.concurrency.or(Some(1)),
        smp: args.smp,
        dynamic_repo: true,
        chain: false,
        continue_on_error: false,
        packages: None,
        targets: vec![target],
        record_db: args.record_db,
        lookaside_dir: args.lookaside_dir,
        fetch_sources: true,
        skip_existing: false,
        force: true,
    };

    handle_build(build_args, dbs_cfg).await
}

/// Dispatches the `clean` subcommand to purge build artifacts, staging environments,
/// repository RPMs, and reset build database records.
pub async fn handle_clean(args: CleanArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    if args.package.is_none() && !args.all {
        return Err(eyre!(
            "Please specify a package name to clean (e.g. 'dbs clean gcc') or use '--all' to clean all build artifacts."
        ));
    }

    let staging_dir = args
        .staging_dir
        .clone()
        .unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let repo_dir = args
        .repo_dir
        .clone()
        .unwrap_or_else(|| dbs_cfg.distro.dest.clone());
    let target_arch = args
        .arch
        .clone()
        .unwrap_or_else(|| dbs_cfg.distro.arch.clone());
    let distgit_dest = &dbs_cfg.distgit.dest;

    // Resolve target package name / stem if specified
    let (pkg_name, target_spec) = match &args.package {
        Some(pkg_input) => {
            let resolved = runner::resolve_package_target(pkg_input, distgit_dest).ok();
            let stem = if let Some(ref path) = resolved {
                path.file_stem().and_then(|s| s.to_str()).unwrap_or(pkg_input).to_string()
            } else {
                let as_path = PathBuf::from(pkg_input);
                if as_path.exists() {
                    as_path.file_stem().and_then(|s| s.to_str()).unwrap_or(pkg_input).to_string()
                } else {
                    pkg_input.to_string()
                }
            };
            (Some(stem), resolved)
        }
        None => (None, None),
    };

    println!("===========================================================");
    println!(" DBS Clean Build Artifacts");
    if let Some(ref name) = pkg_name {
        println!(" Target Package:    {}", name);
        if let Some(ref spec) = target_spec {
            println!(" Resolved Target:   {}", spec.display());
        }
    } else {
        println!(" Target:            ALL PACKAGES (--all)");
    }
    println!(" Staging Directory: {}", staging_dir.display());
    println!(" Repository:        {}", repo_dir.display());
    println!(" Target Arch:       {}", target_arch);
    println!(" Staging Only:      {}", args.staging_only);
    println!(" Repo Only:         {}", args.repo_only);
    println!("===========================================================");

    let mut staging_removed_count = 0usize;
    let mut repo_rpms_removed_count = 0usize;
    let mut db_artifacts_removed_count = 0usize;
    let mut db_packages_reset_count = 0usize;

    // 1. Clean Staging Directories
    if !args.repo_only && staging_dir.exists() {
        if let Ok(entries) = fs::read_dir(&staging_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let fname = entry.file_name().to_string_lossy().to_string();

                let should_remove = match &pkg_name {
                    Some(name) => {
                        let worker_prefix = "worker-";
                        let worker_suffix = format!("-{}", name);
                        (fname.starts_with(worker_prefix) && fname.ends_with(&worker_suffix))
                            || fname == *name
                            || (fname.starts_with(&format!("{}-", name)) && fname.ends_with(".rpm"))
                    }
                    None => {
                        // --all mode: remove all worker directories and temporary build artifacts
                        fname.starts_with("worker-") || fname.ends_with(".rpm") || fname.ends_with(".log")
                    }
                };

                if should_remove {
                    if path.is_dir() {
                        println!("▶ Removing staging directory: {}", path.display());
                        match fs::remove_dir_all(&path) {
                            Ok(_) => staging_removed_count += 1,
                            Err(e) => eprintln!("Warning: failed to remove {}: {}", path.display(), e),
                        }
                    } else if path.is_file() {
                        println!("▶ Removing staging file: {}", path.display());
                        match fs::remove_file(&path) {
                            Ok(_) => staging_removed_count += 1,
                            Err(e) => eprintln!("Warning: failed to remove {}: {}", path.display(), e),
                        }
                    }
                }
            }
        }

        // Also check staging/localrepo or staging/rpms
        let extra_staging_dirs = [
            staging_dir.join("localrepo"),
            staging_dir.join("rpms").join(&target_arch),
            staging_dir.join("rpms").join("noarch"),
        ];
        for extra_dir in extra_staging_dirs {
            if extra_dir.exists()
                && let Ok(entries) = fs::read_dir(&extra_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            let fname = entry.file_name().to_string_lossy().to_string();
                            let matches = match &pkg_name {
                                Some(name) => {
                                    fname.starts_with(&format!("{}-", name)) && fname.ends_with(".rpm")
                                }
                                None => fname.ends_with(".rpm"),
                            };
                            if matches {
                                println!("▶ Removing staging artifact: {}", path.display());
                                match fs::remove_file(&path) {
                                    Ok(_) => staging_removed_count += 1,
                                    Err(e) => eprintln!("Warning: failed to remove {}: {}", path.display(), e),
                                }
                            }
                        }
                    }
                }
        }
    }

    // 2. Clean Published Repository RPMs
    if !args.staging_only && repo_dir.exists() {
        let mut search_dirs = vec![
            repo_dir.clone(),
            repo_dir.join(&target_arch),
            repo_dir.join("noarch"),
            repo_dir.join("SRPMS"),
            repo_dir.join("src"),
            repo_dir.join("Packages"),
        ];

        // Also discover any child directories inside repo_dir (excluding repodata)
        if let Ok(entries) = fs::read_dir(&repo_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && entry.file_name() != "repodata" && !search_dirs.contains(&path) {
                    search_dirs.push(path);
                }
            }
        }

        let mut rpms_to_delete = Vec::new();

        for dir in &search_dirs {
            if !dir.exists() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if !fname.ends_with(".rpm") {
                        continue;
                    }

                    let is_match = match &pkg_name {
                        Some(name) => {
                            // Check header with librpm for 100% precision
                            if let Ok(hdr) = librpm::package::PackageHeader::from_file(
                                &path,
                                Some(&librpm::verify::VerifyOptions::skip_verification()),
                            ) {
                                let hdr_name = hdr.name();
                                let mut matches = hdr_name == *name;
                                if !matches {
                                    // Check if source RPM matches package name (subpackages e.g. gcc-c++, libgcc)
                                    if let Some(librpm::TagData::Str(src_rpm)) = hdr.get(librpm::Tag::SOURCERPM)
                                        && src_rpm.starts_with(&format!("{}-", name)) {
                                            matches = true;
                                        }
                                }
                                matches
                            } else {
                                // Fallback filename prefix check
                                fname.starts_with(&format!("{}-", name))
                            }
                        }
                        None => true, // --all matches all RPMs in repository
                    };

                    if is_match && !rpms_to_delete.contains(&path) {
                        rpms_to_delete.push(path);
                    }
                }
            }
        }

        for rpm_path in rpms_to_delete {
            println!("▶ Removing repository RPM: {}", rpm_path.display());
            match fs::remove_file(&rpm_path) {
                Ok(_) => repo_rpms_removed_count += 1,
                Err(e) => eprintln!("Warning: failed to remove {}: {}", rpm_path.display(), e),
            }
        }

        // Re-index repository via createrepo_c if RPMs were deleted
        if repo_rpms_removed_count > 0 && !args.no_repo_update {
            println!("▶ Refreshing repository metadata via createrepo_c...");
            let update_dirs = [
                repo_dir.join(&target_arch),
                repo_dir.clone(),
            ];
            for u_dir in update_dirs {
                if u_dir.exists() {
                    runner::update_local_repo(&u_dir);
                }
            }
            println!("✓ Repository metadata refreshed.");
        }
    }

    // 3. Clean Mock Chroot if requested
    if args.clean_chroot {
        let mock_root = args
            .mock_root
            .clone()
            .or_else(|| dbs_cfg.distro.chroot.clone())
            .unwrap_or_else(|| dbs_cfg.chroot.profile.clone());
        println!("▶ Cleaning Mock chroot profile: {}...", mock_root);
        let mut clean_cmd = std::process::Command::new("mock");
        clean_cmd.arg(format!("-r={}", mock_root));
        if let Some(cfg) = &args.mock_config_dir.as_ref().or(dbs_cfg.chroot.config_dir.as_ref()) {
            clean_cmd.arg(format!("--configdir={}", cfg.display()));
        }
        clean_cmd.arg("--clean");
        match clean_cmd.status() {
            Ok(s) if s.success() => println!("✓ Mock chroot cleaned successfully."),
            Ok(s) => eprintln!("Warning: mock --clean exited with status: {}", s),
            Err(e) => eprintln!("Warning: failed to execute mock --clean: {}", e),
        }
    }

    // 4. Reset Database Records
    if !args.no_db {
        match db::establish_connection_with_url(dbs_cfg.database.url.as_deref()) {
            Ok(mut conn) => {
                match &pkg_name {
                    Some(name) => {
                        if let Ok(Some(pkg)) = db::find_package_by_name(&mut conn, name) {
                            // Check if package has recorded artifacts and clean any lingering on-disk paths
                            if let Ok(arts) = db::list_package_artifacts(&mut conn, pkg.id) {
                                for art in arts {
                                    let p = PathBuf::from(&art.rpm_path);
                                    if p.exists() && !args.staging_only {
                                        let _ = fs::remove_file(&p);
                                    }
                                }
                            }
                            if let Ok(count) = db::delete_package_artifacts_by_package_id(&mut conn, pkg.id) {
                                db_artifacts_removed_count += count;
                            }
                            if db::reset_package_build_status(&mut conn, pkg.id).is_ok() {
                                db_packages_reset_count += 1;
                            }
                            println!("✓ Reset database build status for '{}' to PENDING.", name);
                        } else {
                            println!("Note: package '{}' not found in database catalog.", name);
                        }
                    }
                    None => {
                        // --all mode: reset all packages and remove all artifacts
                        if let Ok(count) = db::delete_all_package_artifacts(&mut conn) {
                            db_artifacts_removed_count += count;
                        }
                        if let Ok(count) = db::reset_all_package_build_statuses(&mut conn) {
                            db_packages_reset_count += count;
                        }
                        println!("✓ Reset database build status for {} package(s) to PENDING.", db_packages_reset_count);
                    }
                }
            }
            Err(e) => {
                println!("Note: Database not reachable or not configured ({}). Skipped DB reset.", e);
            }
        }
    }

    println!("===========================================================");
    println!(" DBS Build Cleanup Complete");
    println!("  * Staging items removed:     {}", staging_removed_count);
    println!("  * Repository RPMs removed:   {}", repo_rpms_removed_count);
    if !args.no_db {
        println!("  * DB Artifacts deleted:      {}", db_artifacts_removed_count);
        println!("  * DB Packages reset:         {}", db_packages_reset_count);
    }
    println!("===========================================================");

    Ok(())
}

/// Dispatches the top-level `failed` subcommand.
pub async fn handle_failed(mut args: cli::pkg::ListPkgArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    args.failed = true;
    let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
    cli::pkg::list(&mut conn, &args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_clean_requires_target_or_all() {
        let (dbs_cfg, _) = DbsConfig::load(None).unwrap();
        let args = CleanArgs {
            package: None,
            all: false,
            staging_only: false,
            repo_only: false,
            clean_chroot: false,
            mock_root: None,
            mock_config_dir: None,
            staging_dir: None,
            repo_dir: None,
            arch: None,
            no_repo_update: true,
            no_db: true,
        };
        let res = handle_clean(args, &dbs_cfg).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Please specify a package name"));
    }

    #[tokio::test]
    async fn test_clean_package_staging_and_repo() {
        let (mut dbs_cfg, _) = DbsConfig::load(None).unwrap();
        let temp_staging = tempdir().unwrap();
        let temp_repo = tempdir().unwrap();

        // Create staging worker directories
        let worker_foo = temp_staging.path().join("worker-1-foopkg");
        let worker_bar = temp_staging.path().join("worker-2-barpkg");
        fs::create_dir_all(&worker_foo).unwrap();
        fs::create_dir_all(&worker_bar).unwrap();
        fs::write(worker_foo.join("build.log"), "foo log").unwrap();
        fs::write(worker_bar.join("build.log"), "bar log").unwrap();

        // Create repo files
        let repo_x86 = temp_repo.path().join("x86_64");
        fs::create_dir_all(&repo_x86).unwrap();
        let foo_rpm = repo_x86.join("foopkg-1.0-1.tcrs.x86_64.rpm");
        let bar_rpm = repo_x86.join("barpkg-2.0-1.tcrs.x86_64.rpm");
        fs::write(&foo_rpm, "dummy rpm content").unwrap();
        fs::write(&bar_rpm, "dummy rpm content").unwrap();

        dbs_cfg.distro.staging_dir = temp_staging.path().to_path_buf();
        dbs_cfg.distro.dest = temp_repo.path().to_path_buf();

        let args = CleanArgs {
            package: Some("foopkg".to_string()),
            all: false,
            staging_only: false,
            repo_only: false,
            clean_chroot: false,
            mock_root: None,
            mock_config_dir: None,
            staging_dir: Some(temp_staging.path().to_path_buf()),
            repo_dir: Some(temp_repo.path().to_path_buf()),
            arch: Some("x86_64".to_string()),
            no_repo_update: true,
            no_db: true,
        };

        let res = handle_clean(args, &dbs_cfg).await;
        assert!(res.is_ok());

        // foopkg staging and repo files should be removed
        assert!(!worker_foo.exists());
        assert!(!foo_rpm.exists());

        // barpkg staging and repo files should remain untouched
        assert!(worker_bar.exists());
        assert!(bar_rpm.exists());
    }

    #[tokio::test]
    async fn test_clean_all() {
        let (mut dbs_cfg, _) = DbsConfig::load(None).unwrap();
        let temp_staging = tempdir().unwrap();
        let temp_repo = tempdir().unwrap();

        let worker_foo = temp_staging.path().join("worker-1-foopkg");
        let worker_bar = temp_staging.path().join("worker-2-barpkg");
        fs::create_dir_all(&worker_foo).unwrap();
        fs::create_dir_all(&worker_bar).unwrap();

        let repo_x86 = temp_repo.path().join("x86_64");
        fs::create_dir_all(&repo_x86).unwrap();
        let foo_rpm = repo_x86.join("foopkg-1.0-1.tcrs.x86_64.rpm");
        let bar_rpm = repo_x86.join("barpkg-2.0-1.tcrs.x86_64.rpm");
        fs::write(&foo_rpm, "dummy").unwrap();
        fs::write(&bar_rpm, "dummy").unwrap();

        dbs_cfg.distro.staging_dir = temp_staging.path().to_path_buf();
        dbs_cfg.distro.dest = temp_repo.path().to_path_buf();

        let args = CleanArgs {
            package: None,
            all: true,
            staging_only: false,
            repo_only: false,
            clean_chroot: false,
            mock_root: None,
            mock_config_dir: None,
            staging_dir: Some(temp_staging.path().to_path_buf()),
            repo_dir: Some(temp_repo.path().to_path_buf()),
            arch: Some("x86_64".to_string()),
            no_repo_update: true,
            no_db: true,
        };

        let res = handle_clean(args, &dbs_cfg).await;
        assert!(res.is_ok());

        assert!(!worker_foo.exists());
        assert!(!worker_bar.exists());
        assert!(!foo_rpm.exists());
        assert!(!bar_rpm.exists());
    }
}


