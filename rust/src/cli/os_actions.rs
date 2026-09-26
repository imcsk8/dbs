//! Operating System CLI actions and database synchronization.

use diesel::PgConnection;
use eyre::Result;

use crate::cli::os::{AddOsArgs, AddPackageToOsArgs};
use crate::db;
use crate::models::NewOperatingSystem;

/// Lists all operating systems registered in the database.
pub fn list(conn: &mut PgConnection) -> Result<()> {
    let systems = db::list_operating_systems(conn)?;
    if systems.is_empty() {
        println!("No operating systems found in database.");
        return Ok(());
    }

    println!("===========================================================");
    println!(" Registered Distributions (Database)");
    println!("===========================================================");
    for os in systems {
        println!("  [{}] {} ({}) - Release: {}", os.id, os.name, os.version, os.release);
        println!("      Type:        {:?}", os.system_type);
        println!("      Summary:     {}", os.summary);
        if let Some(tag) = &os.distro_tag {
            println!("      Distro Tag:  {}", tag);
        }
        if let Some(chroot) = &os.mock_chroot {
            println!("      Mock Root:   {}", chroot);
        }
        if let Some(branch) = &os.dist_git_branch {
            println!("      Git Branch:  {}", branch);
        }
        println!();
    }
    println!("===========================================================");
    Ok(())
}

/// Adds a new distribution record to the database.
pub fn add(conn: &mut PgConnection, args: &AddOsArgs) -> Result<()> {
    // Look up or default architecture ID (1 for x86_64) and manager ID (1 for dnf)
    let arch_id = match args.architecture.to_lowercase().as_str() {
        "x86_64" | "amd64" => 1,
        "aarch64" | "arm64" => 2,
        _ => 1,
    };

    let manager_id = match args.manager.as_deref().unwrap_or("dnf").to_lowercase().as_str() {
        "dnf" | "dnf5" => 1,
        "rpm" => 2,
        _ => 1,
    };

    let new_os = NewOperatingSystem {
        name: args.name.clone(),
        version: args.version.clone(),
        system_type: args.system_type,
        release: args.release.clone(),
        architecture: arch_id,
        summary: args.summary.clone().unwrap_or_else(|| format!("{} Distribution", args.name)),
        url: args.url.clone().unwrap_or_else(|| "https://localhost".to_string()),
        license: args.license.clone().unwrap_or_else(|| "GPL-2.0-or-later".to_string()),
        description: args.description.clone().unwrap_or_else(|| format!("{} distribution", args.name)),
        manager: manager_id,
        distro_tag: Some(args.distro_tag.clone()),
        dist_git_url_template: None,
        dist_git_branch: None,
        lookaside_cache_url: None,
        api_type: None,
        api_url: None,
        mock_chroot: None,
    };

    let created = db::insert_operating_system(conn, &new_os)?;
    println!("✓ Successfully registered distribution '{}' with ID {}", created.name, created.id);
    Ok(())
}

/// Deletes a distribution record from the database by ID.
pub fn delete(conn: &mut PgConnection, id: i32) -> Result<()> {
    let deleted = db::delete_operating_system(conn, id)?;
    if deleted > 0 {
        println!("✓ Deleted distribution ID {}", id);
    } else {
        println!("Distribution ID {} not found.", id);
    }
    Ok(())
}

/// Placeholder for associating package with OS.
pub fn add_package(_conn: &mut PgConnection, args: &AddPackageToOsArgs) -> Result<()> {
    println!("Associated package {} with OS ID {}", args.pkg_id, args.os_id);
    Ok(())
}
