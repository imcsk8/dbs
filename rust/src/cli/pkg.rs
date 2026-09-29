use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub enum PkgCommands {
    /// Add a new package to the pipeline
    Add(AddPkgArgs),
    /// List packages in the database catalog with optional status filtering
    List(ListPkgArgs),
    /// List failed packages with diagnostic error summaries (shorthand for `dbs pkg list --failed`)
    Failed(ListPkgArgs),
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
    /// Clean build artifacts (staging, repository RPMs, and database build state) for a package
    Clean(CleanPkgArgs),
}

/// Arguments for listing packages from the database catalog.
#[derive(Args, Debug, Clone, Default)]
pub struct ListPkgArgs {
    /// Filter packages by build status (failed, success, building, pending, skipped)
    #[arg(short = 's', long)]
    pub status: Option<String>,

    /// Show only failed packages (shorthand for --status failed)
    #[arg(short = 'f', long)]
    pub failed: bool,

    /// Maximum number of packages to return (defaults to 50, use 0 for unlimited)
    #[arg(short = 'l', long, default_value = "50")]
    pub limit: i64,
}

#[derive(Args, Debug, Clone)]
pub struct CleanPkgArgs {
    /// The name of the package to clean
    #[arg(long, conflicts_with = "id")]
    pub name: Option<String>,
    /// The ID of the package to clean
    #[arg(long)]
    pub id: Option<i32>,
    /// Clean all staging directories and build artifacts across all packages
    #[arg(short = 'a', long)]
    pub all: bool,
    /// Clean only worker staging directories and build logs
    #[arg(long)]
    pub staging_only: bool,
    /// Clean only published RPMs from the repository
    #[arg(long)]
    pub repo_only: bool,
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

/// Resolves the optional `BuildStatus` filter from `ListPkgArgs`.
pub fn resolve_filter_status(args: &ListPkgArgs) -> Result<Option<BuildStatus>> {
    if args.failed {
        Ok(Some(BuildStatus::FAILED))
    } else if let Some(ref st) = args.status {
        match st.to_uppercase().as_str() {
            "FAILED" | "FAIL" => Ok(Some(BuildStatus::FAILED)),
            "SUCCESS" | "OK" => Ok(Some(BuildStatus::SUCCESS)),
            "BUILDING" | "BUILD" => Ok(Some(BuildStatus::BUILDING)),
            "PENDING" => Ok(Some(BuildStatus::PENDING)),
            "SKIPPED" => Ok(Some(BuildStatus::SKIPPED)),
            other => Err(eyre::eyre!(
                "Unknown build status '{}'. Valid statuses: failed, success, building, pending, skipped",
                other
            )),
        }
    } else {
        Ok(None)
    }
}

/// Lists packages stored in the database catalog with optional status filtering.
pub fn list(conn: &mut PgConnection, args: &ListPkgArgs) -> Result<()> {
    let effective_status = resolve_filter_status(args)?;

    let packages = match db::list_packages_with_status(conn, effective_status, args.limit) {
        Ok(pkgs) => pkgs,
        Err(e) => return Err(eyre::eyre!("Failed to retrieve packages: {}", e)),
    };

    if packages.is_empty() {
        if let Some(st) = effective_status {
            println!("No packages with status {:?} found in database catalog.", st);
        } else {
            println!("No packages found in database catalog.");
        }
        return Ok(());
    }

    let title = match effective_status {
        Some(BuildStatus::FAILED) => format!("DBS Failed Packages (Database - {} records)", packages.len()),
        Some(st) => format!("DBS Packages [Status: {:?}] (Database - {} records)", st, packages.len()),
        None => format!("DBS Package Catalog (Database - top {} records)", packages.len()),
    };

    println!("===========================================================");
    println!(" {}", title);
    println!("===========================================================");
    for p in packages {
        let dur_str = p
            .build_duration_seconds
            .map(|d| format!("{:.1}s", d))
            .unwrap_or_else(|| "-".to_string());
        println!("  [{}] {}-{}-{} ({})", p.id, p.name, p.version, p.release, p.package_size);
        println!("      Name:      {} ", p.name);
        println!("      Status:    {:?} (duration: {})", p.build_status, dur_str);
        println!("      Summary:   {}", p.summary);
        if let Some(err) = &p.error_summary {
            println!("      Failure:   {}", err);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_filter_status_default() {
        let args = ListPkgArgs::default();
        let res = resolve_filter_status(&args).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_resolve_filter_status_failed_flag() {
        let args = ListPkgArgs {
            failed: true,
            status: None,
            limit: 50,
        };
        let res = resolve_filter_status(&args).unwrap();
        assert_eq!(res, Some(BuildStatus::FAILED));
    }

    #[test]
    fn test_resolve_filter_status_by_name() {
        let cases = [
            ("failed", BuildStatus::FAILED),
            ("FAIL", BuildStatus::FAILED),
            ("success", BuildStatus::SUCCESS),
            ("OK", BuildStatus::SUCCESS),
            ("building", BuildStatus::BUILDING),
            ("pending", BuildStatus::PENDING),
            ("skipped", BuildStatus::SKIPPED),
        ];

        for (input, expected) in cases {
            let args = ListPkgArgs {
                failed: false,
                status: Some(input.to_string()),
                limit: 50,
            };
            let res = resolve_filter_status(&args).unwrap();
            assert_eq!(res, Some(expected), "Testing status '{}'", input);
        }
    }

    #[test]
    fn test_resolve_filter_status_invalid() {
        let args = ListPkgArgs {
            failed: false,
            status: Some("unknown_status".to_string()),
            limit: 50,
        };
        let res = resolve_filter_status(&args);
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Unknown build status"));
    }
}

