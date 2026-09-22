use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum PkgCommands {
    /// Add a new package to the pipeline
    Add(AddPkgArgs),
    /// List all available packages
    List,
    /// Update an existing package
    Update(UpdatePkgArgs),
    /// Delete a package by its ID
    Delete {
        /// The ID of the package to delete
        #[arg(long)]
        id: i32,
    },
    /// Build packages for a distribution (placeholder)
    Build(BuildPkgArgs),
}

#[derive(Args, Debug)]
pub struct AddPkgArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub version: String,
    #[arg(long)]
    pub release: String,
    #[arg(long)]
    pub architecture: String,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub license: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
}

#[derive(Args, Debug)]
pub struct UpdatePkgArgs {
    #[arg(long)]
    pub id: i32,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub version: Option<String>,
    #[arg(long)]
    pub release: Option<String>,
    #[arg(long)]
    pub architecture: Option<String>,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub license: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
}

#[derive(Args, Debug)]
pub struct BuildPkgArgs {
    /// The name of the package to build
    #[arg(long, conflicts_with = "id")]
    pub name: Option<String>,
    /// The ID of the package to build
    #[arg(long)]
    pub id: Option<i32>,
}

use diesel::PgConnection;
use eyre::Result;
use crate::db;
use crate::models::NewPackage;
use crate::types::BuildStatus;

/// Lists packages stored in the database catalog.
pub fn list(conn: &mut PgConnection, limit: i64) -> Result<()> {
    let packages = db::list_packages(conn, limit)?;
    if packages.is_empty() {
        println!("No packages found in database catalog.");
        return Ok(());
    }

    println!("===========================================================");
    println!(" DBS Package Catalog (Database - top {} records)", packages.len());
    println!("===========================================================");
    for p in packages {
        let dur_str = p.build_duration_seconds.map(|d| format!("{:.1}s", d)).unwrap_or_else(|| "-".to_string());
        println!("  [{}] {}-{}-{} ({})", p.id, p.name, p.version, p.release, p.package_size);
        println!("      Status:    {:?} (duration: {})", p.build_status, dur_str);
        println!("      Summary:   {}", p.summary);
        if let Some(log) = &p.build_log_path {
            println!("      Build Log: {}", log);
        }
        if let Some(spec) = &p.spec_file {
            println!("      Spec File: {}", spec);
        }
        println!();
    }
    println!("===========================================================");
    Ok(())
}

/// Adds a new package record to the database catalog.
pub fn add(conn: &mut PgConnection, args: &AddPkgArgs) -> Result<()> {
    let arch_id = match args.architecture.to_lowercase().as_str() {
        "x86_64" | "amd64" => 1,
        "aarch64" | "arm64" => 2,
        _ => 1,
    };

    let new_pkg = NewPackage {
        name: args.name.clone(),
        epoch: 0,
        version: args.version.clone(),
        release: args.release.clone(),
        architecture: arch_id,
        package_size: "0 MB".to_string(),
        file_size_bytes: 0,
        source: args.source.clone().unwrap_or_default(),
        repository: "default".to_string(),
        summary: args.summary.clone().unwrap_or_else(|| format!("{} package", args.name)),
        url: "https://localhost".to_string(),
        license: args.license.clone().unwrap_or_else(|| "GPL-2.0-or-later".to_string()),
        description: args.description.clone().unwrap_or_else(|| format!("{} package", args.name)),
        in_repo: Some(false),
        created: Some(false),
        vulnerable: Some(false),
        build_status: BuildStatus::PENDING,
        build_duration_seconds: None,
        build_log_path: None,
        error_summary: None,
        worker_id: None,
        sourcerpm: None,
        dist_git_url: None,
        dist_git_branch: None,
        dist_git_commit: None,
        spec_file: None,
    };

    let created = db::insert_package(conn, &new_pkg)?;
    println!("✓ Successfully added package '{}' with ID {}", created.name, created.id);
    Ok(())
}

/// Deletes a package by ID from the database catalog.
pub fn delete(conn: &mut PgConnection, id: i32) -> Result<()> {
    let deleted = db::delete_package(conn, id)?;
    if deleted > 0 {
        println!("✓ Deleted package ID {}", id);
    } else {
        println!("Package ID {} not found.", id);
    }
    Ok(())
}

