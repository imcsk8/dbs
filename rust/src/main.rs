//! Main entry point for Distribution Build System (DBS).
//!
//! Provides CLI initialization and command execution for exploring, cloning,
//! inspecting, and building packages across any Linux distribution using dist-git.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use clap::Parser;
use eyre::{eyre, Result};

pub mod cli;
pub mod dag;
pub mod db;
pub mod distgit;
pub mod models;
pub mod runner;
pub mod schema;
pub mod types;

use cli::{BuildArgs, Cli, Commands, DagArgs, DistgitArgs, DistgitCommands, ExploreArgs, OsArgs, PkgArgs};
use dag::DependencyGraph;
use distgit::provider::DistroConfig;
use distgit::spec::{parse_spec_file, SpecMetadata};
use distgit::DistGitClient;
use runner::{BuildOutput, BuildRunner, MockRunner, RpmbuildRunner};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Explore(args) => handle_explore(args).await?,
        Commands::Distgit(args) => handle_distgit(args).await?,
        Commands::Build(args) => handle_build(args).await?,
        Commands::Os(args) => handle_os(args).await?,
        Commands::Pkg(args) => handle_pkg(args).await?,
        Commands::Dag(args) => handle_dag(args).await?,
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
async fn handle_distgit(args: DistgitArgs) -> Result<()> {
    match args.command {
        DistgitCommands::Clone { distro, dest, rename_as, rename_spec, new_origin, packages } => {
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

        DistgitCommands::Sync { distro, dest, concurrency, sources, search, limit, record_db } => {
            let config = DistroConfig::from_preset(&distro)
                .ok_or_else(|| eyre!("Unknown distribution preset '{}'", distro))?;
            let client = Arc::new(DistGitClient::new(config));

            println!("Discovering packages in {} matching query '{:?}'...", distro, search);
            let projects = client.explore(search.as_deref(), Some(limit)).await?;
            if projects.is_empty() {
                println!("No packages found to synchronize.");
                return Ok(());
            }

            let mut db_conn = if record_db {
                match db::establish_connection() {
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
                            println!("  Downloading lookaside sources for {}...", status.package_name);
                            match client.download_lookaside_sources(&status.local_path, &status.package_name).await {
                                Ok(files) => {
                                    for f in files {
                                        println!("    * Staged: {}", f.display());
                                    }
                                }
                                Err(e) => eprintln!("    ✗ Lookaside download failed: {}", e),
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
async fn handle_build(args: BuildArgs) -> Result<()> {
    println!("===========================================================");
    println!(" DBS Package Build Orchestrator");
    println!(" Runner:       {}", args.runner);
    println!(" Staging:      {}", args.output_dir.display());
    println!(" Targets:      {}", args.targets.len());
    println!(" Concurrency:  {} workers", args.concurrency);
    println!(" Dynamic Repo: {}", args.dynamic_repo);
    if args.record_db {
        println!(" Record to DB: enabled");
    }
    println!("===========================================================");

    let mut db_conn = if args.record_db {
        match db::establish_connection() {
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

    match args.runner.to_lowercase().as_str() {
        "mock" => {
            let chroot = args.mock_root.unwrap_or_else(|| "fedora-rawhide-x86_64".to_string());
            let mut runner = MockRunner::new(chroot);
            if let Some(cfg) = args.mock_config_dir {
                runner = runner.with_config_dir(cfg);
            }

            if args.chain {
                println!("Executing sequential Mock chain build for {} package(s)...", args.targets.len());
                let out = runner.build_chain(&args.targets, &args.output_dir, args.continue_on_error)?;
                print_build_output(&out);
                if let Some(conn) = &mut db_conn {
                    for target in &args.targets {
                        let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                        let _ = db::record_build_result(conn, name, &out);
                    }
                }
            } else if args.targets.len() == 1 || args.concurrency == 1 {
                for target in &args.targets {
                    println!("\nBuilding {} in Mock...", target.display());
                    let out = runner.build(target, &args.output_dir)?;
                    print_build_output(&out);
                    if let Some(conn) = &mut db_conn {
                        let name = target.file_stem().and_then(|s| s.to_str()).unwrap_or("pkg");
                        let _ = db::record_build_result(conn, name, &out);
                    }
                }
            } else {
                println!("Launching {} parallel Mock workers with dynamic local repo feedback...", args.concurrency);
                let runner = Arc::new(runner);
                let results = runner
                    .build_parallel(args.targets.clone(), args.output_dir.clone(), args.concurrency, args.dynamic_repo)
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
                let out = runner.build(target, &args.output_dir)?;
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
async fn handle_os(args: OsArgs) -> Result<()> {
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

            match db::establish_connection() {
                Ok(mut conn) => {
                    let _ = cli::os_actions::list(&mut conn);
                }
                Err(e) => {
                    println!("Note: Database not connected ({})", e);
                }
            }
        }
        cli::os::OsCommands::Add(add_args) => {
            let mut conn = db::establish_connection()?;
            cli::os_actions::add(&mut conn, &add_args)?;
        }
        cli::os::OsCommands::Delete { id } => {
            let mut conn = db::establish_connection()?;
            cli::os_actions::delete(&mut conn, id)?;
        }
        cli::os::OsCommands::AddPackage(pkg_args) => {
            let mut conn = db::establish_connection()?;
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
async fn handle_pkg(args: PkgArgs) -> Result<()> {
    match args.command {
        cli::pkg::PkgCommands::List => {
            let mut conn = db::establish_connection()?;
            cli::pkg::list(&mut conn, 50)?;
        }
        cli::pkg::PkgCommands::Add(add_args) => {
            let mut conn = db::establish_connection()?;
            cli::pkg::add(&mut conn, &add_args)?;
        }
        cli::pkg::PkgCommands::Delete { id } => {
            let mut conn = db::establish_connection()?;
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
async fn handle_dag(args: DagArgs) -> Result<()> {
    println!("===========================================================");
    println!(" DBS Topological Dependency Graph (DAG) Resolution");
    println!(" Repository/Spec Root: {}", args.path.display());
    println!("===========================================================");

    let mut graph = DependencyGraph::new();
    let loaded = match graph.load_from_dir(&args.path) {
        Ok(count) => count,
        Err(e) => return Err(eyre!("Failed loading packages from {}: {}", args.path.display(), e)),
    };

    if loaded == 0 {
        println!("No package .spec files found in {}", args.path.display());
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
        report.push_str(&format!("* **Source Directory:** `{}`\n", args.path.display()));
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

    if args.build {
        println!("\nExecuting layered build orchestration with runner '{}'...", args.runner);
        let chroot = args.mock_root.unwrap_or_else(|| "fedora-rawhide-x86_64".to_string());
        let mut mock_runner = MockRunner::new(chroot);
        if let Some(cfg) = args.mock_config_dir {
            mock_runner = mock_runner.with_config_dir(cfg);
        }
        let runner_arc = Arc::new(mock_runner);

        let mut db_conn = if args.record_db {
            match db::establish_connection() {
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
                    .build_parallel(targets.clone(), args.output_dir.clone(), args.concurrency, args.dynamic_repo)
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

