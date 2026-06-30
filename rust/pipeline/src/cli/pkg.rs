use clap::{Args, Parser, Subcommand, ValueEnum};

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
