pub mod os;
pub mod os_actions;
pub mod pkg;

use std::path::PathBuf;
use clap::{Args, Parser, Subcommand};
use crate::cli::os::OsCommands;
use crate::cli::pkg::PkgCommands;

/// Distribution Build System (DBS) command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "dbs",
    author = "Iván Chavero <ichavero@chavero.com.mx>, TacOS Team",
    version = "0.1.0",
    about = "Distribution Build System: Build, manage, and synchronize Linux distributions from dist-git",
    long_about = "DBS provides a unified platform for discovering, synchronizing, inspecting, \
                  and building Linux distributions (Fedora, CentOS Stream, TacOS, etc.) \
                  from dist-git repositories with pluggable build runners (Mock, rpmbuild, containers)."
)]
pub struct Cli {
    /// Verbose logging output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Explore and search packages across Fedora Rawhide, CentOS Stream, and other dist-git platforms.
    Explore(ExploreArgs),

    /// Manage, clone, pull, and synchronize dist-git repositories.
    Distgit(DistgitArgs),

    /// Compile packages using Mock, rpmbuild, or container runners.
    Build(BuildArgs),

    /// Manage Operating System distribution presets and database records.
    Os(OsArgs),

    /// Query and manage package records in the database.
    Pkg(PkgArgs),

    /// Analyze dependencies across dist-git packages and compute topological build order (DAG).
    Dag(DagArgs),
}

/// Arguments for the `explore` subcommand.
#[derive(Args, Debug)]
pub struct ExploreArgs {
    /// Distribution name or preset to search (e.g. fedora-rawhide, centos-stream-10, centos-stream-9, tacos).
    #[arg(short, long, default_value = "fedora-rawhide")]
    pub distro: String,

    /// Search pattern or package name query.
    #[arg(short, long)]
    pub search: Option<String>,

    /// Maximum number of packages to return.
    #[arg(short, long, default_value = "25")]
    pub limit: usize,
}

/// Arguments for the `distgit` subcommand.
#[derive(Args, Debug)]
pub struct DistgitArgs {
    #[command(subcommand)]
    pub command: DistgitCommands,
}

#[derive(Subcommand, Debug)]
pub enum DistgitCommands {
    /// Clone specific dist-git package repositories.
    Clone {
        /// Distribution preset to clone from.
        #[arg(short, long, default_value = "fedora-rawhide")]
        distro: String,

        /// Destination directory for cloned repositories.
        #[arg(short = 'o', long, default_value = "data/distgit")]
        dest: PathBuf,

        /// Custom target name for cloned repository (e.g. --as tacos-zstd).
        #[arg(name = "as", long = "as", visible_alias = "rename")]
        rename_as: Option<String>,

        /// Rename the .spec file to match the target package name (e.g. zstd.spec -> tacos-zstd.spec).
        #[arg(long)]
        rename_spec: bool,

        /// Set upstream git remote and configure origin for new repository (e.g. Codeberg/Forgejo).
        #[arg(long)]
        new_origin: Option<String>,

        /// Package names to clone (supports 'upstream_pkg' or 'upstream_pkg:target_name').
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// Pull / sync updates for existing cloned dist-git repositories.
    Pull {
        /// Destination directory containing cloned repositories.
        #[arg(short = 'o', long, default_value = "data/distgit")]
        dest: PathBuf,

        /// Specific package names to pull (or all if omitted).
        packages: Vec<String>,
    },

    /// Synchronize a batch of dist-git repositories concurrently with worker pool.
    Sync {
        /// Distribution preset to sync.
        #[arg(short, long, default_value = "fedora-rawhide")]
        distro: String,

        /// Destination directory for repositories.
        #[arg(short = 'o', long, default_value = "data/distgit")]
        dest: PathBuf,

        /// Number of parallel worker tasks.
        #[arg(short = 'j', long, default_value = "4")]
        concurrency: usize,

        /// Also download referenced upstream source archives from lookaside cache.
        #[arg(long)]
        sources: bool,

        /// Specific package names to sync (if omitted, discovers packages matching search).
        #[arg(short, long)]
        search: Option<String>,

        /// Limit on packages when discovering via search.
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Record synchronized packages and dependency capabilities into PostgreSQL database.
        #[arg(long)]
        record_db: bool,
    },

    /// Inspect a `.spec` file or local dist-git repository and display parsed metadata.
    Inspect {
        /// Path to `.spec` file or dist-git repository directory.
        path: PathBuf,
    },
}

/// Arguments for the `build` subcommand.
#[derive(Args, Debug)]
pub struct BuildArgs {
    /// Build runner engine to use: mock, rpmbuild.
    #[arg(long, default_value = "mock")]
    pub runner: String,

    /// Mock chroot configuration profile name (e.g. fedora-rawhide-x86_64, centos-stream-10-x86_64).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Directory for build logs and staged RPM artifacts.
    #[arg(short = 'o', long, default_value = "staging")]
    pub output_dir: PathBuf,

    /// Number of concurrent Mock worker processes.
    #[arg(short = 'j', long, default_value = "4")]
    pub concurrency: usize,

    /// Dynamically feed staged RPMs back to Mock workers via a local repository.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub dynamic_repo: bool,

    /// Execute sequential chain build for multiple interdependent packages (mock --chain).
    #[arg(long)]
    pub chain: bool,

    /// Continue building remaining packages even if one fails in chain mode.
    #[arg(short = 'c', long)]
    pub continue_on_error: bool,

    /// Spec file or SRPM file paths to build.
    #[arg(required = true)]
    pub targets: Vec<PathBuf>,

    /// Record build metrics, status, and output artifacts into PostgreSQL database.
    #[arg(long)]
    pub record_db: bool,
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

/// Arguments for the `dag` subcommand.
#[derive(Args, Debug)]
pub struct DagArgs {
    /// Directory containing .spec files or cloned dist-git repositories.
    #[arg(short = 'i', long, default_value = "data/distgit")]
    pub path: PathBuf,

    /// Output path to save the generated dependency graph report in markdown.
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Automatically trigger Mock/rpmbuild compilation in topological layer order.
    #[arg(long)]
    pub build: bool,

    /// Build runner engine when --build is enabled: mock, rpmbuild.
    #[arg(long, default_value = "mock")]
    pub runner: String,

    /// Mock chroot configuration profile name (e.g. fedora-rawhide-x86_64, centos-stream-10-x86_64).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Directory for build logs and staged RPM artifacts.
    #[arg(short = 'o', long, default_value = "staging")]
    pub output_dir: PathBuf,

    /// Number of concurrent workers per layer.
    #[arg(short = 'j', long, default_value = "4")]
    pub concurrency: usize,

    /// Dynamically feed staged RPMs back to Mock workers via local repository.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub dynamic_repo: bool,

    /// Record build metrics, status, and output artifacts into PostgreSQL database.
    #[arg(long)]
    pub record_db: bool,
}
