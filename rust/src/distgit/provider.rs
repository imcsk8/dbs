//! Distribution configurations and dist-git provider presets for DBS.
//!
//! Supports any Linux distribution using dist-git (e.g., Fedora, CentOS Stream,
//! TacOS, RHEL, and custom forks).

use serde::{Deserialize, Serialize};

/// Supported API backends for querying dist-git projects and packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiType {
    /// Pagure API (e.g. src.fedoraproject.org).
    Pagure,
    /// GitLab API (e.g. gitlab.com/redhat/centos-stream/rpms).
    GitLab,
    /// Forgejo / Gitea API (e.g. codeberg.org).
    Forgejo,
    /// Direct repository metadata (repomd.xml).
    Repodata,
    /// Generic Git server without a search API.
    GenericGit,
}

impl std::fmt::Display for ApiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiType::Pagure => write!(f, "pagure"),
            ApiType::GitLab => write!(f, "gitlab"),
            ApiType::Forgejo => write!(f, "forgejo"),
            ApiType::Repodata => write!(f, "repodata"),
            ApiType::GenericGit => write!(f, "generic-git"),
        }
    }
}

/// Configuration settings for a target distribution using dist-git.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistroConfig {
    /// Human-readable distribution name (e.g. "Fedora Rawhide", "CentOS Stream 10").
    pub name: String,
    /// Version or release identifier (e.g. "rawhide", "10", "9").
    pub version: String,
    /// Template for constructing git clone URLs. Supports `{package}` placeholder.
    pub dist_git_url_template: String,
    /// Default git branch to clone/track.
    pub dist_git_branch: String,
    /// Base URL for the lookaside source tarball cache.
    pub lookaside_cache_url: Option<String>,
    /// Type of API used for repository discovery.
    pub api_type: ApiType,
    /// Base API URL for project search and catalog discovery.
    pub api_url: Option<String>,
    /// Default Mock chroot profile name.
    pub mock_chroot: Option<String>,
    /// Package manager associated with this distribution (e.g. "rpm").
    pub package_manager: String,
    /// Architecture target (e.g. "x86_64").
    pub architecture: String,
}

impl DistroConfig {
    /// Generates the exact clone URL for a given package name.
    pub fn git_url_for_package(&self, package_name: &str) -> String {
        self.dist_git_url_template.replace("{package}", package_name)
    }

    /// Presets for Fedora Rawhide dist-git.
    pub fn fedora_rawhide() -> Self {
        Self {
            name: "Fedora".to_string(),
            version: "Rawhide".to_string(),
            dist_git_url_template: "https://src.fedoraproject.org/rpms/{package}.git".to_string(),
            dist_git_branch: "rawhide".to_string(),
            lookaside_cache_url: Some("https://src.fedoraproject.org/repo/pkgs".to_string()),
            api_type: ApiType::Pagure,
            api_url: Some("https://src.fedoraproject.org/api/0".to_string()),
            mock_chroot: Some("fedora-rawhide-x86_64".to_string()),
            package_manager: "rpm".to_string(),
            architecture: "x86_64".to_string(),
        }
    }

    /// Presets for CentOS Stream 10 dist-git.
    pub fn centos_stream_10() -> Self {
        Self {
            name: "CentOS Stream".to_string(),
            version: "10".to_string(),
            dist_git_url_template: "https://gitlab.com/redhat/centos-stream/rpms/{package}.git".to_string(),
            dist_git_branch: "c10s".to_string(),
            lookaside_cache_url: Some("https://sources.stream.centos.org/sources/rpms".to_string()),
            api_type: ApiType::GitLab,
            api_url: Some("https://gitlab.com/api/v4/groups/8794173/projects".to_string()),
            mock_chroot: Some("centos-stream-10-x86_64".to_string()),
            package_manager: "rpm".to_string(),
            architecture: "x86_64".to_string(),
        }
    }

    /// Presets for CentOS Stream 9 dist-git.
    pub fn centos_stream_9() -> Self {
        Self {
            name: "CentOS Stream".to_string(),
            version: "9".to_string(),
            dist_git_url_template: "https://gitlab.com/redhat/centos-stream/rpms/{package}.git".to_string(),
            dist_git_branch: "c9s".to_string(),
            lookaside_cache_url: Some("https://sources.stream.centos.org/sources/rpms".to_string()),
            api_type: ApiType::GitLab,
            api_url: Some("https://gitlab.com/api/v4/groups/8794173/projects".to_string()),
            mock_chroot: Some("centos-stream-9-x86_64".to_string()),
            package_manager: "rpm".to_string(),
            architecture: "x86_64".to_string(),
        }
    }

    /// Presets for TacOS Rolling release based on ELN/Rawhide.
    pub fn tacos_rolling() -> Self {
        Self {
            name: "TacOS".to_string(),
            version: "Rolling".to_string(),
            dist_git_url_template: "https://codeberg.org/imcsk8/tacos.git".to_string(),
            dist_git_branch: "master".to_string(),
            lookaside_cache_url: Some("https://repos.tacos.org.mx/sources".to_string()),
            api_type: ApiType::Forgejo,
            api_url: Some("https://codeberg.org/api/v1".to_string()),
            mock_chroot: Some("tacos-rolling-x86_64".to_string()),
            package_manager: "rpm".to_string(),
            architecture: "x86_64".to_string(),
        }
    }

    /// Resolves a configuration preset from an identifier.
    pub fn from_preset(preset: &str) -> Option<Self> {
        match preset.to_lowercase().replace('_', "-").as_str() {
            "fedora" | "rawhide" | "fedora-rawhide" => Some(Self::fedora_rawhide()),
            "centos" | "c10s" | "centos-10" | "centos-stream-10" => Some(Self::centos_stream_10()),
            "c9s" | "centos-9" | "centos-stream-9" => Some(Self::centos_stream_9()),
            "tacos" | "tacos-rolling" => Some(Self::tacos_rolling()),
            _ => None,
        }
    }

    /// Lists all built-in distribution presets.
    pub fn all_presets() -> Vec<Self> {
        vec![
            Self::fedora_rawhide(),
            Self::centos_stream_10(),
            Self::centos_stream_9(),
            Self::tacos_rolling(),
        ]
    }
}
