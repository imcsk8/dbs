//! Configuration management for Distribution Build System (DBS).
//!
//! Provides TOML configuration parsing, standard location discovery,
//! and unified settings across database, chroot, distgit, and distro subsystems.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use eyre::{eyre, Result};

use crate::distgit::provider::{ApiType, DistroConfig};

/// Root configuration structure representing `dbs.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct DbsConfig {
    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub chroot: ChrootConfig,

    #[serde(default)]
    pub distgit: DistgitConfig,

    #[serde(default)]
    pub lookaside: LookasideConfig,

    #[serde(default)]
    pub distro: DistroSettings,

    #[serde(default)]
    pub distros: HashMap<String, DistroProfileConfig>,

    #[serde(default)]
    pub build: BuildConfig,
}


/// Settings for package compilation and smart build gating.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    /// Skip compiling packages if the package version/release is already built.
    #[serde(default = "default_skip_existing")]
    pub skip_existing: bool,

    /// Disable running test suites in Mock and rpmbuild (%check phase).
    #[serde(default)]
    pub nocheck: bool,

    /// Specific package names that should always skip test suites (%check phase).
    #[serde(default)]
    pub nocheck_packages: Vec<String>,
}

fn default_skip_existing() -> bool {
    true
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            skip_existing: default_skip_existing(),
            nocheck: false,
            nocheck_packages: Vec::new(),
        }
    }
}

/// Settings for PostgreSQL database connectivity and metric recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL (e.g. postgres://dbs:password@127.0.0.1:5432/dbs).
    pub url: Option<String>,

    /// Automatically record distgit syncs and package builds in the database.
    #[serde(default)]
    pub record_db: bool,
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
    match std::thread::available_parallelism() {
        Ok(n) => n.get(),
        Err(_) => 24,
    }
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

/// Settings for dist-git lookaside source cache storage and upstream remote mirrors.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LookasideConfig {
    /// Optional root directory for lookaside cache (overrides distgit.lookaside_dir if specified).
    pub dir: Option<PathBuf>,

    /// Ordered remote lookaside URL templates to query when downloading sources (prepended to defaults).
    #[serde(default)]
    pub remotes: Vec<String>,
}

/// Custom distribution profile settings in `dbs.toml` (`[distros.<name>]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DistroProfileConfig {
    /// Human-readable distribution name.
    pub name: Option<String>,

    /// Version or release identifier.
    pub version: Option<String>,

    /// Git clone URL template (supports `{package}` and `{pkg}`).
    pub git_url: Option<String>,

    /// Default branch to clone/track.
    pub branch: Option<String>,

    /// Base URL for the lookaside source tarball cache.
    pub lookaside_cache_url: Option<String>,

    /// Ordered remote lookaside URL templates to query for this distribution.
    #[serde(default)]
    pub lookaside_urls: Vec<String>,

    /// Dist-git forge API type (pagure, gitlab, forgejo, repodata, generic-git).
    pub api_type: Option<ApiType>,

    /// Forge API base URL.
    pub api_url: Option<String>,

    /// Mock chroot profile name.
    pub mock_chroot: Option<String>,

    /// Package manager (e.g. rpm, dnf5).
    pub package_manager: Option<String>,

    /// Target hardware architecture (e.g. x86_64, aarch64).
    pub architecture: Option<String>,
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

    /// Base URL of new origin git remote to automatically configure for synced repos (e.g. https://codeberg.org/imcsk8/tacos).
    #[serde(default)]
    pub new_top_origin: Option<String>,

    /// Optional API key / personal access token for dist-git forge authentication.
    #[serde(default)]
    pub api_key: Option<String>,

    /// Optional custom git clone URL template overriding preset (supports {package}, {pkg}).
    #[serde(default)]
    pub url_template: Option<String>,

    /// Optional custom git branch overriding preset.
    #[serde(default)]
    pub branch: Option<String>,

    /// Optional dist-git forge API base URL overriding preset.
    #[serde(default)]
    pub api_url: Option<String>,

    /// Optional dist-git forge API type overriding preset.
    #[serde(default)]
    pub api_type: Option<ApiType>,

    /// Optional primary lookaside cache base URL overriding preset.
    #[serde(default)]
    pub lookaside_cache_url: Option<String>,

    /// Additional remote lookaside URL templates to prepend to the default lookaside mirrors.
    #[serde(default)]
    pub lookaside_urls: Vec<String>,
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
            new_top_origin: None,
            api_key: None,
            url_template: None,
            branch: None,
            api_url: None,
            api_type: None,
            lookaside_cache_url: None,
            lookaside_urls: Vec::new(),
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

    /// Staged distribution build pipelines (stage name -> package targets).
    #[serde(default)]
    pub stages: HashMap<String, Vec<String>>,

    /// Explicit execution order of stages (defaults to lexicographical sort of stage names).
    #[serde(default)]
    pub stage_order: Option<Vec<String>>,
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
            stages: HashMap::new(),
            stage_order: None,
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

    /// Computes the effective list of lookaside remote URL templates.
    ///
    /// Priority order (prepending custom configuration to defaults):
    /// 1. `[lookaside].remotes` from configuration
    /// 2. `[distgit].lookaside_urls` from configuration
    /// 3. Active distribution profile `[distros.<distro>].lookaside_urls`
    /// 4. Built-in distribution preset templates / defaults
    ///
    /// Duplicates are eliminated while preserving priority ordering.
    pub fn effective_lookaside_remotes(&self) -> Vec<String> {
        let mut urls = Vec::new();

        for u in &self.lookaside.remotes {
            if !urls.contains(u) {
                urls.push(u.clone());
            }
        }
        for u in &self.distgit.lookaside_urls {
            if !urls.contains(u) {
                urls.push(u.clone());
            }
        }
        if let Some(profile) = self.distros.get(&self.distgit.distro) {
            for u in &profile.lookaside_urls {
                if !urls.contains(u) {
                    urls.push(u.clone());
                }
            }
        }
        if let Some(preset) = DistroConfig::from_preset(&self.distgit.distro) {
            for u in &preset.lookaside_urls {
                if !urls.contains(u) {
                    urls.push(u.clone());
                }
            }
        }
        for u in crate::lookaside::default_lookaside_templates() {
            if !urls.contains(&u) {
                urls.push(u);
            }
        }

        urls
    }

    /// Resolves the comprehensive distribution configuration by merging:
    /// 1. Built-in distribution preset (if matching)
    /// 2. Custom distribution profile defined under `[distros.<distro>]`
    /// 3. Top-level overrides under `[distgit]` (url_template, branch, api_url, api_type, lookaside_cache_url)
    /// 4. Effective lookaside URL templates prepended to defaults
    pub fn resolve_distro(&self, distro_name: &str) -> Result<DistroConfig> {
        let mut cfg = if let Some(preset) = DistroConfig::from_preset(distro_name) {
            preset
        } else if let Some(profile) = self.distros.get(distro_name) {
            DistroConfig {
                name: profile.name.clone().unwrap_or_else(|| distro_name.to_string()),
                version: profile.version.clone().unwrap_or_else(|| "custom".to_string()),
                dist_git_url_template: profile.git_url.clone().unwrap_or_else(|| {
                    "https://src.fedoraproject.org/rpms/{package}.git".to_string()
                }),
                dist_git_branch: profile.branch.clone().unwrap_or_else(|| "rawhide".to_string()),
                lookaside_cache_url: profile.lookaside_cache_url.clone(),
                lookaside_urls: profile.lookaside_urls.clone(),
                api_type: profile.api_type.unwrap_or(ApiType::Pagure),
                api_url: profile.api_url.clone(),
                mock_chroot: profile.mock_chroot.clone(),
                package_manager: profile.package_manager.clone().unwrap_or_else(|| "rpm".to_string()),
                architecture: profile.architecture.clone().unwrap_or_else(|| self.chroot.arch.clone()),
            }
        } else {
            return Err(eyre!(
                "Unknown distribution preset or custom profile '{}'. Check dbs.toml [distros] or available presets.",
                distro_name
            ));
        };

        // Merge profile overrides if present (even if distro_name also matched a preset)
        if let Some(profile) = self.distros.get(distro_name) {
            if let Some(ref name) = profile.name { cfg.name = name.clone(); }
            if let Some(ref version) = profile.version { cfg.version = version.clone(); }
            if let Some(ref git_url) = profile.git_url { cfg.dist_git_url_template = git_url.clone(); }
            if let Some(ref branch) = profile.branch { cfg.dist_git_branch = branch.clone(); }
            if let Some(ref lookaside_cache_url) = profile.lookaside_cache_url {
                cfg.lookaside_cache_url = Some(lookaside_cache_url.clone());
            }
            if let Some(api_type) = profile.api_type { cfg.api_type = api_type; }
            if let Some(ref api_url) = profile.api_url { cfg.api_url = Some(api_url.clone()); }
            if let Some(ref mock_chroot) = profile.mock_chroot { cfg.mock_chroot = Some(mock_chroot.clone()); }
            if let Some(ref pkg_mgr) = profile.package_manager { cfg.package_manager = pkg_mgr.clone(); }
            if let Some(ref arch) = profile.architecture { cfg.architecture = arch.clone(); }
        }

        // Apply top-level distgit overrides if distro matches self.distgit.distro
        if distro_name == self.distgit.distro {
            if let Some(ref tmpl) = self.distgit.url_template {
                cfg.dist_git_url_template = tmpl.clone();
            }
            if let Some(ref branch) = self.distgit.branch {
                cfg.dist_git_branch = branch.clone();
            }
            if let Some(ref api_url) = self.distgit.api_url {
                cfg.api_url = Some(api_url.clone());
            }
            if let Some(api_type) = self.distgit.api_type {
                cfg.api_type = api_type;
            }
            if let Some(ref lookaside_cache_url) = self.distgit.lookaside_cache_url {
                cfg.lookaside_cache_url = Some(lookaside_cache_url.clone());
            }
        }

        // Compute combined lookaside URLs:
        // Priority order (TOML remotes prepended to preset defaults):
        // 1. [lookaside].remotes
        // 2. [distgit].lookaside_urls
        // 3. [distros.<distro>].lookaside_urls
        // 4. cfg.lookaside_urls (from preset or defaults)
        let mut effective_lookaside = Vec::new();
        for u in &self.lookaside.remotes {
            if !effective_lookaside.contains(u) {
                effective_lookaside.push(u.clone());
            }
        }
        for u in &self.distgit.lookaside_urls {
            if !effective_lookaside.contains(u) {
                effective_lookaside.push(u.clone());
            }
        }
        if let Some(profile) = self.distros.get(distro_name) {
            for u in &profile.lookaside_urls {
                if !effective_lookaside.contains(u) {
                    effective_lookaside.push(u.clone());
                }
            }
        }
        for u in &cfg.lookaside_urls {
            if !effective_lookaside.contains(u) {
                effective_lookaside.push(u.clone());
            }
        }
        for u in crate::lookaside::default_lookaside_templates() {
            if !effective_lookaside.contains(&u) {
                effective_lookaside.push(u);
            }
        }

        cfg.lookaside_urls = effective_lookaside;
        Ok(cfg)
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
smp_cpus = 24

[distgit]
# Default upstream distribution preset (e.g. fedora-rawhide, centos-stream-10, tacos)
distro = "fedora-rawhide"

# Destination directory for cloned dist-git package repositories
dest = "/srv/dbs/tacos/rpm"

# Dist-git lookaside source cache directory (BTRFS CoW)
lookaside_dir = "/srv/dbs/lookaside"

# Parallel concurrency for git clone and synchronization
concurrency = 4

# Base URL of new origin git remote to automatically configure for synced repos
# e.g. https://codeberg.org/imcsk8/tacos will configure https://codeberg.org/imcsk8/tacos/<package>
# new_top_origin = "https://codeberg.org/imcsk8/tacos"

# Optional API token / key for authenticating with dist-git forge APIs
# api_key = ""

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

    #[test]
    fn test_deserialize_new_top_origin_and_api_key() {
        let toml_data = r#"
        [distgit]
        distro = "fedora-rawhide"
        new_top_origin = "https://codeberg.org/imcsk8/tacos"
        api_key = "secret_forge_token"
        "#;
        let parsed: DbsConfig = toml::from_str(toml_data).expect("Failed to parse TOML");
        assert_eq!(parsed.distgit.new_top_origin.as_deref(), Some("https://codeberg.org/imcsk8/tacos"));
        assert_eq!(parsed.distgit.api_key.as_deref(), Some("secret_forge_token"));
    }

    #[test]
    fn test_deserialize_build_config_nocheck() {
        let toml_data = r#"
        [build]
        skip_existing = false
        nocheck = true
        nocheck_packages = ["cockpit", "git"]
        "#;
        let parsed: DbsConfig = toml::from_str(toml_data).expect("Failed to parse TOML");
        assert!(!parsed.build.skip_existing);
        assert!(parsed.build.nocheck);
        assert_eq!(parsed.build.nocheck_packages, vec!["cockpit", "git"]);
    }

    #[test]
    fn test_deserialize_lookaside_and_distros() {
        let toml_data = r#"
        [distgit]
        distro = "my-custom-distro"
        url_template = "https://git.example.com/rpms/{package}.git"
        branch = "eln"

        [lookaside]
        remotes = [
            "https://cache1.example.com/sources/{package}/{filename}",
            "https://cache2.example.com/sources/{package}/{filename}/{hashtype}/{hash}/{filename}"
        ]

        [distros.my-custom-distro]
        name = "My Custom Distro"
        version = "1.0"
        git_url = "https://git.example.com/rpms/{package}.git"
        branch = "eln"
        lookaside_urls = [
            "https://distro-cache.example.com/{package}/{filename}"
        ]
        api_type = "forgejo"
        api_url = "https://git.example.com/api/v1"
        "#;

        let parsed: DbsConfig = toml::from_str(toml_data).expect("Failed to parse TOML with lookaside and distros");
        assert_eq!(parsed.distgit.url_template.as_deref(), Some("https://git.example.com/rpms/{package}.git"));
        assert_eq!(parsed.distgit.branch.as_deref(), Some("eln"));
        assert_eq!(parsed.lookaside.remotes.len(), 2);
        assert!(parsed.distros.contains_key("my-custom-distro"));

        let resolved = parsed.resolve_distro("my-custom-distro").expect("Failed to resolve custom distro");
        assert_eq!(resolved.name, "My Custom Distro");
        assert_eq!(resolved.dist_git_branch, "eln");
        assert_eq!(resolved.api_type, ApiType::Forgejo);

        // Verify that lookaside remotes are PREPENDED in priority order:
        // 1. [lookaside].remotes
        // 2. [distros.my-custom-distro].lookaside_urls
        // 3. Defaults
        assert_eq!(resolved.lookaside_urls[0], "https://cache1.example.com/sources/{package}/{filename}");
        assert_eq!(resolved.lookaside_urls[1], "https://cache2.example.com/sources/{package}/{filename}/{hashtype}/{hash}/{filename}");
        assert_eq!(resolved.lookaside_urls[2], "https://distro-cache.example.com/{package}/{filename}");
        assert!(resolved.lookaside_urls.contains(&"https://repos.tacos.org.mx/sources/{package}/{filename}".to_string()));
    }
}

