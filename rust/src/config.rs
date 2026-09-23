//! Configuration management for Distribution Build System (DBS).
//!
//! Provides TOML configuration parsing, standard location discovery,
//! and unified settings across database, chroot, distgit, and distro subsystems.

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use eyre::{eyre, Result};

/// Root configuration structure representing `dbs.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbsConfig {
    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub chroot: ChrootConfig,

    #[serde(default)]
    pub distgit: DistgitConfig,

    #[serde(default)]
    pub distro: DistroSettings,
}

impl Default for DbsConfig {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            chroot: ChrootConfig::default(),
            distgit: DistgitConfig::default(),
            distro: DistroSettings::default(),
        }
    }
}

/// Settings for PostgreSQL database connectivity and metric recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL (e.g. postgres://dbs:password@127.0.0.1:5432/dbs).
    pub url: Option<String>,

    /// Automatically record distgit syncs and package builds in the database.
    #[serde(default)]
    pub record_db: bool,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: None,
            record_db: false,
        }
    }
}

/// Settings for Mock chroot environments and compilation isolation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChrootConfig {
    /// Default chroot profile name or path (e.g. tacos-rolling-x86_64).
    #[serde(default = "default_chroot_profile")]
    pub profile: String,

    /// Mock configuration directory containing .cfg profiles and templates/.
    pub config_dir: Option<PathBuf>,

    /// Additional filesystem directories searched for chroot .cfg profiles and templates.
    #[serde(default = "default_search_paths")]
    pub search_paths: Vec<PathBuf>,

    /// Target hardware architecture (e.g. x86_64, aarch64).
    #[serde(default = "default_arch")]
    pub arch: String,

    /// Max SMP concurrency CPUs for package compilation inside the buildroot.
    #[serde(default = "default_smp_cpus")]
    pub smp_cpus: usize,
}

fn default_chroot_profile() -> String {
    "tacos-rolling-x86_64".to_string()
}

fn default_search_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("mock"),
        PathBuf::from("../mock"),
        PathBuf::from("../tacos/mock"),
        PathBuf::from("/etc/mock"),
    ]
}

fn default_arch() -> String {
    "x86_64".to_string()
}

fn default_smp_cpus() -> usize {
    2
}

impl Default for ChrootConfig {
    fn default() -> Self {
        Self {
            profile: default_chroot_profile(),
            config_dir: None,
            search_paths: default_search_paths(),
            arch: default_arch(),
            smp_cpus: default_smp_cpus(),
        }
    }
}

/// Settings for dist-git synchronization and lookaside cache storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistgitConfig {
    /// Default upstream distribution preset (e.g. fedora-rawhide, centos-stream-10, tacos).
    #[serde(default = "default_distgit_distro")]
    pub distro: String,

    /// Destination directory for cloned dist-git package repositories.
    #[serde(default = "default_distgit_dest")]
    pub dest: PathBuf,

    /// Dist-git lookaside source cache directory (backed by BTRFS CoW).
    #[serde(default = "default_lookaside_dir")]
    pub lookaside_dir: PathBuf,

    /// Concurrency workers for parallel git operations and downloads.
    #[serde(default = "default_workers")]
    pub concurrency: usize,
}

fn default_distgit_distro() -> String {
    "fedora-rawhide".to_string()
}

fn default_distgit_dest() -> PathBuf {
    PathBuf::from("data/distgit")
}

fn default_lookaside_dir() -> PathBuf {
    PathBuf::from("data/lookaside")
}

fn default_workers() -> usize {
    4
}

impl Default for DistgitConfig {
    fn default() -> Self {
        Self {
            distro: default_distgit_distro(),
            dest: default_distgit_dest(),
            lookaside_dir: default_lookaside_dir(),
            concurrency: default_workers(),
        }
    }
}

/// Settings for distribution repository lifecycle, publishing, and HTTP serving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistroSettings {
    /// Target distribution identifier (e.g. tacos-stable-x86_64).
    #[serde(default = "default_distro_name")]
    pub name: String,

    /// Chroot profile name used for distribution builds (defaults to `chroot.profile` if unset).
    pub chroot: Option<String>,

    /// Root repository directory for published RPMs and repodata metadata.
    #[serde(default = "default_distro_dest")]
    pub dest: PathBuf,

    /// Staging directory for temporary worker builds and logs.
    #[serde(default = "default_staging_dir")]
    pub staging_dir: PathBuf,

    /// Target distribution architecture.
    #[serde(default = "default_arch")]
    pub arch: String,

    /// Base URL for generated client .repo definitions.
    #[serde(default = "default_base_url")]
    pub base_url: String,

    /// Optional GPG key ID or fingerprint for signing RPMs and repomd.xml.
    pub sign_key: Option<String>,

    /// Worker concurrency for createrepo_c metadata generation.
    #[serde(default = "default_workers")]
    pub workers: usize,

    /// Repository HTTP server hostname.
    #[serde(default = "default_server_name")]
    pub server_name: String,

    /// Repository HTTP server bind address.
    #[serde(default = "default_server_host")]
    pub server_host: String,

    /// Repository HTTP server bind port.
    #[serde(default = "default_server_port")]
    pub server_port: u16,
}

fn default_distro_name() -> String {
    "tacos-stable-x86_64".to_string()
}

fn default_distro_dest() -> PathBuf {
    PathBuf::from("/srv/dbs/tacos/distro")
}

fn default_staging_dir() -> PathBuf {
    PathBuf::from("/srv/dbs/tacos/staging")
}

fn default_base_url() -> String {
    "http://repos.tacos.org.mx".to_string()
}

fn default_server_name() -> String {
    "repos.tacos.org.mx".to_string()
}

fn default_server_host() -> String {
    "0.0.0.0".to_string()
}

fn default_server_port() -> u16 {
    8080
}

impl Default for DistroSettings {
    fn default() -> Self {
        Self {
            name: default_distro_name(),
            chroot: None,
            dest: default_distro_dest(),
            staging_dir: default_staging_dir(),
            arch: default_arch(),
            base_url: default_base_url(),
            sign_key: None,
            workers: default_workers(),
            server_name: default_server_name(),
            server_host: default_server_host(),
            server_port: default_server_port(),
        }
    }
}

impl DbsConfig {
    /// Loads configuration from an explicit path or standard search locations.
    ///
    /// Discovery precedence:
    /// 1. Explicit path passed via `--config`
    /// 2. `./dbs.toml` or `../dbs.toml`
    /// 3. `$HOME/.config/dbs/dbs.toml` (or `config.toml`)
    /// 4. `/etc/dbs/dbs.toml` (or `config.toml`)
    /// 5. Default built-in fallback configuration
    pub fn load(explicit_path: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        if let Some(path) = explicit_path {
            if !path.exists() {
                return Err(eyre!("Configuration file not found at: {}", path.display()));
            }
            let content = fs::read_to_string(path)
                .map_err(|e| eyre!("Failed to read config file {}: {}", path.display(), e))?;
            let config: DbsConfig = toml::from_str(&content)
                .map_err(|e| eyre!("Failed to parse TOML configuration from {}: {}", path.display(), e))?;
            return Ok((config, Some(path.to_path_buf())));
        }

        // Search candidate paths
        let mut candidates = vec![
            PathBuf::from("dbs.toml"),
            PathBuf::from("../dbs.toml"),
            PathBuf::from("config/dbs.toml"),
        ];

        if let Ok(home) = std::env::var("HOME") {
            candidates.push(PathBuf::from(home.clone()).join(".config/dbs/dbs.toml"));
            candidates.push(PathBuf::from(home).join(".config/dbs/config.toml"));
        }

        candidates.push(PathBuf::from("/etc/dbs/dbs.toml"));
        candidates.push(PathBuf::from("/etc/dbs/config.toml"));

        for p in &candidates {
            if p.is_file() {
                let content = fs::read_to_string(p)
                    .map_err(|e| eyre!("Failed to read config file {}: {}", p.display(), e))?;
                let config: DbsConfig = toml::from_str(&content)
                    .map_err(|e| eyre!("Failed to parse TOML configuration from {}: {}", p.display(), e))?;
                return Ok((config, Some(p.clone())));
            }
        }

        // Fallback to defaults if no configuration file was found
        Ok((DbsConfig::default(), None))
    }

    /// Generates a well-documented starter `dbs.toml` configuration template.
    pub fn sample_toml() -> &'static str {
        r#"# ==============================================================================
# Distribution Build System (DBS) Configuration
# ==============================================================================

[database]
# PostgreSQL connection string (overrides .env)
# url = "postgres://dbs:prueba123@127.0.0.1:5432/dbs"

# Automatically record package synchronizations and build metrics in PostgreSQL
record_db = false

[chroot]
# Default chroot profile name or path (in mock/ or ../mock/)
profile = "tacos-rolling-x86_64"

# Additional directories searched for chroot .cfg profiles and templates
search_paths = [
    "mock",
    "../mock",
    "../tacos/mock",
    "/etc/mock",
]

# Default compilation architecture
arch = "x86_64"

# Max SMP CPUs passed to Mock chroot (%_smp_build_ncpus)
smp_cpus = 2

[distgit]
# Default upstream distribution preset (e.g. fedora-rawhide, centos-stream-10, tacos)
distro = "fedora-rawhide"

# Destination directory for cloned dist-git package repositories
dest = "/srv/dbs/tacos/rpm"

# Dist-git lookaside source cache directory (BTRFS CoW)
lookaside_dir = "/srv/dbs/lookaside"

# Parallel concurrency for git clone and synchronization
concurrency = 4

[distro]
# Target distribution identifier
name = "tacos-stable-x86_64"

# Specific chroot profile for distribution builds (defaults to chroot.profile if unset)
chroot = "tacos-stable-x86_64"

# Root repository destination directory for published RPMs and repodata
dest = "/srv/dbs/tacos/distro"

# Temporary staging directory for worker build logs and artifacts
staging_dir = "/srv/dbs/tacos/staging"

# Target distribution architecture
arch = "x86_64"

# Base URL written into client .repo repository configuration files
base_url = "http://repos.tacos.org.mx"

# Optional GPG key ID or email to sign RPMs and repomd.xml
# sign_key = "security@tacos.org.mx"

# Concurrency for createrepo_c metadata generation
workers = 4

# Domain name for generated Nginx virtual hosts
server_name = "repos.tacos.org.mx"

# Built-in HTTP repository server bind address and port
server_host = "0.0.0.0"
server_port = 8080
"#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_valid() {
        let cfg = DbsConfig::default();
        assert_eq!(cfg.chroot.profile, "tacos-rolling-x86_64");
        assert_eq!(cfg.distgit.distro, "fedora-rawhide");
        assert_eq!(cfg.distro.name, "tacos-stable-x86_64");
        assert_eq!(cfg.distro.server_port, 8080);
        assert!(!cfg.database.record_db);
    }

    #[test]
    fn test_parse_sample_toml() {
        let sample = DbsConfig::sample_toml();
        let parsed: DbsConfig = toml::from_str(sample).expect("Failed to parse sample TOML");
        assert_eq!(parsed.chroot.profile, "tacos-rolling-x86_64");
        assert_eq!(parsed.distgit.distro, "fedora-rawhide");
        assert_eq!(parsed.distgit.dest, PathBuf::from("/srv/dbs/tacos/rpm"));
        assert_eq!(parsed.distgit.lookaside_dir, PathBuf::from("/srv/dbs/lookaside"));
        assert_eq!(parsed.distro.name, "tacos-stable-x86_64");
        assert_eq!(parsed.distro.chroot, Some("tacos-stable-x86_64".to_string()));
        assert_eq!(parsed.distro.server_port, 8080);
    }

    #[test]
    fn test_deserialize_partial_toml() {
        let toml_data = r#"
        [database]
        url = "postgres://custom:secret@db.local:5432/custom_dbs"
        record_db = true

        [distgit]
        distro = "centos-stream-10"
        "#;

        let parsed: DbsConfig = toml::from_str(toml_data).expect("Failed to parse partial TOML");
        assert_eq!(
            parsed.database.url,
            Some("postgres://custom:secret@db.local:5432/custom_dbs".to_string())
        );
        assert!(parsed.database.record_db);
        assert_eq!(parsed.distgit.distro, "centos-stream-10");
        // Defaults should be preserved for omitted sections
        assert_eq!(parsed.chroot.profile, "tacos-rolling-x86_64");
        assert_eq!(parsed.distro.name, "tacos-stable-x86_64");
    }

    #[test]
    fn test_load_explicit_nonexistent() {
        let result = DbsConfig::load(Some(Path::new("/nonexistent/path/dbs.toml")));
        assert!(result.is_err());
    }
}
