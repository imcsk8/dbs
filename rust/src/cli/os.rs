use clap::{Args, Subcommand};
use crate::types::OsType;


#[derive(Subcommand, Debug, Clone)]
pub enum OsCommands {
    /// Add a new distribution to the database (deprecated: use 'dbs distro add')
    Add(AddDistroArgs),
    /// List all available distributions (deprecated: use 'dbs distro list')
    List,
    /// Update an existing distribution
    Update(UpdateOsArgs),
    /// Delete a distribution by its ID (deprecated: use 'dbs distro delete')
    Delete {
        /// The ID of the distribution to delete
        #[arg(long)]
        id: i32,
    },
    /// Associate a package with a distribution
    AddPackage(AddPackageToOsArgs),
    /// Build a distribution (deprecated: use 'dbs distro build')
    Build {
        /// The ID of the distribution to build
        #[arg(long)]
        id: i32,
    },
}

#[derive(Args, Debug, Clone)]
pub struct AddDistroArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub version: String,
    #[arg(long, value_enum, default_value_t = OsType::VERSIONED)]
    pub system_type: OsType,
    #[arg(long)]
    pub release: String,
    #[arg(long, default_value = "x86_64")]
    pub architecture: String,
    #[arg(long)]
    pub distro_tag: String,
    #[arg(long)]
    pub manager: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub license: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
}

/// Backward compatibility alias for AddDistroArgs.
pub type AddOsArgs = AddDistroArgs;

#[derive(Args, Debug, Clone)]
pub struct UpdateOsArgs {
    #[arg(long)]
    pub id: i32,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub version: Option<String>,
    #[arg(long, value_enum)]
    pub system_type: Option<OsType>,
    #[arg(long)]
    pub release: Option<String>,
    #[arg(long)]
    pub architecture: Option<String>,
    #[arg(long)]
    pub distro_tag: Option<String>,
    #[arg(long)]
    pub manager: Option<String>,
    #[arg(long)]
    pub summary: Option<String>,
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub license: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AddPackageToOsArgs {
    /// ID of the Operating System
    #[arg(long)]
    pub os_id: i32,
    /// ID of the Package to add
    #[arg(long)]
    pub pkg_id: i32,
}


