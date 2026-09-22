use clap::{Args, Subcommand};
use crate::types::OsType;


#[derive(Subcommand, Debug)]
pub enum OsCommands {
    /// Add a new OS distribution to the build pipeline
    Add(AddOsArgs),
    /// List all available OS distributions
    List,
    /// Update an existing OS distribution
    Update(UpdateOsArgs),
    /// Delete an OS distribution by its ID
    Delete {
        /// The ID of the OS to delete
        #[arg(long)]
        id: i32,
    },
    /// Associate a package with an OS distribution
    AddPackage(AddPackageToOsArgs),
    /// Build an operating system (placeholder)
    Build {
        /// The ID of the OS to build
        #[arg(long)]
        id: i32,
    },
}

#[derive(Args, Debug)]
pub struct AddOsArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub version: String,
    #[arg(long, value_enum, default_value_t = OsType::VERSIONED)]
    pub system_type: OsType,
    #[arg(long)]
    pub release: String,
    #[arg(long)]
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

#[derive(Args, Debug)]
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

#[derive(Args, Debug)]
pub struct AddPackageToOsArgs {
    /// ID of the Operating System
    #[arg(long)]
    pub os_id: i32,
    /// ID of the Package to add
    #[arg(long)]
    pub pkg_id: i32,
}


