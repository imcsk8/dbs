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
    /// Path to TOML configuration file (e.g. dbs.toml).
    #[arg(short = 'c', long = "config", global = true)]
    pub config: Option<PathBuf>,

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

    /// Manage distributions, presets, database records, and repository lifecycle.
    Distro(DistroArgs),

    /// Query and manage package records in the database.
    Pkg(PkgArgs),

    /// Analyze dependencies across dist-git packages and compute topological build order (DAG).
    Dag(DagArgs),

    /// Manage, list, inspect, and validate Mock chroot build configurations.
    #[command(alias = "mock")]
    Chroot(ChrootArgs),

    /// Manage, maintain, synchronize, and upload source archives into the dist-git lookaside cache.
    Lookaside(LookasideArgs),

    /// [Deprecated: use 'distro'] Manage distribution presets and database records.
    #[command(hide = true)]
    Os(OsArgs),


    /// Manage database lifecycle, schema bootstrap, status, reset, and SQL schema dumps.
    Db(DbArgs),

    /// Launch interactive terminal UI package browser (shorthand for `dbs explore -i`).
    Browse(ExploreArgs),

    /// Interactive live TUI dashboard monitoring distribution, lookaside cache, mock chroots, and database.
    #[command(alias = "top", alias = "dashboard")]
    Monitor(MonitorArgs),

    /// Drop into an interactive Mock chroot shell at a package's build environment.
    Shell(ShellArgs),

    /// Clean staging artifacts and retry the build for a failed package.
    Retry(RetryArgs),

    /// Clean staging artifacts, remove built RPMs from repository, and reset build database records.
    #[command(alias = "delete-build", alias = "purge")]
    Clean(CleanArgs),

    /// List failed packages with diagnostic error summaries (shorthand for `dbs pkg list --failed`).
    #[command(alias = "list-failed")]
    Failed(crate::cli::pkg::ListPkgArgs),

    /// Manage, view, and initialize DBS TOML configuration files.
    Config(ConfigArgs),

    /// Inspect, list, and resolve RPM comps groups and environments (e.g. @workstation-product-environment, @core).
    Comps(CompsArgs),
}

/// Arguments for the `monitor` / `top` subcommand.
#[derive(Args, Debug, Clone)]
pub struct MonitorArgs {
    /// Distribution name or repository to monitor (defaults to config or tacos).
    #[arg(short, long)]
    pub distro: Option<String>,
}

/// Arguments for the `explore` subcommand.
#[derive(Args, Debug, Clone)]
pub struct ExploreArgs {
    /// Distribution name or preset to search (e.g. fedora-rawhide, centos-stream-10, centos-stream-9, tacos).
    #[arg(short, long, default_value = "fedora-rawhide")]
    pub distro: String,

    /// Search pattern or package name query.
    #[arg(short, long)]
    pub search: Option<String>,

    /// Maximum number of packages to return (ignored if --all is specified).
    #[arg(short, long, default_value = "25")]
    pub limit: usize,

    /// Discover all available packages across all pages without limit.
    #[arg(long)]
    pub all: bool,

    /// Launch interactive terminal UI (TUI) package browser.
    #[arg(short = 'i', long)]
    pub interactive: bool,

    /// Optional API token / key for authenticating with dist-git forge APIs.
    #[arg(long, env = "DBS_DISTGIT_API_KEY")]
    pub api_key: Option<String>,
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
        /// Distribution preset to clone from (defaults to [distgit].distro in dbs.toml).
        #[arg(short, long)]
        distro: Option<String>,

        /// Destination directory for cloned repositories (defaults to [distgit].dest in dbs.toml).
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,

        /// Custom target name for cloned repository (e.g. --as tacos-zstd).
        #[arg(name = "as", long = "as", visible_alias = "rename")]
        rename_as: Option<String>,

        /// Rename the .spec file to match the target package name (e.g. zstd.spec -> tacos-zstd.spec).
        #[arg(long)]
        rename_spec: bool,

        /// Set upstream git remote and configure origin for new repository (e.g. Codeberg/Forgejo).
        #[arg(long)]
        new_origin: Option<String>,

        /// Base URL of new origin git remote (e.g. https://codeberg.org/imcsk8/tacos).
        #[arg(long)]
        new_top_origin: Option<String>,

        /// Optional API token / key for authenticating with dist-git forge APIs.
        #[arg(long, env = "DBS_DISTGIT_API_KEY")]
        api_key: Option<String>,

        /// Package names to clone (supports 'upstream_pkg' or 'upstream_pkg:target_name').
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// Pull / sync updates for existing cloned dist-git repositories.
    Pull {
        /// Destination directory containing cloned repositories (defaults to [distgit].dest in dbs.toml).
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,

        /// Specific package names to pull (or all if omitted).
        packages: Vec<String>,
    },

    /// Synchronize a batch of dist-git repositories concurrently with worker pool.
    Sync {
        /// Distribution preset to sync (defaults to [distgit].distro in dbs.toml).
        #[arg(short, long)]
        distro: Option<String>,

        /// Destination directory for repositories (defaults to [distgit].dest in dbs.toml).
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,

        /// Number of parallel worker tasks (defaults to [distgit].concurrency in dbs.toml).
        #[arg(short = 'j', long)]
        concurrency: Option<usize>,

        /// Also download referenced upstream source archives from lookaside cache.
        #[arg(long)]
        sources: bool,

        /// Specific package names to sync (if omitted, discovers packages matching search).
        #[arg(short, long)]
        search: Option<String>,

        /// Limit on packages when discovering via search (ignored if --all is specified).
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Synchronize all available upstream repositories across all pages without limit.
        #[arg(long)]
        all: bool,

        /// Record synchronized packages and dependency capabilities into PostgreSQL database.
        #[arg(long)]
        record_db: bool,

        /// Path to the local lookaside cache directory for instant BTRFS CoW staging.
        #[arg(long, env = "DBS_LOOKASIDE_DIR")]
        lookaside_dir: Option<PathBuf>,

        /// Base URL of new origin git remote to automatically configure for each synced repo (e.g. https://codeberg.org/imcsk8/tacos).
        #[arg(long)]
        new_top_origin: Option<String>,

        /// Optional API token / key for authenticating with dist-git forge APIs.
        #[arg(long, env = "DBS_DISTGIT_API_KEY")]
        api_key: Option<String>,
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

    /// Mock chroot configuration profile name or direct path to a .cfg file (e.g. tacos-rolling-x86_64, /path/to/profile.cfg).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Directory for build logs and staged RPM artifacts (defaults to [distro].staging_dir in dbs.toml).
    #[arg(short = 'o', long)]
    pub output_dir: Option<PathBuf>,

    /// Number of concurrent Mock worker processes (defaults to [distgit].concurrency in dbs.toml).
    #[arg(short = 'j', long)]
    pub concurrency: Option<usize>,

    /// Max SMP concurrency CPUs for package compilation inside Mock (%_smp_mflags / %_smp_build_ncpus).
    #[arg(long, env = "DBS_SMP_CPUS")]
    pub smp: Option<usize>,

    /// Dynamically feed staged RPMs back to Mock workers via a local repository.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub dynamic_repo: bool,

    /// Execute sequential chain build for multiple interdependent packages (mock --chain).
    #[arg(long)]
    pub chain: bool,

    /// Continue building remaining packages even if one fails in chain mode.
    #[arg(short = 'C', long)]
    pub continue_on_error: bool,

    /// Path to a file containing package names or spec paths to build (one per line).
    #[arg(short = 'p', long)]
    pub packages: Option<PathBuf>,

    /// Spec file or SRPM file paths to build.
    #[arg(required_unless_present = "packages")]
    pub targets: Vec<PathBuf>,

    /// Record build metrics, status, and output artifacts into PostgreSQL database.
    #[arg(long)]
    pub record_db: bool,

    /// Path to local lookaside cache for instant BTRFS CoW staging of source tarballs (defaults to [distgit].lookaside_dir).
    #[arg(long, env = "DBS_LOOKASIDE_DIR")]
    pub lookaside_dir: Option<PathBuf>,

    /// Automatically download and cache missing source archives into the lookaside cache.
    #[arg(long, visible_alias = "sources", default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub fetch_sources: bool,

    /// Skip building packages if the exact version/release RPM is already built (defaults to true).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub skip_existing: bool,

    /// Force rebuild of package(s) even if the same version has already been built.
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Disable running test suites in Mock and rpmbuild (%check phase).
    #[arg(long)]
    pub nocheck: bool,
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
    /// Directory containing .spec files or cloned dist-git repositories (defaults to [distgit].dest in dbs.toml).
    #[arg(short = 'i', long)]
    pub path: Option<PathBuf>,

    /// Output path to save the generated dependency graph report in markdown.
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Automatically trigger Mock/rpmbuild compilation in topological layer order.
    #[arg(long)]
    pub build: bool,

    /// Build runner engine when --build is enabled: mock, rpmbuild.
    #[arg(long, default_value = "mock")]
    pub runner: String,

    /// Mock chroot configuration profile name or direct path to a .cfg file (defaults to [chroot].profile in dbs.toml).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Directory for build logs and staged RPM artifacts (defaults to [distro].staging_dir in dbs.toml).
    #[arg(short = 'o', long)]
    pub output_dir: Option<PathBuf>,

    /// Number of concurrent workers per layer (defaults to [distgit].concurrency in dbs.toml).
    #[arg(short = 'j', long)]
    pub concurrency: Option<usize>,

    /// Dynamically feed staged RPMs back to Mock workers via local repository.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub dynamic_repo: bool,

    /// Record build metrics, status, and output artifacts into PostgreSQL database.
    #[arg(long)]
    pub record_db: bool,

    /// Path to local lookaside cache for instant BTRFS CoW staging of source tarballs.
    #[arg(long, env = "DBS_LOOKASIDE_DIR")]
    pub lookaside_dir: Option<PathBuf>,

    /// Automatically download and cache missing source archives into the lookaside cache.
    #[arg(long, visible_alias = "sources", default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub fetch_sources: bool,

    /// Automatically detect and break circular dependencies via base chroot fallback.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub break_cycles: bool,

    /// Disable running test suites in Mock and rpmbuild (%check phase).
    #[arg(long)]
    pub nocheck: bool,
}

/// Arguments for the `chroot` subcommand.
#[derive(Args, Debug)]
pub struct ChrootArgs {
    #[command(subcommand)]
    pub command: ChrootCommands,
}

#[derive(Subcommand, Debug)]
pub enum ChrootCommands {
    /// List available Mock chroot configuration profiles across search paths.
    List {
        /// Optional custom directory containing mock configs.
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,

        /// Include standard system configurations from /etc/mock.
        #[arg(short, long)]
        all: bool,
    },

    /// Inspect a Mock chroot profile or .cfg file and display configuration parameters.
    Inspect {
        /// Profile name or path to .cfg file (defaults to [chroot].profile in dbs.toml).
        target: Option<String>,

        /// Optional custom directory containing mock configs.
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,
    },

    /// Check and validate a chroot configuration and test root creation with Mock.
    Check {
        /// Profile name or path to .cfg file (defaults to [chroot].profile in dbs.toml).
        target: Option<String>,

        /// Optional custom directory containing mock configs.
        #[arg(short = 'd', long)]
        dir: Option<PathBuf>,
    },

    /// Add / import a custom chroot configuration file or directory into DBS workspace.
    Add {
        /// Path to .cfg file or directory containing chroot configurations.
        path: PathBuf,

        /// Destination directory (defaults to ./mock).
        #[arg(short = 'o', long, default_value = "mock")]
        dest: PathBuf,
    },

    /// Initialize a new custom distribution chroot configuration template.
    Init {
        /// Name of the new chroot (e.g. tacos-rolling-x86_64, my-distro-x86_64).
        name: String,

        /// Target architecture.
        #[arg(short = 'a', long, default_value = "x86_64")]
        arch: String,

        /// Destination directory (defaults to ./mock).
        #[arg(short = 'o', long, default_value = "mock")]
        dest: PathBuf,

        /// Max SMP concurrency CPUs for package compilation inside Mock chroot.
        #[arg(long)]
        smp_cpus: Option<usize>,
    },

    /// Drop into an interactive Mock chroot shell at a package's build environment.
    Shell(ShellArgs),
}

/// Arguments for the `lookaside` subcommand.
#[derive(Args, Debug)]
pub struct LookasideArgs {
    /// Path to the lookaside cache root directory (defaults to /srv/dbs/lookaside, or data/lookaside).
    #[arg(short = 'd', long = "dir", global = true, env = "DBS_LOOKASIDE_DIR")]
    pub dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: LookasideCommands,
}

#[derive(Subcommand, Debug)]
pub enum LookasideCommands {
    /// Upload and store a source archive into the lookaside cache and optionally update the dist-git sources file.
    Upload {
        /// Path to the source archive file (tar.gz, tar.xz, etc.).
        #[arg(short, long)]
        file: PathBuf,

        /// Package name (e.g. zstd, strace).
        #[arg(short, long)]
        pkg: String,

        /// Optional path to the package's .spec file to discover package directory and sources manifest.
        #[arg(short, long)]
        spec: Option<PathBuf>,

        /// Do not update the dist-git 'sources' manifest file with the new SHA-512 hash.
        #[arg(long)]
        no_sources: bool,
    },

    /// Retrieve a source archive from local lookaside (via BTRFS reflink) or upstream mirrors.
    Get {
        /// Package name (e.g. zstd, strace).
        #[arg(short, long)]
        pkg: String,

        /// Filename of the source archive (e.g. zstd-1.5.7.tar.gz).
        #[arg(short, long)]
        file: String,

        /// Expected SHA-512 cryptographic hash.
        #[arg(short = 'H', long)]
        hash: String,

        /// Destination path where the file should be staged.
        #[arg(short, long, default_value = ".")]
        dest: PathBuf,
    },

    /// Pre-fetch and synchronize missing source archives for all dist-git packages in a directory.
    Sync {
        /// Path to dist-git directory containing package repositories or a single repository.
        #[arg(short = 'i', long, default_value = "data/distgit")]
        path: PathBuf,

        /// Number of concurrent download workers.
        #[arg(short = 'j', long, default_value = "4")]
        concurrency: usize,
    },

    /// Display lookaside cache metrics, disk usage, CAS objects, and BTRFS filesystem status.
    Status,

    /// Scan and clean up unreferenced/orphaned source archives in the Content-Addressable Storage (.cas).
    Gc {
        /// Dry-run mode: show orphaned files and reclaimable space without deleting.
        #[arg(long)]
        dry_run: bool,

        /// Optional path to dist-git workspace to check for active hash references.
        #[arg(long)]
        distgit: Option<PathBuf>,
    },
}

/// Arguments for the `distro` subcommand suite.
#[derive(Args, Debug)]
pub struct DistroArgs {
    #[command(subcommand)]
    pub action: DistroCommands,
}

/// Actions supported by the `distro` subcommand.
#[derive(Subcommand, Debug)]
pub enum DistroCommands {
    /// List available distribution presets (static) and registered database distributions.
    List,

    /// Register a new distribution in the database catalog.
    Add(crate::cli::os::AddDistroArgs),

    /// Delete a distribution record from the database by ID.
    Delete {
        /// The ID of the distribution to delete
        #[arg(long)]
        id: i32,
    },

    /// Initialize a new distribution repository structure and Mock chroot profile.
    Init {
        /// Distribution identifier (e.g. tacos-stable-x86_64).
        name: String,

        /// Target CPU architecture (e.g. x86_64, aarch64).
        #[arg(short, long, default_value = "x86_64")]
        arch: String,

        /// Distribution release channel (e.g. stable, rolling, testing).
        #[arg(short, long, default_value = "stable")]
        channel: String,

        /// Upstream release version base (e.g. 46, 10).
        #[arg(short, long, default_value = "46")]
        releasever: String,

        /// RPM distribution macro tag (e.g. tcst, tcrs).
        #[arg(short, long, default_value = "tcst")]
        dist: String,

        /// Base directory for distribution repositories (defaults to [distro].dest in dbs.toml).
        #[arg(short, long)]
        dest: Option<PathBuf>,

        /// Directory to store Mock chroot configuration files.
        #[arg(short, long)]
        mock_dir: Option<PathBuf>,

        /// Max SMP concurrency CPUs for package compilation inside Mock chroot.
        #[arg(long)]
        smp_cpus: Option<usize>,
    },

    /// Build a complete distribution: DAG resolution, lookaside source staging, and layered builds.
    Build {
        /// Distribution identifier (defaults to [distro].name in dbs.toml).
        name: Option<String>,

        /// Directory containing dist-git package specifications (defaults to [distgit].dest in dbs.toml).
        #[arg(short = 'i', long)]
        path: Option<PathBuf>,

        /// Mock chroot profile name to use (defaults to [distro].chroot or [chroot].profile in dbs.toml).
        #[arg(short = 'r', long)]
        mock_root: Option<String>,

        /// Directory containing Mock .cfg profile files.
        #[arg(long)]
        mock_config_dir: Option<PathBuf>,

        /// Destination directory for distribution repository output (defaults to [distro].dest in dbs.toml).
        #[arg(short = 'o', long)]
        dest: Option<PathBuf>,

        /// Lookaside cache directory for instant BTRFS CoW source staging (defaults to [distgit].lookaside_dir).
        #[arg(short = 'l', long)]
        lookaside_dir: Option<PathBuf>,

        /// Build worker concurrency (defaults to [distro].workers in dbs.toml).
        #[arg(short = 'j', long)]
        concurrency: Option<usize>,

        /// Max SMP concurrency CPUs for package compilation inside Mock (%_smp_mflags / %_smp_build_ncpus).
        #[arg(long, env = "DBS_SMP_CPUS")]
        smp: Option<usize>,

        /// Staging directory for temporary worker builds (defaults to [distro].staging_dir in dbs.toml).
        #[arg(long)]
        staging_dir: Option<PathBuf>,

        /// Optional GPG key ID to sign RPMs and repomd metadata (defaults to [distro].sign_key).
        #[arg(long)]
        sign_key: Option<String>,

        /// Record builds and capabilities in the database (defaults to [database].record_db in dbs.toml).
        #[arg(long)]
        record_db: bool,

        /// Build a specific distribution stage (e.g. bootstrap, system, desktop).
        #[arg(long)]
        stage: Option<String>,

        /// Build all configured stages in sequential order.
        #[arg(long)]
        stages: bool,

        /// Path to a text file containing package names or spec paths to build (one per line).
        #[arg(short = 'p', long)]
        packages: Option<PathBuf>,

        /// Automatically detect and break circular dependencies via base chroot fallback.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
        break_cycles: bool,

        /// Disable running test suites in Mock and rpmbuild (%check phase).
        #[arg(long)]
        nocheck: bool,

        /// Target package specifications, names, or comps groups/environments to build (e.g. @core, @workstation-product-environment, bash).
        targets: Vec<String>,
    },

    /// Organize RPMs into standard layout, run createrepo_c, optionally GPG sign, and generate client .repo.
    Publish {
        /// Distribution identifier (defaults to [distro].name in dbs.toml).
        name: Option<String>,

        /// Source staging directory where built RPMs are located (defaults to [distro].staging_dir in dbs.toml).
        #[arg(short = 's', long)]
        staging_dir: Option<PathBuf>,

        /// Destination directory for distribution repository (defaults to [distro].dest in dbs.toml).
        #[arg(short = 'd', long)]
        dest: Option<PathBuf>,

        /// Target architecture (defaults to [distro].arch in dbs.toml).
        #[arg(short, long)]
        arch: Option<String>,

        /// Base URL for client .repo file (defaults to [distro].base_url in dbs.toml).
        #[arg(short, long)]
        base_url: Option<String>,

        /// Optional GPG Key ID to sign packages and repomd.xml with (defaults to [distro].sign_key in dbs.toml).
        #[arg(long)]
        sign_key: Option<String>,

        /// Number of parallel workers for createrepo_c (defaults to [distro].workers in dbs.toml).
        #[arg(short = 'j', long)]
        workers: Option<usize>,
    },

    /// Generate an Nginx virtual host configuration or launch a lightweight built-in HTTP repository server.
    Serve {
        /// Distribution root directory (defaults to [distro].dest in dbs.toml).
        #[arg(short, long)]
        path: Option<PathBuf>,

        /// Generate Nginx configuration and write to file instead of running built-in server.
        #[arg(long)]
        nginx_conf: Option<PathBuf>,

        /// Server name domain for Nginx config (defaults to [distro].server_name in dbs.toml).
        #[arg(long)]
        server_name: Option<String>,

        /// HTTP port for built-in repository server or Nginx listen port (defaults to [distro].server_port).
        #[arg(short, long)]
        port: Option<u16>,

        /// Bind host for built-in server (defaults to [distro].server_host in dbs.toml).
        #[arg(long)]
        host: Option<String>,
    },

    /// Display the status and package inventory of a distribution repository.
    Status {
        /// Distribution identifier (defaults to [distro].name in dbs.toml).
        name: Option<String>,

        /// Distribution root directory (defaults to [distro].dest in dbs.toml).
        #[arg(short, long)]
        dest: Option<PathBuf>,

        /// Target architecture (defaults to [distro].arch in dbs.toml).
        #[arg(short, long)]
        arch: Option<String>,
    },
}

/// Arguments for the `db` subcommand.
#[derive(Args, Debug)]
pub struct DbArgs {
    /// Database connection URL (e.g. postgres://dbs:password@localhost:5432/dbs).
    /// Overrides DATABASE_URL environment variable and configuration files.
    #[arg(short = 'u', long = "url", global = true)]
    pub database_url: Option<String>,

    #[command(subcommand)]
    pub command: DbCommands,
}

#[derive(Subcommand, Debug)]
pub enum DbCommands {
    /// Initialize and bootstrap the database schema, tables, and seed data.
    #[command(alias = "init")]
    Bootstrap {
        /// Force re-initialization even if existing tables are detected.
        #[arg(short, long)]
        force: bool,
    },

    /// Check database connectivity, PostgreSQL server version, and table row counts.
    Status,

    /// Reset database: drop all tables/enums and reapply the clean schema.
    Reset {
        /// Force reset without interactive confirmation.
        #[arg(short, long)]
        force: bool,
    },

    /// Dump the embedded SQL schema (up or down) to stdout for manual DBA inspection.
    DumpSchema {
        /// Output the teardown (down.sql) schema instead of initialization (up.sql).
        #[arg(long)]
        down: bool,
    },

    /// Reconcile unexpanded RPM macros across catalog packages using dist-git specs.
    Reconcile {
        /// Optional path to dist-git directory containing spec files (defaults to config).
        #[arg(short, long)]
        path: Option<std::path::PathBuf>,
    },
}

/// Arguments for the `config` subcommand.
#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Display the resolved DBS configuration and loaded configuration file path.
    Show,

    /// Generate a starter dbs.toml configuration file.
    Init {
        /// Destination path for generated configuration file.
        #[arg(short = 'o', long = "output", default_value = "dbs.toml")]
        output: PathBuf,

        /// Overwrite destination file if it already exists.
        #[arg(short, long)]
        force: bool,
    },
}

/// Arguments for the `shell` subcommand.
#[derive(Args, Debug, Clone)]
pub struct ShellArgs {
    /// Package name or spec/SRPM path to enter build chroot for (e.g. gcc, /srv/dbs/tacos/rpm/gcc/gcc.spec).
    pub package: String,

    /// Mock chroot configuration profile name or .cfg path (defaults to [chroot].profile in dbs.toml).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Staging directory containing worker directories (defaults to [distro].staging_dir in dbs.toml).
    #[arg(short = 's', long)]
    pub staging_dir: Option<PathBuf>,

    /// Specific worker unique extension (e.g. w1, w2). Auto-detected if omitted.
    #[arg(short = 'u', long)]
    pub uniqueext: Option<String>,

    /// Working directory inside the chroot (defaults to auto-detected /builddir/build/BUILD/<pkg>*).
    #[arg(long)]
    pub cwd: Option<String>,

    /// Optional command to execute within chroot instead of interactive shell.
    #[arg(trailing_var_arg = true)]
    pub cmd: Vec<String>,
}

/// Arguments for the `retry` subcommand.
#[derive(Args, Debug, Clone)]
pub struct RetryArgs {
    /// Package name or spec/SRPM path to retry building (e.g. gcc, /srv/dbs/tacos/rpm/gcc/gcc.spec).
    #[arg(required_unless_present = "file")]
    pub package: Option<String>,

    /// Path to a text file containing package names or spec paths to retry building (one per line).
    #[arg(short = 'f', long = "file", visible_alias = "packages", short_alias = 'p', value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Mock chroot configuration profile name or .cfg path (defaults to [chroot].profile in dbs.toml).
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Directory for build logs and staged RPM artifacts (defaults to [distro].staging_dir in dbs.toml).
    #[arg(short = 'o', long)]
    pub output_dir: Option<PathBuf>,

    /// Number of concurrent Mock worker processes (defaults to 1 for retry).
    #[arg(short = 'j', long)]
    pub concurrency: Option<usize>,

    /// Max SMP concurrency CPUs for package compilation inside Mock (%_smp_mflags / %_smp_build_ncpus).
    #[arg(long, env = "DBS_SMP_CPUS")]
    pub smp: Option<usize>,

    /// Clean Mock chroot before retrying build (mock --clean).
    #[arg(long)]
    pub clean_chroot: bool,

    /// Record build metrics, status, and output artifacts into PostgreSQL database.
    #[arg(long)]
    pub record_db: bool,

    /// Path to local lookaside cache for instant BTRFS CoW staging of source tarballs.
    #[arg(long, env = "DBS_LOOKASIDE_DIR")]
    pub lookaside_dir: Option<PathBuf>,

    /// Disable running test suites in Mock and rpmbuild (%check phase).
    #[arg(long)]
    pub nocheck: bool,
}

/// Arguments for the `clean` subcommand.
#[derive(Args, Debug, Clone)]
pub struct CleanArgs {
    /// Package name or spec/SRPM path to clean (e.g. gcc, /srv/dbs/tacos/rpm/gcc/gcc.spec).
    pub package: Option<String>,

    /// Clean all staging directories and build artifacts across all packages.
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Clean only worker staging directories and build logs (leaves repository RPMs intact).
    #[arg(long, conflicts_with = "repo_only")]
    pub staging_only: bool,

    /// Clean only published RPMs from the repository (leaves staging logs intact).
    #[arg(long, conflicts_with = "staging_only")]
    pub repo_only: bool,

    /// Clean Mock chroot profile (mock --clean).
    #[arg(long)]
    pub clean_chroot: bool,

    /// Mock chroot configuration profile name or direct path to a .cfg file.
    #[arg(short = 'r', long)]
    pub mock_root: Option<String>,

    /// Path to directory containing Mock configuration profiles.
    #[arg(long)]
    pub mock_config_dir: Option<PathBuf>,

    /// Staging directory containing worker directories (defaults to [distro].staging_dir in dbs.toml).
    #[arg(short = 's', long)]
    pub staging_dir: Option<PathBuf>,

    /// Repository root directory containing published RPMs (defaults to [distro].dest in dbs.toml).
    #[arg(long)]
    pub repo_dir: Option<PathBuf>,

    /// Target architecture to clean in repo (defaults to [distro].arch in dbs.toml).
    #[arg(long)]
    pub arch: Option<String>,

    /// Do not update createrepo_c metadata after removing RPMs.
    #[arg(long)]
    pub no_repo_update: bool,

    /// Do not reset database build status and package_artifact records.
    #[arg(long)]
    pub no_db: bool,
}

/// Arguments for the `comps` subcommand.
#[derive(Args, Debug, Clone)]
pub struct CompsArgs {
    #[command(subcommand)]
    pub action: CompsCommands,
}

/// Comps subcommands for inspecting and listing RPM comps metadata.
#[derive(Subcommand, Debug, Clone)]
pub enum CompsCommands {
    /// Inspect an environment or group, showing constituent groups, packages, and local dist-git status.
    Inspect(CompsInspectArgs),

    /// List package names contained in a comps environment or group (one per line).
    #[command(alias = "packages", alias = "pkgs")]
    ListPackages(CompsListPackagesArgs),

    /// List all available comps environments and groups from repositories.
    List(CompsListArgs),
}

/// Arguments for `dbs comps inspect`.
#[derive(Args, Debug, Clone)]
pub struct CompsInspectArgs {
    /// Comps environment or group identifier (e.g. @workstation-product-environment, core).
    pub target: String,

    /// Include optional packages in inspection.
    #[arg(long)]
    pub optional: bool,

    /// Path to dist-git directory to check local package presence against.
    #[arg(short = 'd', long)]
    pub dest: Option<PathBuf>,
}

/// Arguments for `dbs comps list-packages`.
#[derive(Args, Debug, Clone)]
pub struct CompsListPackagesArgs {
    /// Comps environment or group identifier (e.g. @core, @workstation-product-environment).
    pub target: String,

    /// Include optional packages in output.
    #[arg(long)]
    pub optional: bool,

    /// Only list packages that are present locally in dist-git directory.
    #[arg(long, conflicts_with = "missing_only")]
    pub present_only: bool,

    /// Only list packages that are missing locally from dist-git directory.
    #[arg(long, conflicts_with = "present_only")]
    pub missing_only: bool,

    /// Path to dist-git directory to check local package presence against.
    #[arg(short = 'd', long)]
    pub dest: Option<PathBuf>,
}

/// Arguments for `dbs comps list`.
#[derive(Args, Debug, Clone)]
pub struct CompsListArgs {
    /// Show only environments.
    #[arg(long, conflicts_with = "groups")]
    pub environments: bool,

    /// Show only groups (including hidden groups).
    #[arg(long, conflicts_with = "environments")]
    pub groups: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_build_nocheck_flag() {
        let cli = Cli::try_parse_from(["dbs", "build", "--nocheck", "test.spec"]).unwrap();
        match cli.command {
            Commands::Build(args) => {
                assert!(args.nocheck);
            }
            _ => panic!("Expected Commands::Build"),
        }

        let cli_default = Cli::try_parse_from(["dbs", "build", "test.spec"]).unwrap();
        match cli_default.command {
            Commands::Build(args) => {
                assert!(!args.nocheck);
            }
            _ => panic!("Expected Commands::Build"),
        }
    }

    #[test]
    fn test_cli_retry_nocheck_flag() {
        let cli = Cli::try_parse_from(["dbs", "retry", "--nocheck", "cockpit"]).unwrap();
        match cli.command {
            Commands::Retry(args) => {
                assert!(args.nocheck);
                assert_eq!(args.package.as_deref(), Some("cockpit"));
            }
            _ => panic!("Expected Commands::Retry"),
        }
    }

    #[test]
    fn test_cli_dag_nocheck_flag() {
        let cli = Cli::try_parse_from(["dbs", "dag", "--build", "--nocheck"]).unwrap();
        match cli.command {
            Commands::Dag(args) => {
                assert!(args.build);
                assert!(args.nocheck);
            }
            _ => panic!("Expected Commands::Dag"),
        }
    }

    #[test]
    fn test_cli_distro_build_nocheck_flag() {
        let cli = Cli::try_parse_from(["dbs", "distro", "build", "--nocheck", "tacos"]).unwrap();
        match cli.command {
            Commands::Distro(args) => match args.action {
                crate::cli::DistroCommands::Build { nocheck, .. } => {
                    assert!(nocheck);
                }
                _ => panic!("Expected DistroCommands::Build"),
            },
            _ => panic!("Expected Commands::Distro"),
        }
    }
}



