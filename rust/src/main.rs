//! Main entry point for Distribution Build System (DBS).
//!
//! Provides CLI initialization and command execution for exploring, cloning,
//! inspecting, and building packages across any Linux distribution using dist-git.

use clap::Parser;
use eyre::Result;
use log::debug;

pub mod chroot;
pub mod cli;
pub mod comps;
pub mod config;
pub mod dag;
pub mod db;
pub mod distgit;
pub mod distro;
pub mod handlers;
pub mod lookaside;
pub mod models;
pub mod runner;
pub mod schema;
pub mod tui;
pub mod types;

use cli::{Cli, Commands};
use config::DbsConfig;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    debug!("Logger initialized");
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
        Commands::Explore(args) => handlers::handle_explore(args, &dbs_cfg).await?,
        Commands::Browse(mut args) => {
            args.interactive = true;
            handlers::handle_explore(args, &dbs_cfg).await?
        }
        Commands::Monitor(args) => handlers::handle_monitor(args, &dbs_cfg).await?,
        Commands::Distgit(args) => handlers::handle_distgit(args, &dbs_cfg).await?,
        Commands::Build(args) => handlers::handle_build(args, &dbs_cfg).await?,
        Commands::Os(args) => handlers::handle_os(args, &dbs_cfg).await?,
        Commands::Pkg(args) => handlers::handle_pkg(args, &dbs_cfg).await?,
        Commands::Dag(args) => handlers::handle_dag(args, &dbs_cfg).await?,
        Commands::Chroot(args) => handlers::handle_chroot(args, &dbs_cfg).await?,
        Commands::Lookaside(args) => handlers::handle_lookaside(args, &dbs_cfg).await?,
        Commands::Distro(args) => handlers::handle_distro(args, &dbs_cfg).await?,
        Commands::Db(args) => handlers::handle_db(args, &dbs_cfg).await?,
        Commands::Shell(args) => handlers::handle_shell(args, &dbs_cfg).await?,
        Commands::Retry(args) => handlers::handle_retry(args, &dbs_cfg).await?,
        Commands::Clean(args) => handlers::handle_clean(args, &dbs_cfg).await?,
        Commands::Failed(args) => handlers::handle_failed(args, &dbs_cfg).await?,
        Commands::Config(args) => handlers::handle_config(args, &dbs_cfg, loaded_path.as_deref()).await?,
        Commands::Comps(args) => handlers::handle_comps(args, &dbs_cfg).await?,
    }

    Ok(())
}
