//! PostgreSQL database client and operations using Diesel.
//!
//! Provides connection management, transaction handling, and CRUD operations for
//! operating systems, packages, capability dependencies, and build artifacts.

pub mod bootstrap;

use std::env;
use std::fs;
use std::path::PathBuf;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use eyre::{eyre, Result};

use crate::models::{
    NewOperatingSystem, NewPackage, NewPackageArtifact, NewPackageProvides, NewPackageRequires,
    OperatingSystem, Package, PackageArtifact,
};
use crate::schema::{operating_system, package, package_artifact, package_provides, package_requires};
use crate::types::BuildStatus;

/// Resolves `DATABASE_URL` from the environment or by reading config/env files.
pub fn get_database_url() -> Option<String> {
    if let Ok(url) = env::var("DATABASE_URL") {
        return Some(url);
    }

    let mut candidates = vec![
        PathBuf::from(".env"),
        PathBuf::from("../.env"),
        PathBuf::from("/etc/dbs/dbs.env"),
        PathBuf::from("/etc/dbs/dbs.conf"),
    ];

    if let Ok(home) = env::var("HOME") {
        candidates.push(PathBuf::from(home).join(".config/dbs/config.env"));
    }

    for path in &candidates {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((k, v)) = trimmed.split_once('=') {
                        if k.trim() == "DATABASE_URL" {
                            let val = v.trim().trim_matches('"').trim_matches('\'');
                            return Some(val.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Establishes a direct PostgreSQL connection with an optional custom connection URL.
pub fn establish_connection_with_url(custom_url: Option<&str>) -> Result<PgConnection> {
    let database_url = match custom_url {
        Some(url) => url.to_string(),
        None => match get_database_url() {
            Some(url) => url,
            None => {
                return Err(eyre!(
                    "DATABASE_URL environment variable is not set. Please set it, pass '--url <URL>', or configure /etc/dbs/dbs.env or .env"
                ))
            }
        },
    };

    match PgConnection::establish(&database_url) {
        Ok(conn) => Ok(conn),
        Err(e) => Err(eyre!(
            "Failed to connect to database at {}: {}",
            database_url,
            e
        )),
    }
}

/// Establishes a direct PostgreSQL connection using `DATABASE_URL`.
pub fn establish_connection() -> Result<PgConnection> {
    establish_connection_with_url(None)
}

/// Retrieves all registered operating system distribution records.
pub fn list_operating_systems(conn: &mut PgConnection) -> Result<Vec<OperatingSystem>> {
    match operating_system::table.load::<OperatingSystem>(conn) {
        Ok(records) => Ok(records),
        Err(e) => Err(eyre!("Failed to query operating_system table: {}", e)),
    }
}

/// Inserts a new operating system distribution record and returns the persisted entity.
pub fn insert_operating_system(
    conn: &mut PgConnection,
    new_os: &NewOperatingSystem,
) -> Result<OperatingSystem> {
    match diesel::insert_into(operating_system::table)
        .values(new_os)
        .get_result::<OperatingSystem>(conn)
    {
        Ok(record) => Ok(record),
        Err(e) => Err(eyre!("Failed to insert operating system '{}': {}", new_os.name, e)),
    }
}

/// Deletes an operating system distribution record by its primary key ID.
pub fn delete_operating_system(conn: &mut PgConnection, os_id: i32) -> Result<usize> {
    match diesel::delete(operating_system::table.filter(operating_system::id.eq(os_id))).execute(conn) {
        Ok(count) => Ok(count),
        Err(e) => Err(eyre!("Failed to delete operating system ID {}: {}", os_id, e)),
    }
}

/// Queries packages from the package catalog with an optional limit.
pub fn list_packages(conn: &mut PgConnection, limit: i64) -> Result<Vec<Package>> {
    match package::table.limit(limit).load::<Package>(conn) {
        Ok(records) => Ok(records),
        Err(e) => Err(eyre!("Failed to query package table: {}", e)),
    }
}

/// Looks up a package by its name.
pub fn find_package_by_name(conn: &mut PgConnection, pkg_name: &str) -> Result<Option<Package>> {
    match package::table
        .filter(package::name.eq(pkg_name))
        .first::<Package>(conn)
        .optional()
    {
        Ok(rec) => Ok(rec),
        Err(e) => Err(eyre!("Failed to search package by name '{}': {}", pkg_name, e)),
    }
}

/// Inserts a new package record and returns the persisted entity.
pub fn insert_package(conn: &mut PgConnection, new_pkg: &NewPackage) -> Result<Package> {
    match diesel::insert_into(package::table)
        .values(new_pkg)
        .get_result::<Package>(conn)
    {
        Ok(record) => Ok(record),
        Err(e) => Err(eyre!("Failed to insert package '{}': {}", new_pkg.name, e)),
    }
}

/// Deletes a package by its primary key ID.
pub fn delete_package(conn: &mut PgConnection, pkg_id: i32) -> Result<usize> {
    match diesel::delete(package::table.filter(package::id.eq(pkg_id))).execute(conn) {
        Ok(count) => Ok(count),
        Err(e) => Err(eyre!("Failed to delete package ID {}: {}", pkg_id, e)),
    }
}

/// Updates build metrics and status for a package record.
pub fn update_package_build_result(
    conn: &mut PgConnection,
    pkg_id: i32,
    status: BuildStatus,
    duration: f32,
    log_path: &str,
    error: Option<&str>,
) -> Result<()> {
    match diesel::update(package::table.filter(package::id.eq(pkg_id)))
        .set((
            package::build_status.eq(status),
            package::build_duration_seconds.eq(Some(duration)),
            package::build_log_path.eq(Some(log_path)),
            package::error_summary.eq(error),
        ))
        .execute(conn)
    {
        Ok(_) => Ok(()),
        Err(e) => Err(eyre!("Failed to update build status for package ID {}: {}", pkg_id, e)),
    }
}

/// Inserts provided capability records for a package.
pub fn insert_package_provides(
    conn: &mut PgConnection,
    provides: &[NewPackageProvides],
) -> Result<usize> {
    if provides.is_empty() {
        return Ok(0);
    }
    match diesel::insert_into(package_provides::table)
        .values(provides)
        .execute(conn)
    {
        Ok(count) => Ok(count),
        Err(e) => Err(eyre!("Failed to insert package provides: {}", e)),
    }
}

/// Inserts required capability records for a package.
pub fn insert_package_requires(
    conn: &mut PgConnection,
    requires: &[NewPackageRequires],
) -> Result<usize> {
    if requires.is_empty() {
        return Ok(0);
    }
    match diesel::insert_into(package_requires::table)
        .values(requires)
        .execute(conn)
    {
        Ok(count) => Ok(count),
        Err(e) => Err(eyre!("Failed to insert package requires: {}", e)),
    }
}

/// Inserts a built RPM package artifact record.
pub fn insert_package_artifact(
    conn: &mut PgConnection,
    artifact: &NewPackageArtifact,
) -> Result<PackageArtifact> {
    match diesel::insert_into(package_artifact::table)
        .values(artifact)
        .get_result::<PackageArtifact>(conn)
    {
        Ok(record) => Ok(record),
        Err(e) => Err(eyre!(
            "Failed to insert package artifact '{}': {}",
            artifact.rpm_filename,
            e
        )),
    }
}

/// Convenience function to persist a synchronized dist-git package with its capabilities.
pub fn record_synced_package(
    conn: &mut PgConnection,
    meta: &crate::distgit::spec::SpecMetadata,
    distro: &str,
    branch: &str,
    commit: &str,
) -> Result<Package> {
    if let Ok(Some(existing)) = find_package_by_name(conn, &meta.name) {
        let _ = delete_package(conn, existing.id);
    }

    let spec_file_str = meta.spec_path.as_ref().map(|p| p.display().to_string());

    let new_pkg = NewPackage {
        name: meta.name.clone(),
        epoch: meta.epoch,
        version: meta.version.clone(),
        release: meta.release.clone(),
        architecture: 1, // x86_64
        package_size: "0 MB".to_string(),
        file_size_bytes: 0,
        source: format!("{}.src.rpm", meta.name),
        repository: distro.to_string(),
        summary: meta.summary.clone(),
        url: meta.url.clone(),
        license: meta.license.clone(),
        description: format!("{} from dist-git {}", meta.name, distro),
        in_repo: Some(false),
        created: Some(false),
        vulnerable: Some(false),
        build_status: BuildStatus::PENDING,
        build_duration_seconds: None,
        build_log_path: None,
        error_summary: None,
        worker_id: None,
        sourcerpm: Some(format!("{}-{}-{}.src.rpm", meta.name, meta.version, meta.release)),
        dist_git_url: None,
        dist_git_branch: Some(branch.to_string()),
        dist_git_commit: Some(commit.to_string()),
        spec_file: spec_file_str,
    };

    let pkg = insert_package(conn, &new_pkg)?;

    // Insert provides
    let mut provides = Vec::new();
    provides.push(NewPackageProvides {
        id_package: pkg.id,
        name: meta.name.clone(),
        flags: None,
        version: Some(meta.version.clone()),
    });
    for prov in &meta.provides {
        provides.push(NewPackageProvides {
            id_package: pkg.id,
            name: prov.clone(),
            flags: None,
            version: None,
        });
    }
    let _ = insert_package_provides(conn, &provides);

    // Insert requires
    let mut requires = Vec::new();
    for br in &meta.build_requires {
        requires.push(NewPackageRequires {
            id_package: pkg.id,
            name: br.clone(),
            flags: None,
            version: None,
            is_build_require: true,
        });
    }
    for req in &meta.requires {
        requires.push(NewPackageRequires {
            id_package: pkg.id,
            name: req.clone(),
            flags: None,
            version: None,
            is_build_require: false,
        });
    }
    let _ = insert_package_requires(conn, &requires);

    Ok(pkg)
}

/// Records the build execution results and output artifacts for a package.
pub fn record_build_result(
    conn: &mut PgConnection,
    pkg_name: &str,
    output: &crate::runner::BuildOutput,
) -> Result<()> {
    let pkg_id = match find_package_by_name(conn, pkg_name)? {
        Some(p) => p.id,
        None => {
            let new_pkg = NewPackage {
                name: pkg_name.to_string(),
                epoch: 0,
                version: "0.0.0".to_string(),
                release: "1".to_string(),
                architecture: 1,
                package_size: "0 MB".to_string(),
                file_size_bytes: 0,
                source: format!("{}.src.rpm", pkg_name),
                repository: "build".to_string(),
                summary: format!("Built package {}", pkg_name),
                url: "https://localhost".to_string(),
                license: "Unknown".to_string(),
                description: format!("Built package {}", pkg_name),
                in_repo: Some(true),
                created: Some(output.success),
                vulnerable: Some(false),
                build_status: if output.success { BuildStatus::SUCCESS } else { BuildStatus::FAILED },
                build_duration_seconds: Some(output.duration_seconds as f32),
                build_log_path: Some(output.log_path.display().to_string()),
                error_summary: output.error_summary.clone(),
                worker_id: None,
                sourcerpm: None,
                dist_git_url: None,
                dist_git_branch: None,
                dist_git_commit: None,
                spec_file: None,
            };
            let rec = insert_package(conn, &new_pkg)?;
            rec.id
        }
    };

    let status = if output.success { BuildStatus::SUCCESS } else { BuildStatus::FAILED };
    update_package_build_result(
        conn,
        pkg_id,
        status,
        output.duration_seconds as f32,
        &output.log_path.display().to_string(),
        output.error_summary.as_deref(),
    )?;

    for art in &output.artifacts {
        let filename = art.file_name().and_then(|f| f.to_str()).unwrap_or("unknown.rpm");
        let is_src = filename.ends_with(".src.rpm");
        let size = fs::metadata(art).map(|m| m.len() as i64).unwrap_or(0);
        let new_art = NewPackageArtifact {
            id_package: Some(pkg_id),
            rpm_filename: filename.to_string(),
            rpm_path: art.display().to_string(),
            arch: if is_src { "src".to_string() } else { "x86_64".to_string() },
            is_source: is_src,
            file_size_bytes: size,
        };
        let _ = insert_package_artifact(conn, &new_art);
    }

    Ok(())
}

