//! Pure Rust client for exploring, cloning, and synchronizing dist-git repositories.
//!
//! Replaces legacy external shell and python scripts with asynchronous, parallelized
//! Rust workflows featuring worker pools, automatic branch tracking, and lookaside cache retrieval.

use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::distgit::provider::{ApiType, DistroConfig};
use crate::distgit::spec::{parse_spec_file, SpecMetadata};

/// Discovered package or project from dist-git search or remote API queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredProject {
    /// Package name.
    pub name: String,
    /// Brief description if reported by API.
    pub description: Option<String>,
    /// Web UI URL for project.
    pub web_url: Option<String>,
    /// Direct git clone URL.
    pub git_url: String,
}

/// Status and metadata for a locally synchronized dist-git repository.
#[derive(Debug, Clone)]
pub struct GitRepoStatus {
    /// Package name.
    pub package_name: String,
    /// Local filesystem path to repository.
    pub local_path: PathBuf,
    /// Checked-out branch.
    pub branch: String,
    /// Commit hash of HEAD.
    pub commit_hash: String,
    /// Path to discovered `.spec` file.
    pub spec_path: PathBuf,
    /// Parsed spec file metadata.
    pub spec_meta: SpecMetadata,
    /// Whether the repository was freshly cloned (true) or updated (false).
    pub freshly_cloned: bool,
}

/// Client for interacting with dist-git repositories for a specific distribution.
pub struct DistGitClient {
    /// Target distribution configuration.
    config: DistroConfig,
    /// HTTP client for API and lookaside downloads.
    http: reqwest::Client,
}

impl DistGitClient {
    /// Creates a new `DistGitClient` for the given distribution configuration.
    pub fn new(config: DistroConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent("dbs-distgit/0.1.0 (Distribution Build System; Linux)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { config, http }
    }

    /// Accesses the target distribution configuration.
    pub fn config(&self) -> &DistroConfig {
        &self.config
    }

    /// Queries the remote dist-git API to explore and discover available packages.
    pub async fn explore(&self, search: Option<&str>, limit: Option<usize>) -> Result<Vec<DiscoveredProject>> {
        match self.config.api_type {
            ApiType::Pagure => self.explore_pagure(search, limit).await,
            ApiType::GitLab => self.explore_gitlab(search, limit).await,
            ApiType::Forgejo => self.explore_forgejo(search, limit).await,
            ApiType::Repodata | ApiType::GenericGit => {
                // If direct API is unavailable, synthesize from configured pattern
                if let Some(term) = search {
                    Ok(vec![DiscoveredProject {
                        name: term.to_string(),
                        description: Some(format!("Package in {} dist-git", self.config.name)),
                        web_url: None,
                        git_url: self.config.git_url_for_package(term),
                    }])
                } else {
                    Ok(Vec::new())
                }
            }
        }
    }

    /// Queries Fedora Pagure API (`src.fedoraproject.org`).
    async fn explore_pagure(&self, search: Option<&str>, limit: Option<usize>) -> Result<Vec<DiscoveredProject>> {
        let base_api = match &self.config.api_url {
            Some(u) => u.as_str(),
            None => "https://src.fedoraproject.org/api/0",
        };

        let mut results = Vec::new();
        let mut page = 1;

        loop {
            let per_page = match limit {
                Some(lim) => {
                    let remaining = lim.saturating_sub(results.len());
                    if remaining == 0 {
                        break;
                    }
                    remaining.min(100)
                }
                None => 100,
            };

            let url = format!("{}/projects", base_api);
            let page_str = page.to_string();
            let per_page_str = per_page.to_string();
            let mut req = self.http.get(&url)
                .query(&[
                    ("namespace", "rpms"),
                    ("fork", "false"),
                    ("per_page", per_page_str.as_str()),
                    ("page", page_str.as_str()),
                ]);

            if let Some(pattern) = search {
                req = req.query(&[("pattern", pattern)]);
            }

            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(eyre!("Pagure API error {}: {}", resp.status(), resp.text().await?));
            }

            let body: serde_json::Value = resp.json().await?;
            let projects = match body.get("projects").and_then(|p| p.as_array()) {
                Some(p) => p,
                None => break,
            };

            if projects.is_empty() {
                break;
            }

            for proj in projects {
                if let Some(name) = proj.get("name").and_then(|n| n.as_str()) {
                    let desc = proj.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    let web_url = proj.get("full_url").and_then(|u| u.as_str()).map(|s| s.to_string());
                    let git_url = self.config.git_url_for_package(name);

                    results.push(DiscoveredProject {
                        name: name.to_string(),
                        description: desc,
                        web_url,
                        git_url,
                    });

                    if let Some(max) = limit {
                        if results.len() >= max {
                            return Ok(results);
                        }
                    }
                }
            }

            let has_next = body.get("pagination")
                .and_then(|p| p.get("next"))
                .map(|n| !n.is_null())
                .unwrap_or(false);

            if !has_next {
                break;
            }

            page += 1;
        }

        Ok(results)
    }

    /// Queries CentOS Stream GitLab API (`gitlab.com/redhat/centos-stream/rpms`).
    async fn explore_gitlab(&self, search: Option<&str>, limit: Option<usize>) -> Result<Vec<DiscoveredProject>> {
        let base_api = match &self.config.api_url {
            Some(u) => u.as_str(),
            None => "https://gitlab.com/api/v4/groups/8794173/projects",
        };

        let mut results = Vec::new();
        let mut page = 1;

        loop {
            let per_page = match limit {
                Some(lim) => {
                    let remaining = lim.saturating_sub(results.len());
                    if remaining == 0 {
                        break;
                    }
                    remaining.min(100)
                }
                None => 100,
            };

            let page_str = page.to_string();
            let per_page_str = per_page.to_string();
            let mut req = self.http.get(base_api)
                .query(&[
                    ("with_shared", "false"),
                    ("per_page", per_page_str.as_str()),
                    ("page", page_str.as_str()),
                ]);

            if let Some(pattern) = search {
                req = req.query(&[("search", pattern)]);
            }

            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(eyre!("GitLab API error {}: {}", resp.status(), resp.text().await?));
            }

            let projects: Vec<serde_json::Value> = resp.json().await?;
            if projects.is_empty() {
                break;
            }

            for proj in projects {
                if let Some(name) = proj.get("name").and_then(|n| n.as_str()) {
                    let desc = proj.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    let web_url = proj.get("web_url").and_then(|u| u.as_str()).map(|s| s.to_string());
                    let git_url = proj.get("http_url_to_repo")
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| self.config.git_url_for_package(name));

                    results.push(DiscoveredProject {
                        name: name.to_string(),
                        description: desc,
                        web_url,
                        git_url,
                    });

                    if let Some(max) = limit {
                        if results.len() >= max {
                            return Ok(results);
                        }
                    }
                }
            }

            page += 1;
        }

        Ok(results)
    }

    /// Queries Forgejo / Gitea API.
    async fn explore_forgejo(&self, search: Option<&str>, limit: Option<usize>) -> Result<Vec<DiscoveredProject>> {
        let base_api = match &self.config.api_url {
            Some(u) => u.as_str(),
            None => "https://codeberg.org/api/v1",
        };

        let url = format!("{}/repos/search", base_api);
        let mut results = Vec::new();
        let mut page = 1;

        loop {
            let per_page = match limit {
                Some(lim) => {
                    let remaining = lim.saturating_sub(results.len());
                    if remaining == 0 {
                        break;
                    }
                    remaining.min(50)
                }
                None => 50,
            };

            let page_str = page.to_string();
            let per_page_str = per_page.to_string();
            let mut req = self.http.get(&url)
                .query(&[
                    ("limit", per_page_str.as_str()),
                    ("page", page_str.as_str()),
                ]);

            if let Some(q) = search {
                req = req.query(&[("q", q)]);
            }

            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(eyre!("Forgejo API error {}: {}", resp.status(), resp.text().await?));
            }

            let body: serde_json::Value = resp.json().await?;
            let repos = match body.get("data").and_then(|d| d.as_array()) {
                Some(r) => r,
                None => break,
            };

            if repos.is_empty() {
                break;
            }

            for repo in repos {
                if let Some(name) = repo.get("name").and_then(|n| n.as_str()) {
                    let desc = repo.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                    let web_url = repo.get("html_url").and_then(|u| u.as_str()).map(|s| s.to_string());
                    let git_url = repo.get("clone_url")
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| self.config.git_url_for_package(name));

                    results.push(DiscoveredProject {
                        name: name.to_string(),
                        description: desc,
                        web_url,
                        git_url,
                    });

                    if let Some(max) = limit {
                        if results.len() >= max {
                            return Ok(results);
                        }
                    }
                }
            }

            page += 1;
        }

        Ok(results)
    }

    /// Clones or pulls a single dist-git repository into the destination directory.
    pub fn clone_or_pull(&self, package_name: &str, dest_dir: &Path) -> Result<GitRepoStatus> {
        self.clone_or_pull_as(package_name, package_name, dest_dir, false, None)
    }

    /// Clones or pulls a dist-git repository under a custom target name, with optional
    /// spec renaming and origin remote reconfiguration.
    pub fn clone_or_pull_as(
        &self,
        source_package: &str,
        target_name: &str,
        dest_dir: &Path,
        rename_spec: bool,
        new_origin: Option<&str>,
    ) -> Result<GitRepoStatus> {
        let pkg_dir = dest_dir.join(target_name);
        let git_url = self.config.git_url_for_package(source_package);
        let branch = &self.config.dist_git_branch;

        let freshly_cloned = if pkg_dir.join(".git").exists() {
            // Determine active remote name (origin or upstream)
            let fetch_status = Command::new("git")
                .arg("-C")
                .arg(&pkg_dir)
                .arg("fetch")
                .arg("--depth=1")
                .arg("origin")
                .arg(branch)
                .status();

            let remote_name = match fetch_status {
                Ok(s) if s.success() => "origin",
                _ => {
                    let upstream_status = Command::new("git")
                        .arg("-C")
                        .arg(&pkg_dir)
                        .arg("fetch")
                        .arg("--depth=1")
                        .arg("upstream")
                        .arg(branch)
                        .status()?;
                    if !upstream_status.success() {
                        return Err(eyre!("git fetch failed for {} on branch {}", source_package, branch));
                    }
                    "upstream"
                }
            };

            let reset_status = Command::new("git")
                .arg("-C")
                .arg(&pkg_dir)
                .arg("reset")
                .arg("--hard")
                .arg(format!("{}/{}", remote_name, branch))
                .status()?;

            if !reset_status.success() {
                return Err(eyre!("git reset failed for {} on branch {}", source_package, branch));
            }

            false
        } else {
            // Fresh clone
            if let Some(parent) = pkg_dir.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let clone_status = Command::new("git")
                .arg("clone")
                .arg("--depth=1")
                .arg("--branch")
                .arg(branch)
                .arg(&git_url)
                .arg(&pkg_dir)
                .status()?;

            if !clone_status.success() {
                return Err(eyre!("git clone failed for {} from {}", source_package, git_url));
            }

            true
        };

        // If a new origin was specified, configure origin and upstream remotes
        if let Some(origin_template) = new_origin {
            let final_origin_url = if origin_template.contains("{package}") {
                origin_template.replace("{package}", target_name)
            } else if origin_template.ends_with('/') {
                format!("{}{}.git", origin_template, target_name)
            } else {
                origin_template.to_string()
            };

            Self::configure_remotes(&pkg_dir, &git_url, &final_origin_url)?;
        }

        // Extract commit hash
        let rev_output = Command::new("git")
            .arg("-C")
            .arg(&pkg_dir)
            .arg("rev-parse")
            .arg("HEAD")
            .output()?;

        let commit_hash = String::from_utf8_lossy(&rev_output.stdout).trim().to_string();

        // Locate `.spec` file
        let mut spec_path = self.locate_spec_file(&pkg_dir, source_package)?;

        // Optionally rename `.spec` file to match target_name
        if rename_spec && target_name != source_package {
            let target_spec_path = pkg_dir.join(format!("{}.spec", target_name));
            if spec_path != target_spec_path {
                fs::rename(&spec_path, &target_spec_path)?;
                spec_path = target_spec_path;
            }
        }

        let spec_meta = parse_spec_file(&spec_path)?;

        Ok(GitRepoStatus {
            package_name: target_name.to_string(),
            local_path: pkg_dir,
            branch: branch.clone(),
            commit_hash,
            spec_path,
            spec_meta,
            freshly_cloned,
        })
    }

    /// Synchronizes multiple dist-git repositories in parallel using Tokio tasks and worker throttling,
    /// streaming completed items over a channel with (completed_index, total_count, Result<GitRepoStatus>).
    pub async fn sync_batch_stream(
        self: Arc<Self>,
        packages: Vec<String>,
        dest_dir: PathBuf,
        concurrency: usize,
    ) -> tokio::sync::mpsc::Receiver<(usize, usize, Result<GitRepoStatus>)> {
        let total = packages.len();
        let (tx, rx) = tokio::sync::mpsc::channel(concurrency * 2);
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let completed_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        tokio::spawn(async move {
            for pkg in packages {
                let permit = match semaphore.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break,
                };
                let client = self.clone();
                let dest = dest_dir.clone();
                let tx_clone = tx.clone();
                let counter = completed_counter.clone();

                tokio::task::spawn_blocking(move || {
                    let res = client.clone_or_pull(&pkg, &dest);
                    drop(permit);
                    let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let _ = tx_clone.blocking_send((idx, total, res));
                });
            }
        });

        rx
    }

    /// Synchronizes multiple dist-git repositories in parallel using Tokio tasks and worker throttling.
    pub async fn sync_batch(
        self: Arc<Self>,
        packages: Vec<String>,
        dest_dir: PathBuf,
        concurrency: usize,
    ) -> Vec<Result<GitRepoStatus>> {
        let mut rx = self.sync_batch_stream(packages, dest_dir, concurrency).await;
        let mut results = Vec::new();
        while let Some((_, _, res)) = rx.recv().await {
            results.push(res);
        }
        results
    }

    /// Downloads referenced source tarballs from the lookaside cache if present.
    pub async fn download_lookaside_sources(&self, repo_dir: &Path, package_name: &str) -> Result<Vec<PathBuf>> {
        let sources_file = repo_dir.join("sources");
        if !sources_file.exists() {
            return Ok(Vec::new());
        }

        let cache_base = match &self.config.lookaside_cache_url {
            Some(u) => u.as_str(),
            None => return Ok(Vec::new()),
        };

        let content = fs::read_to_string(&sources_file)?;
        let mut downloaded = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Fedora format: SHA512 (filename) = hash
            // Legacy format: hash  filename
            let (filename, hash) = if let Some(m) = line.strip_prefix("SHA512 (") {
                if let Some((fname, h)) = m.split_once(") = ") {
                    (fname.trim(), h.trim())
                } else {
                    continue;
                }
            } else if let Some((h, fname)) = line.split_once([' ', '\t']) {
                (fname.trim(), h.trim())
            } else {
                continue;
            };

            let dest_file = repo_dir.join(filename);
            if dest_file.exists() {
                downloaded.push(dest_file);
                continue;
            }

            let source_url = match self.config.api_type {
                ApiType::Pagure => {
                    format!("{}/{}/{}/sha512/{}/{}", cache_base, package_name, filename, hash, filename)
                }
                ApiType::GitLab => {
                    format!("{}/{}/{}/sha512/{}/{}", cache_base, package_name, filename, hash, filename)
                }
                _ => {
                    format!("{}/{}/{}", cache_base, package_name, filename)
                }
            };

            let resp = self.http.get(&source_url).send().await?;
            if !resp.status().is_success() {
                return Err(eyre!("Failed to download {} from lookaside: {}", filename, resp.status()));
            }

            let bytes = resp.bytes().await?;
            let mut file = File::create(&dest_file)?;
            let mut cursor = std::io::Cursor::new(bytes);
            copy(&mut cursor, &mut file)?;

            downloaded.push(dest_file);
        }

        Ok(downloaded)
    }

    /// Finds the `.spec` file in a cloned repository.
    fn locate_spec_file(&self, pkg_dir: &Path, package_name: &str) -> Result<PathBuf> {
        let expected = pkg_dir.join(format!("{}.spec", package_name));
        if expected.exists() {
            return Ok(expected);
        }

        // Search directory for any file ending in .spec
        if let Ok(entries) = fs::read_dir(pkg_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("spec") {
                    return Ok(path);
                }
            }
        }

        Err(eyre!("No .spec file found in {}", pkg_dir.display()))
    }

    /// Reconfigures git remotes so `upstream` points to the source dist-git (e.g. Fedora Rawhide)
    /// and `origin` points to the new distribution repository (e.g. TacOS on Codeberg/Forgejo).
    pub fn configure_remotes(pkg_dir: &Path, upstream_url: &str, new_origin_url: &str) -> Result<()> {
        let remotes_output = Command::new("git")
            .arg("-C")
            .arg(pkg_dir)
            .arg("remote")
            .output()?;

        let remotes_str = String::from_utf8_lossy(&remotes_output.stdout);
        let remotes: Vec<&str> = remotes_str.lines().map(|l| l.trim()).collect();

        if !remotes.contains(&"upstream") {
            if remotes.contains(&"origin") {
                let _ = Command::new("git")
                    .arg("-C")
                    .arg(pkg_dir)
                    .arg("remote")
                    .arg("rename")
                    .arg("origin")
                    .arg("upstream")
                    .status();
            } else {
                let _ = Command::new("git")
                    .arg("-C")
                    .arg(pkg_dir)
                    .arg("remote")
                    .arg("add")
                    .arg("upstream")
                    .arg(upstream_url)
                    .status();
            }
        }

        // Query remotes to cleanly set-url or add 'origin'
        let remotes_current = Command::new("git")
            .arg("-C")
            .arg(pkg_dir)
            .arg("remote")
            .output()?;
        let remotes_current_str = String::from_utf8_lossy(&remotes_current.stdout);
        let current_remotes: Vec<&str> = remotes_current_str.lines().map(|l| l.trim()).collect();

        if current_remotes.contains(&"origin") {
            let _ = Command::new("git")
                .arg("-C")
                .arg(pkg_dir)
                .arg("remote")
                .arg("set-url")
                .arg("origin")
                .arg(new_origin_url)
                .status();
        } else {
            let _ = Command::new("git")
                .arg("-C")
                .arg(pkg_dir)
                .arg("remote")
                .arg("add")
                .arg("origin")
                .arg(new_origin_url)
                .status();
        }

        Ok(())
    }
}
