pub mod os;
pub mod os_actions;
pub mod pkg;

use clap::{Args, Parser, Subcommand, ValueEnum};
use crate::cli::os::OsCommands;
use crate::cli::pkg::PkgCommands; 

/// Command line parser
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage Operating Systems
    Os(OsArgs),
    /// Manage Packages
    Pkg(PkgArgs),
}

#[derive(Args, Debug)]
pub struct OsArgs {
    #[command(subcommand)]
    pub command: OsCommands,
}

#[derive(Args, Debug)]
pub struct PkgArgs {
    #[command(subcommand)]
    pub command: PkgCommands,
}
