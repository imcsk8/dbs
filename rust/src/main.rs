//! Main entry point for Distribution Build System (DBS).
//!
//! Provides CLI initialization and command execution for exploring, cloning,
//! inspecting, and building packages across any Linux distribution using dist-git.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use clap::Parser;
use eyre::{eyre, Result};

pub mod chroot;
pub mod cli;
pub mod config;
pub mod dag;
pub mod db;
pub mod distgit;
pub mod distro;
pub mod lookaside;
pub mod models;
pub mod runner;
pub mod schema;
pub mod types;

use cli::{BuildArgs, ChrootArgs, ChrootCommands, Cli, Commands, ConfigArgs, ConfigCommands, DagArgs, DbArgs, DbCommands, DistgitArgs, DistgitCommands, DistroArgs, DistroCommands, ExploreArgs, LookasideArgs, LookasideCommands, OsArgs, PkgArgs};
use config::DbsConfig;
use dag::DependencyGraph;
use distgit::provider::DistroConfig;
use distgit::spec::{parse_spec_file, SpecMetadata};
use distgit::DistGitClient;
use distro::{generate_nginx_config, get_distro_status, init_distro, publish_distro, run_http_server, DistroInitOptions, DistroPublishOptions};
use lookaside::LookasideManager;
use runner::{BuildOutput, BuildRunner, MockRunner, RpmbuildRunner};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (dbs_cfg, loaded_path) = DbsConfig::load(cli.config.as_deref())?;

    if cli.verbose {
        if let Some(ref p) = loaded_path {
            println!("[CONFIG] Loaded configuration from: {}", p.display());
        } else {
            println!("[CONFIG] Using default configuration (no config file found)");
        }
    }

    match cli.command {
        Commands::Explore(args) => handle_explore(args).await?,
        Commands::Distgit(args) => handle_distgit(args, &dbs_cfg).await?,
        Commands::Build(args) => handle_build(args, &dbs_cfg).await?,
        Commands::Os(args) => handle_os(args, &dbs_cfg).await?,
        Commands::Pkg(args) => handle_pkg(args, &dbs_cfg).await?,
        Commands::Dag(args) => handle_dag(args, &dbs_cfg).await?,
        Commands::Chroot(args) => handle_chroot(args, &dbs_cfg).await?,
        Commands::Lookaside(args) => handle_lookaside(args, &dbs_cfg).await?,
        Commands::Distro(args) => handle_distro(args, &dbs_cfg).await?,
        Commands::Db(args) => handle_db(args, &dbs_cfg).await?,
        Commands::Config(args) => handle_config(args, &dbs_cfg, loaded_path.as_deref()).await?,
    }

    Ok(())
}

/// Dispatches the `explore` subcommand to discover remote packages in dist-git.
async fn handle_explore(args: ExploreArgs) -> Result<()> {
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

    println!("===========================================================");
    println!(" DBS Remote Package Explorer");
    println!(" Target:    {} ({})", config.name, config.version);
    println!(" API Type:  {}", config.api_type);
    if let Some(pattern) = &args.search {
        println!(" Query:     '{}'", pattern);
    }
    println!("===========================================================");

    let client = DistGitClient::new(config);
    let projects = client.explore(args.search.as_deref(), Some(args.limit)).await?;

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

/// Dispatches the `distgit` subcommand for cloning, pulling, syncing, and inspecting.
async fn handle_distgit(args: DistgitArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        DistgitCommands::Clone { distro, dest, rename_as, rename_spec, new_origin, packages } => {
            let distro = distro.unwrap_or_else(|| dbs_cfg.distgit.distro.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let config = DistroConfig::from_preset(&distro)
                .ok_or_else(|| eyre!("Unknown distribution preset '{}'", distro))?;
            let client = DistGitClient::new(config);

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

                match client.clone_or_pull_as(source_pkg, target_name, &dest, rename_spec, new_origin.as_deref()) {
                    Ok(status) => {
                        println!("✓ Cloned {} -> {} (branch: {}, commit: {:.8})", source_pkg, status.package_name, status.branch, status.commit_hash);
                        println!("  Spec: {} ({}-{})", status.spec_path.display(), status.spec_meta.version, status.spec_meta.release);
                        if let Some(origin_template) = &new_origin {
                            let final_origin = if origin_template.contains("{package}") {
                                origin_template.replace("{package}", target_name)
                            } else if origin_template.ends_with('/') {
                                format!("{}{}.git", origin_template, target_name)
                            } else {
                                origin_template.clone()
                            };
                            println!("  Git Remote 'origin':   {}", final_origin);
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
                        if entry.path().join(".git").exists() {
                            if let Some(name) = entry.file_name().to_str() {
                                found.push(name.to_string());
                            }
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

        DistgitCommands::Sync { distro, dest, concurrency, sources, search, limit, record_db, lookaside_dir } => {
            let distro = distro.unwrap_or_else(|| dbs_cfg.distgit.distro.clone());
            let dest = dest.unwrap_or_else(|| dbs_cfg.distgit.dest.clone());
            let concurrency = concurrency.unwrap_or(dbs_cfg.distgit.concurrency);
            let lookaside_dir = lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
            let record_db = record_db || dbs_cfg.database.record_db;

            let config = DistroConfig::from_preset(&distro)
                .ok_or_else(|| eyre!("Unknown distribution preset '{}'", distro))?;
            let client = Arc::new(DistGitClient::new(config));
            let lookaside_mgr = LookasideManager::resolve_default(lookaside_dir.as_deref());

            println!("Discovering packages in {} matching query '{:?}'...", distro, search);
            let projects = client.explore(search.as_deref(), Some(limit)).await?;
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
            println!("Synchronizing {} repository(ies) with {} workers into {}...", pkg_names.len(), concurrency, dest.display());

            let results = client.clone().sync_batch(pkg_names, dest.clone(), concurrency).await;
            let mut success_count = 0;

            for res in results {
                match res {
                    Ok(status) => {
                        success_count += 1;
                        println!("✓ {} -> {} (commit: {:.8})", status.package_name, status.spec_meta.name, status.commit_hash);

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
                    Err(e) => eprintln!("✗ Sync error: {}", e),
                }
            }

            println!("Completed: {}/{} repositories synchronized successfully.", success_count, limit);
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
async fn handle_build(args: BuildArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    let output_dir = args.output_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
    let concurrency = args.concurrency.unwrap_or(dbs_cfg.distgit.concurrency);
    let mock_root = args.mock_root.or_else(|| Some(dbs_cfg.chroot.profile.clone()));
    let lookaside_dir = args.lookaside_dir.or_else(|| Some(dbs_cfg.distgit.lookaside_dir.clone()));
    let record_db = args.record_db || dbs_cfg.database.record_db;

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
            if let Some(l_dir) = lookaside_dir {
                runner = runner.with_lookaside_dir(l_dir);
            }
            println!(" Chroot Profile: {}", runner.root_name);
            if let Some(cfg) = &runner.config_dir {
                println!(" Chroot Config:  {}", cfg.display());
            }
            if let Some(ld) = &runner.lookaside_dir {
                println!(" Lookaside Dir:  {}", ld.display());
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
                            if let Some(conn) = &mut db_conn {
                                if let Some(target) = args.targets.get(idx) {
                                    let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                    let _ = db::record_build_result(conn, name, &out);
                                }
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
                println!("\nBuilding {} on host with rpmbuild...", target.display());
                let out = runner.build(target, &output_dir)?;
                print_build_output(&out);
                if let Some(conn) = &mut db_conn {
                    let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
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

/// Dispatches the `os` subcommand.
async fn handle_os(args: OsArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        cli::os::OsCommands::List => {
            println!("===========================================================");
            println!(" DBS Supported Operating System Presets (Static)");
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
        cli::os::OsCommands::Add(add_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::add(&mut conn, &add_args)?;
        }
        cli::os::OsCommands::Delete { id } => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::delete(&mut conn, id)?;
        }
        cli::os::OsCommands::AddPackage(pkg_args) => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::os_actions::add_package(&mut conn, &pkg_args)?;
        }
        cli::os::OsCommands::Update(_) => {
            println!("OS update action is not yet implemented.");
        }
        cli::os::OsCommands::Build { id } => {
            println!("OS build orchestration for OS ID {}", id);
        }
    }
    Ok(())
}

/// Dispatches the `pkg` subcommand.
async fn handle_pkg(args: PkgArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        cli::pkg::PkgCommands::List => {
            let mut conn = db::establish_connection_with_url(dbs_cfg.database.url.as_deref())?;
            cli::pkg::list(&mut conn, 50)?;
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
}

/// Dispatches the `dag` subcommand to compute topological build layers and optionally execute builds.
async fn handle_dag(args: DagArgs, dbs_cfg: &DbsConfig) -> Result<()> {
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
        let lookaside_mgr = LookasideManager::resolve_default(lookaside_dir.as_deref());
        println!("\n▶ Synchronizing source archives into lookaside cache ({}) for packages in {}...", lookaside_mgr.root.display(), path.display());
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
        if let Some(l_dir) = lookaside_dir {
            mock_runner = mock_runner.with_lookaside_dir(l_dir);
        }
        println!(" Chroot Profile: {}", mock_runner.root_name);
        if let Some(cfg) = &mock_runner.config_dir {
            println!(" Chroot Config:  {}", cfg.display());
        }
        if let Some(ld) = &mock_runner.lookaside_dir {
            println!(" Lookaside Dir:  {}", ld.display());
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
                if let Some(meta) = graph.packages.get(pkg) {
                    if let Some(spec) = &meta.spec_path {
                        targets.push(spec.clone());
                    }
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
                            if let Some(conn) = &mut db_conn {
                                if let Some(target) = targets.get(idx) {
                                    let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                    let _ = db::record_build_result(conn, name, &out);
                                }
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
async fn handle_chroot(args: ChrootArgs, dbs_cfg: &DbsConfig) -> Result<()> {
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

        ChrootCommands::Init { name, arch, dest } => {
            println!("Initializing new Mock chroot configuration '{}' (arch: {}) in {}...", name, arch, dest.display());
            let created_path = match chroot::ChrootResolver::init(&name, &arch, &dest) {
                Ok(p) => p,
                Err(e) => return Err(eyre!("Failed to initialize chroot '{}': {}", name, e)),
            };
            println!("✓ Successfully initialized chroot configuration: {}", created_path.display());
            println!("  Template created at: {}/templates/{}.tpl", dest.display(), name);
            println!("You can customize this configuration and verify it using 'dbs chroot check {}'", name);
        }
    }

    Ok(())
}

/// Dispatches the `lookaside` subcommand for managing source archive storage and synchronization.
async fn handle_lookaside(args: LookasideArgs, dbs_cfg: &DbsConfig) -> Result<()> {
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
async fn handle_distro(args: DistroArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.action {
        DistroCommands::Init {
            name,
            arch,
            channel,
            releasever,
            dist,
            dest,
            mock_dir,
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
            staging_dir,
            sign_key,
            record_db,
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
            let staging_dir = staging_dir.unwrap_or_else(|| dbs_cfg.distro.staging_dir.clone());
            let sign_key = sign_key.or_else(|| dbs_cfg.distro.sign_key.clone());
            let record_db = record_db || dbs_cfg.database.record_db;

            println!("===========================================================");
            println!(" DBS Distribution Build Orchestration");
            println!(" Target Distribution: {}", name);
            println!(" Spec/Dist-Git Root:  {}", path.display());
            println!(" Staging Directory:   {}", staging_dir.display());
            println!(" Concurrency:         {}", concurrency);
            println!("===========================================================");

            // 1. Ensure lookaside sources are synchronized
            let lookaside_mgr = LookasideManager::resolve_default(lookaside_dir.as_deref());
            println!("\n▶ Synchronizing source archives into lookaside cache ({})...", lookaside_mgr.root.display());
            match lookaside_mgr.sync_dir(&path, concurrency).await {
                Ok(rep) => {
                    println!("✓ Lookaside sources: {} cached, {} downloaded, {} failed (total: {})",
                        rep.already_cached, rep.downloaded, rep.failed, rep.total_sources_found);
                }
                Err(e) => eprintln!("Warning: Lookaside sync issue: {}", e),
            }

            // 2. Resolve dependency graph and compute layers
            println!("\n▶ Resolving package dependency graph (DAG)...");
            let mut graph = DependencyGraph::new();
            let loaded = graph.load_from_dir(&path)?;
            if loaded == 0 {
                return Err(eyre!("No package .spec files found in {}", path.display()));
            }

            let layers = graph.compute_layers()?;
            println!("✓ Loaded {} packages across {} compilation layers.", graph.packages.len(), layers.len());

            // 3. Setup runner
            let mut mock_runner = MockRunner::resolve(&mock_root, mock_config_dir)?;
            if let Some(ld) = lookaside_dir.clone() {
                mock_runner = mock_runner.with_lookaside_dir(ld);
            }
            let runner_arc = Arc::new(mock_runner);

            let mut db_conn = if record_db {
                db::establish_connection_with_url(dbs_cfg.database.url.as_deref()).ok()
            } else {
                None
            };

            // 4. Execute layered builds with dynamic repo
            for layer in &layers {
                println!("\n===========================================================");
                println!(" Starting Layer {} ({} package(s))", layer.layer_index, layer.packages.len());
                println!("===========================================================");

                let mut targets = Vec::new();
                for pkg in &layer.packages {
                    if let Some(meta) = graph.packages.get(pkg) {
                        if let Some(spec) = &meta.spec_path {
                            targets.push(spec.clone());
                        }
                    }
                }

                if !targets.is_empty() {
                    let results = runner_arc
                        .clone()
                        .build_parallel(targets.clone(), staging_dir.clone(), concurrency, true)
                        .await;

                    for (idx, res) in results.into_iter().enumerate() {
                        match res {
                            Ok(out) => {
                                print_build_output(&out);
                                if let Some(conn) = &mut db_conn {
                                    if let Some(target) = targets.get(idx) {
                                        let pkg_name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                                        let _ = db::record_build_result(conn, pkg_name, &out);
                                    }
                                }
                            }
                            Err(e) => eprintln!("✗ Worker build error: {}", e),
                        }
                    }
                }
            }

            // 5. Automatically publish build artifacts to distribution repo
            println!("\n▶ Publishing build artifacts into distribution repository...");
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
async fn handle_db(args: DbArgs, dbs_cfg: &DbsConfig) -> Result<()> {
    match args.command {
        DbCommands::DumpSchema { down } => {
            let sql = db::bootstrap::dump_schema(down);
            print!("{}", sql);
            return Ok(());
        }
        _ => {}
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

        DbCommands::DumpSchema { .. } => unreachable!(),
    }

    Ok(())
}

/// Dispatches the `config` subcommand to view resolved configuration or initialize a new dbs.toml.
async fn handle_config(args: ConfigArgs, dbs_cfg: &DbsConfig, loaded_path: Option<&Path>) -> Result<()> {
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
            if let Some(parent) = output.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }
            let sample = DbsConfig::sample_toml();
            fs::write(&output, sample)?;
            println!("✓ Successfully generated DBS configuration file at: {}", output.display());
            println!("Edit this file to customize your database, chroot, distgit, and distro options.");
        }
    }

    Ok(())
}


