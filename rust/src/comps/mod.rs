//! Comps group and environment resolution for Distribution Build System (DBS).
//!
//! Provides inspection, dependency expansion, and package target resolution
//! for Fedora / ELN / TacOS comps environments and groups (e.g. `@workstation-product-environment`, `@core`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use eyre::{eyre, Result};

/// Represents the classification of a comps target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompsKind {
    /// An environment comprising multiple required and optional groups (e.g. `@workstation-product-environment`).
    Environment,
    /// A group comprising mandatory, default, and optional packages (e.g. `@core`).
    Group,
}

impl std::fmt::Display for CompsKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Environment => write!(f, "Environment"),
            Self::Group => write!(f, "Group"),
        }
    }
}

/// Metadata and constituent group listing for a comps environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvironmentInfo {
    /// Machine identifier of the environment (e.g. "workstation-product-environment").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Detailed description of the environment.
    pub description: String,
    /// Group identifiers required by this environment.
    pub required_groups: Vec<String>,
    /// Optional group identifiers associated with this environment.
    pub optional_groups: Vec<String>,
}

/// Metadata and package lists for an individual comps group.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupInfo {
    /// Machine identifier of the group (e.g. "core", "gnome-desktop").
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Detailed description of the group.
    pub description: String,
    /// Mandatory package names that must be installed for this group.
    pub mandatory_packages: Vec<String>,
    /// Default package names included unless specifically excluded.
    pub default_packages: Vec<String>,
    /// Optional package names that can be included on demand.
    pub optional_packages: Vec<String>,
}

/// Comprehensive inspection report for a comps target.
#[derive(Debug, Clone)]
pub struct CompsInspection {
    /// Original target identifier queried (e.g. "@workstation-product-environment").
    pub target: String,
    /// Whether target is an Environment or Group.
    pub kind: CompsKind,
    /// Resolved identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Constituent groups included in this target.
    pub groups: Vec<GroupInfo>,
    /// Deduplicated and sorted list of all resolved packages.
    pub all_packages: Vec<String>,
    /// Count of mandatory packages.
    pub mandatory_count: usize,
    /// Count of default packages.
    pub default_count: usize,
    /// Count of optional packages.
    pub optional_count: usize,
    /// Packages present in the local dist-git directory.
    pub local_present_packages: Vec<String>,
    /// Packages missing from the local dist-git directory.
    pub local_missing_packages: Vec<String>,
}

/// Determines whether a string target represents a comps environment or group.
///
/// Targets starting with `@` (e.g. `@core`, `@workstation-product-environment`)
/// or containing XML `<package name="@...">` elements are identified as comps targets.
pub fn is_comps_target(target: &str) -> bool {
    let clean = extract_package_entry(target).trim();
    clean.starts_with('@')
}

/// Strips leading `@` and any surrounding quotes or whitespace from a comps target string.
pub fn clean_target_name(target: &str) -> &str {
    let extracted = extract_package_entry(target).trim();
    let stripped = match extracted.strip_prefix('@') {
        Some(s) => s,
        None => extracted,
    };
    stripped.trim_matches(|c| c == '"' || c == '\'' || char::is_whitespace(c))
}

/// Extracts a package or comps target identifier from a string, handling XML `<package name="..."/>` tags if present.
pub fn extract_package_entry(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(pos) = trimmed.find("<package") {
        let remainder = &trimmed[pos + 8..];
        if let Some(name_pos) = remainder.find("name=") {
            let val_part = &remainder[name_pos + 5..];
            let quote = match val_part.chars().next() {
                Some(q) if q == '"' || q == '\'' => q,
                _ => return trimmed,
            };
            let inside = &val_part[1..];
            if let Some(end_quote) = inside.find(quote) {
                return inside[..end_quote].trim();
            }
        }
    }
    trimmed
}

/// Resolver for Fedora / ELN / TacOS comps metadata via `dnf5`.
#[derive(Debug, Clone, Default)]
pub struct CompsResolver {
    /// Optional releasever override (e.g. "eln", "rawhide").
    pub releasever: Option<String>,
}

impl CompsResolver {
    /// Creates a new `CompsResolver` using default host repositories.
    pub fn new() -> Self {
        Self { releasever: None }
    }

    /// Sets an optional releasever for package manager queries.
    pub fn with_releasever(mut self, ver: impl Into<String>) -> Self {
        self.releasever = Some(ver.into());
        self
    }

    /// Executes `dnf5` command with `LC_ALL=C` and cache-only preference.
    fn run_dnf5_command(&self, args: &[&str]) -> Result<String> {
        // First try with -C (cache-only) for instant response
        let mut cache_cmd = Command::new("dnf5");
        cache_cmd.env("LC_ALL", "C");
        cache_cmd.arg("-C");
        if let Some(ref ver) = self.releasever {
            cache_cmd.arg(format!("--releasever={}", ver));
        }
        cache_cmd.args(args);

        let cache_output = match cache_cmd.output() {
            Ok(out) => out,
            Err(e) => {
                return Err(eyre!(
                    "Failed to execute 'dnf5': {}. Ensure dnf5 is installed and in PATH.",
                    e
                ));
            }
        };

        let stdout = String::from_utf8_lossy(&cache_output.stdout).to_string();
        if cache_output.status.success() && !stdout.contains("No matches found.") {
            return Ok(stdout);
        }

        // Fallback: run without -C if cache-only produced no matches or error
        let mut live_cmd = Command::new("dnf5");
        live_cmd.env("LC_ALL", "C");
        if let Some(ref ver) = self.releasever {
            live_cmd.arg(format!("--releasever={}", ver));
        }
        live_cmd.args(args);

        let live_output = match live_cmd.output() {
            Ok(out) => out,
            Err(e) => {
                return Err(eyre!("Failed to execute 'dnf5': {}", e));
            }
        };

        if !live_output.status.success() {
            let stderr = String::from_utf8_lossy(&live_output.stderr);
            return Err(eyre!(
                "dnf5 command failed with status {}: {}",
                live_output.status,
                stderr.trim()
            ));
        }

        Ok(String::from_utf8_lossy(&live_output.stdout).to_string())
    }

    /// Queries information for a specific comps environment.
    pub fn resolve_environment(&self, env_id: &str) -> Result<Option<EnvironmentInfo>> {
        let clean = clean_target_name(env_id);
        let output = match self.run_dnf5_command(&["environment", "info", clean]) {
            Ok(out) => out,
            Err(e) => return Err(eyre!("Failed to query environment info for '{}': {}", clean, e)),
        };

        if output.contains("No matches found.") {
            return Ok(None);
        }

        let parsed = parse_environment_info_output(&output);
        let matching = parsed.into_iter().find(|e| e.id.eq_ignore_ascii_case(clean));
        Ok(matching)
    }

    /// Queries information for one or more comps groups in a single batch operation.
    pub fn resolve_groups(&self, group_ids: &[&str]) -> Result<Vec<GroupInfo>> {
        if group_ids.is_empty() {
            return Ok(Vec::new());
        }

        let cleaned: Vec<&str> = group_ids.iter().map(|g| clean_target_name(g)).collect();
        let mut args = vec!["group", "info"];
        args.extend_from_slice(&cleaned);

        let output = match self.run_dnf5_command(&args) {
            Ok(out) => out,
            Err(e) => {
                return Err(eyre!(
                    "Failed to query group info for groups {:?}: {}",
                    cleaned,
                    e
                ));
            }
        };

        if output.contains("No matches found.") && !output.contains("Id                   :") {
            return Ok(Vec::new());
        }

        Ok(parse_group_info_output(&output))
    }

    /// Resolves all package names belonging to a comps target (environment or group).
    ///
    /// By default includes mandatory and default packages. If `include_optional` is true,
    /// also includes optional packages. Returns a deduplicated and sorted vector of package names.
    pub fn resolve_comps_target(&self, target: &str, include_optional: bool) -> Result<Vec<String>> {
        let clean = clean_target_name(target);

        // 1. Check if target is an environment
        match self.resolve_environment(clean) {
            Ok(Some(env)) => {
                let mut group_ids: Vec<&str> = env.required_groups.iter().map(|s| s.as_str()).collect();
                if include_optional {
                    group_ids.extend(env.optional_groups.iter().map(|s| s.as_str()));
                }

                let groups = match self.resolve_groups(&group_ids) {
                    Ok(g) => g,
                    Err(e) => {
                        return Err(eyre!(
                            "Failed to resolve constituent groups for environment '{}': {}",
                            clean,
                            e
                        ));
                    }
                };

                let mut packages = HashSet::new();
                for g in groups {
                    for p in g.mandatory_packages {
                        packages.insert(p);
                    }
                    for p in g.default_packages {
                        packages.insert(p);
                    }
                    if include_optional {
                        for p in g.optional_packages {
                            packages.insert(p);
                        }
                    }
                }

                let mut sorted: Vec<String> = packages.into_iter().collect();
                sorted.sort();
                return Ok(sorted);
            }
            Ok(None) => {}
            Err(e) => {
                return Err(eyre!("Error checking environment '{}': {}", clean, e));
            }
        }

        // 2. Check if target is a group
        let groups = match self.resolve_groups(&[clean]) {
            Ok(g) => g,
            Err(e) => return Err(eyre!("Error checking group '{}': {}", clean, e)),
        };

        if let Some(group) = groups.into_iter().find(|g| g.id.eq_ignore_ascii_case(clean)) {
            let mut packages = HashSet::new();
            for p in group.mandatory_packages {
                packages.insert(p);
            }
            for p in group.default_packages {
                packages.insert(p);
            }
            if include_optional {
                for p in group.optional_packages {
                    packages.insert(p);
                }
            }

            let mut sorted: Vec<String> = packages.into_iter().collect();
            sorted.sort();
            return Ok(sorted);
        }

        Err(eyre!(
            "Target '@{}' is neither a valid comps environment nor a group in active repositories",
            clean
        ))
    }

    /// Performs full inspection of a comps target, reporting metadata, groups, packages, and local dist-git status.
    pub fn inspect_target(
        &self,
        target: &str,
        include_optional: bool,
        distgit_dest: Option<&Path>,
    ) -> Result<CompsInspection> {
        let clean = clean_target_name(target);

        // Check if environment
        let maybe_env = match self.resolve_environment(clean) {
            Ok(opt) => opt,
            Err(e) => return Err(eyre!("Failed inspecting target '{}': {}", clean, e)),
        };

        if let Some(env) = maybe_env {
            let mut group_ids: Vec<&str> = env.required_groups.iter().map(|s| s.as_str()).collect();
            if include_optional {
                group_ids.extend(env.optional_groups.iter().map(|s| s.as_str()));
            }

            let groups = match self.resolve_groups(&group_ids) {
                Ok(g) => g,
                Err(e) => {
                    return Err(eyre!(
                        "Failed resolving groups for environment '{}': {}",
                        clean,
                        e
                    ));
                }
            };

            let mut all_set = HashSet::new();
            let mut mandatory_set = HashSet::new();
            let mut default_set = HashSet::new();
            let mut optional_set = HashSet::new();

            for g in &groups {
                for p in &g.mandatory_packages {
                    all_set.insert(p.clone());
                    mandatory_set.insert(p.clone());
                }
                for p in &g.default_packages {
                    all_set.insert(p.clone());
                    default_set.insert(p.clone());
                }
                for p in &g.optional_packages {
                    optional_set.insert(p.clone());
                    if include_optional {
                        all_set.insert(p.clone());
                    }
                }
            }

            let mut all_packages: Vec<String> = all_set.into_iter().collect();
            all_packages.sort();

            let (present, missing) = match distgit_dest {
                Some(dest) => partition_local_packages(&all_packages, dest, self.releasever.as_deref()),
                None => (Vec::new(), Vec::new()),
            };

            return Ok(CompsInspection {
                target: target.to_string(),
                kind: CompsKind::Environment,
                id: env.id,
                name: env.name,
                description: env.description,
                groups,
                all_packages,
                mandatory_count: mandatory_set.len(),
                default_count: default_set.len(),
                optional_count: optional_set.len(),
                local_present_packages: present,
                local_missing_packages: missing,
            });
        }

        // Check if group
        let groups = match self.resolve_groups(&[clean]) {
            Ok(g) => g,
            Err(e) => return Err(eyre!("Failed inspecting group '{}': {}", clean, e)),
        };

        if let Some(group) = groups.into_iter().find(|g| g.id.eq_ignore_ascii_case(clean)) {
            let mut all_set = HashSet::new();
            let mut mandatory_set = HashSet::new();
            let mut default_set = HashSet::new();
            let mut optional_set = HashSet::new();

            for p in &group.mandatory_packages {
                all_set.insert(p.clone());
                mandatory_set.insert(p.clone());
            }
            for p in &group.default_packages {
                all_set.insert(p.clone());
                default_set.insert(p.clone());
            }
            for p in &group.optional_packages {
                optional_set.insert(p.clone());
                if include_optional {
                    all_set.insert(p.clone());
                }
            }

            let mut all_packages: Vec<String> = all_set.into_iter().collect();
            all_packages.sort();

            let (present, missing) = match distgit_dest {
                Some(dest) => partition_local_packages(&all_packages, dest, self.releasever.as_deref()),
                None => (Vec::new(), Vec::new()),
            };

            let id = group.id.clone();
            let name = group.name.clone();
            let description = group.description.clone();

            return Ok(CompsInspection {
                target: target.to_string(),
                kind: CompsKind::Group,
                id,
                name,
                description,
                groups: vec![group],
                all_packages,
                mandatory_count: mandatory_set.len(),
                default_count: default_set.len(),
                optional_count: optional_set.len(),
                local_present_packages: present,
                local_missing_packages: missing,
            });
        }

        Err(eyre!(
            "Target '@{}' not found as an environment or group in comps metadata",
            clean
        ))
    }

    /// Lists all available comps environments.
    pub fn list_environments(&self) -> Result<Vec<(String, String)>> {
        let output = match self.run_dnf5_command(&["environment", "list"]) {
            Ok(out) => out,
            Err(e) => return Err(eyre!("Failed to list environments: {}", e)),
        };
        Ok(parse_environment_or_group_list(&output))
    }

    /// Lists all available comps groups (including hidden groups).
    pub fn list_groups(&self) -> Result<Vec<(String, String)>> {
        let output = match self.run_dnf5_command(&["group", "list", "--hidden"]) {
            Ok(out) => out,
            Err(e) => return Err(eyre!("Failed to list groups: {}", e)),
        };
        Ok(parse_environment_or_group_list(&output))
    }
}

/// Extracts package names, subpackage names, and provides capabilities from an RPM .spec file.
pub fn extract_spec_provided_packages(spec_path: &Path) -> Vec<String> {
    let content = match std::fs::read_to_string(spec_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut provided = Vec::new();
    let mut main_name = String::new();

    // Pass 1: detect main package Name:
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.to_ascii_lowercase().starts_with("name:")
            && let Some((_, val)) = trimmed.split_once(':') {
                let name = val.split_whitespace().next().unwrap_or("").trim();
                let clean_name = name.split('%').next().unwrap_or(name).trim();
                if !clean_name.is_empty() {
                    main_name = clean_name.to_string();
                    provided.push(main_name.clone());
                    break;
                }
            }
    }

    if main_name.is_empty()
        && let Some(stem) = spec_path.file_stem().and_then(|s| s.to_str()) {
            main_name = stem.to_string();
            provided.push(main_name.clone());
        }

    // Pass 2: extract %package and Provides:
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with("%package") {
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            if tokens.len() >= 2 {
                if tokens[1] == "-n" {
                    if tokens.len() >= 3 {
                        let subpkg = tokens[2]
                            .replace("%{name}", &main_name)
                            .replace("%name", &main_name);
                        let clean = subpkg.trim();
                        if !clean.is_empty() && !clean.contains('%') {
                            provided.push(clean.to_string());
                        }
                    }
                } else if !tokens[1].starts_with('-') {
                    let subname = tokens[1]
                        .replace("%{name}", &main_name)
                        .replace("%name", &main_name);
                    let clean = subname.trim();
                    if !clean.is_empty() && !clean.contains('%') {
                        provided.push(format!("{}-{}", main_name, clean));
                    }
                }
            }
        } else if trimmed.to_ascii_lowercase().starts_with("provides:")
            && let Some((_, val)) = trimmed.split_once(':') {
                let cap = val.split_whitespace().next().unwrap_or("").trim();
                let clean_cap = cap.split('%').next().unwrap_or(cap).trim();
                if !clean_cap.is_empty()
                    && !clean_cap.starts_with('/')
                    && !clean_cap.contains('(')
                    && !clean_cap.contains(')')
                    && !clean_cap.contains('=')
                    && !clean_cap.contains('<')
                    && !clean_cap.contains('>')
                {
                    provided.push(clean_cap.to_string());
                }
            }
    }

    provided.dedup();
    provided
}

/// Scans the local dist-git directory and specs directory, mapping all declared package names,
/// subpackages, and provides capabilities to their parent .spec file path.
pub fn scan_local_spec_providers(distgit_dest: &Path) -> HashMap<String, PathBuf> {
    let mut providers = HashMap::new();

    // Helper closure to index a spec file
    let mut index_spec = |spec_path: PathBuf, dir_alias: Option<&str>| {
        if !spec_path.is_file() {
            return;
        }
        if let Some(stem) = spec_path.file_stem().and_then(|s| s.to_str()) {
            providers.insert(stem.to_string(), spec_path.clone());
        }
        if let Some(alias) = dir_alias
            && !alias.is_empty() {
                providers.insert(alias.to_string(), spec_path.clone());
            }
        for pkg_name in extract_spec_provided_packages(&spec_path) {
            providers.insert(pkg_name, spec_path.clone());
        }
    };

    // 1. Scan direct subdirectories in distgit_dest
    if distgit_dest.is_dir()
        && let Ok(entries) = std::fs::read_dir(distgit_dest) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let spec_candidate = path.join(format!("{}.spec", dir_name));
                    if spec_candidate.is_file() {
                        index_spec(spec_candidate, Some(dir_name));
                    } else if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.extension().and_then(|e| e.to_str()) == Some("spec") {
                                index_spec(sub_path, Some(dir_name));
                                break;
                            }
                        }
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("spec") {
                    index_spec(path, None);
                }
            }
        }

    // 2. Scan specs/ directory if present
    let specs_dir = Path::new("specs");
    if specs_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(specs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let spec_candidate = path.join(format!("{}.spec", dir_name));
                    if spec_candidate.is_file() {
                        index_spec(spec_candidate, Some(dir_name));
                    } else if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.extension().and_then(|e| e.to_str()) == Some("spec") {
                                index_spec(sub_path, Some(dir_name));
                                break;
                            }
                        }
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("spec") {
                    index_spec(path, None);
                }
            }
        }

    providers
}

/// Queries dnf5 repoquery to resolve binary package names to their canonical source package (SRPM) names.
///
/// Returns a map of `binary_package_name -> source_package_name`.
/// Executes in batches of 100 packages using cache-only `-C` first, falling back to live repositories if needed.
pub fn query_source_package_names(packages: &[String], releasever: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if packages.is_empty() {
        return map;
    }

    for chunk in packages.chunks(100) {
        let mut cache_cmd = Command::new("dnf5");
        cache_cmd.env("LC_ALL", "C");
        cache_cmd.arg("-C");
        cache_cmd.arg("-q");
        if let Some(ver) = releasever {
            cache_cmd.arg(format!("--releasever={}", ver));
        }
        cache_cmd.arg("repoquery");
        cache_cmd.arg("--queryformat");
        cache_cmd.arg("%{name}|%{source_name}\n");
        for pkg in chunk {
            cache_cmd.arg(pkg);
        }

        let output = match cache_cmd.output() {
            Ok(out) if out.status.success() && !out.stdout.is_empty() => {
                String::from_utf8_lossy(&out.stdout).to_string()
            }
            _ => {
                let mut live_cmd = Command::new("dnf5");
                live_cmd.env("LC_ALL", "C");
                live_cmd.arg("-q");
                if let Some(ver) = releasever {
                    live_cmd.arg(format!("--releasever={}", ver));
                }
                live_cmd.arg("repoquery");
                live_cmd.arg("--queryformat");
                live_cmd.arg("%{name}|%{source_name}\n");
                for pkg in chunk {
                    live_cmd.arg(pkg);
                }
                match live_cmd.output() {
                    Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
                    _ => String::new(),
                }
            }
        };

        for line in output.lines() {
            let trimmed = line.trim();
            if let Some((bin_name, src_name)) = trimmed.split_once('|') {
                let bin = bin_name.trim();
                let src = src_name.trim();
                if !bin.is_empty() && !src.is_empty() {
                    map.insert(bin.to_string(), src.to_string());
                }
            }
        }
    }

    map
}

/// Checks which packages exist in the local dist-git root directory.
///
/// Resolves both direct package names and binary subpackages:
/// 1. Direct directory / spec file check.
/// 2. Local spec inspection (identifying %package [-n] and Provides declarations).
/// 3. DNF5 repoquery source package mapping: if a binary subpackage's source RPM
///    (e.g. linux-firmware for intel-vsc-firmware) is present locally, the package is marked present.
pub fn partition_local_packages(
    packages: &[String],
    distgit_dest: &Path,
    releasever: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let mut present = Vec::new();
    let mut candidate_missing = Vec::new();

    // 1. Scan local specs in distgit_dest to map all declared binary/subpackages to local specs
    let local_providers = scan_local_spec_providers(distgit_dest);

    for pkg in packages {
        let pkg_dir = distgit_dest.join(pkg);
        let pkg_spec = distgit_dest.join(format!("{}.spec", pkg));
        let direct_spec = Path::new("specs").join(format!("{}.spec", pkg));
        let in_specs_dir = Path::new("specs").join(pkg);

        if pkg_dir.is_dir()
            || pkg_spec.is_file()
            || direct_spec.is_file()
            || in_specs_dir.is_dir()
            || local_providers.contains_key(pkg)
        {
            present.push(pkg.clone());
        } else {
            candidate_missing.push(pkg.clone());
        }
    }

    if candidate_missing.is_empty() {
        return (present, Vec::new());
    }

    // 2. For remaining candidates, query dnf5 to find their parent source package (SRPM) names
    let src_map = query_source_package_names(&candidate_missing, releasever);
    let mut missing = Vec::new();

    for pkg in candidate_missing {
        if let Some(src_name) = src_map.get(&pkg) {
            let src_dir = distgit_dest.join(src_name);
            let src_spec = distgit_dest.join(format!("{}.spec", src_name));
            let direct_src_spec = Path::new("specs").join(format!("{}.spec", src_name));
            let in_specs_src_dir = Path::new("specs").join(src_name);

            if src_dir.is_dir()
                || src_spec.is_file()
                || direct_src_spec.is_file()
                || in_specs_src_dir.is_dir()
                || local_providers.contains_key(src_name)
            {
                present.push(pkg);
                continue;
            }
        }
        missing.push(pkg);
    }

    (present, missing)
}

/// Section tracker for parsing `dnf5 environment info` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvSection {
    None,
    RequiredGroups,
    OptionalGroups,
    Other,
}

/// Parses output from `dnf5 environment info`.
pub fn parse_environment_info_output(output: &str) -> Vec<EnvironmentInfo> {
    let mut results = Vec::new();
    let mut current = EnvironmentInfo::default();
    let mut section = EnvSection::None;
    let mut in_env = false;

    for line in output.lines() {
        if let Some((raw_key, raw_val)) = line.split_once(':') {
            let key = raw_key.trim();
            let val = raw_val.trim();

            if key == "Id" {
                if in_env && !current.id.is_empty() {
                    results.push(current);
                    current = EnvironmentInfo::default();
                }
                current.id = val.to_string();
                in_env = true;
                section = EnvSection::None;
            } else if !in_env {
                continue;
            } else if key == "Name" {
                current.name = val.to_string();
                section = EnvSection::None;
            } else if key == "Description" {
                current.description = val.to_string();
                section = EnvSection::None;
            } else if key == "Required groups" {
                section = EnvSection::RequiredGroups;
                if !val.is_empty() {
                    current.required_groups.push(val.to_string());
                }
            } else if key == "Optional groups" {
                section = EnvSection::OptionalGroups;
                if !val.is_empty() {
                    current.optional_groups.push(val.to_string());
                }
            } else if key.is_empty() {
                // Continuation line of previous list field
                if !val.is_empty() {
                    match section {
                        EnvSection::RequiredGroups => current.required_groups.push(val.to_string()),
                        EnvSection::OptionalGroups => current.optional_groups.push(val.to_string()),
                        _ => {}
                    }
                }
            } else {
                section = EnvSection::Other;
            }
        }
    }

    if in_env && !current.id.is_empty() {
        results.push(current);
    }

    results
}

/// Section tracker for parsing `dnf5 group info` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupSection {
    None,
    Mandatory,
    Default,
    Optional,
    Other,
}

/// Parses output from `dnf5 group info`.
pub fn parse_group_info_output(output: &str) -> Vec<GroupInfo> {
    let mut results = Vec::new();
    let mut current = GroupInfo::default();
    let mut section = GroupSection::None;
    let mut in_group = false;

    for line in output.lines() {
        if let Some((raw_key, raw_val)) = line.split_once(':') {
            let key = raw_key.trim();
            let val = raw_val.trim();

            if key == "Id" {
                if in_group && !current.id.is_empty() {
                    results.push(current);
                    current = GroupInfo::default();
                }
                current.id = val.to_string();
                in_group = true;
                section = GroupSection::None;
            } else if !in_group {
                continue;
            } else if key == "Name" {
                current.name = val.to_string();
                section = GroupSection::None;
            } else if key == "Description" {
                current.description = val.to_string();
                section = GroupSection::None;
            } else if key == "Mandatory packages" {
                section = GroupSection::Mandatory;
                if !val.is_empty() {
                    current.mandatory_packages.push(val.to_string());
                }
            } else if key == "Default packages" {
                section = GroupSection::Default;
                if !val.is_empty() {
                    current.default_packages.push(val.to_string());
                }
            } else if key == "Optional packages" {
                section = GroupSection::Optional;
                if !val.is_empty() {
                    current.optional_packages.push(val.to_string());
                }
            } else if key.is_empty() {
                // Continuation line
                if !val.is_empty() {
                    match section {
                        GroupSection::Mandatory => current.mandatory_packages.push(val.to_string()),
                        GroupSection::Default => current.default_packages.push(val.to_string()),
                        GroupSection::Optional => current.optional_packages.push(val.to_string()),
                        _ => {}
                    }
                }
            } else {
                section = GroupSection::Other;
            }
        }
    }

    if in_group && !current.id.is_empty() {
        results.push(current);
    }

    results
}

/// Parses ID and Name from `dnf5 environment list` or `dnf5 group list`.
fn parse_environment_or_group_list(output: &str) -> Vec<(String, String)> {
    let mut list = Vec::new();
    let mut header_seen = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ID") && trimmed.contains("Name") {
            header_seen = true;
            continue;
        }
        if !header_seen || trimmed.is_empty() || trimmed.starts_with("Groups:") || trimmed.starts_with("Environments:") {
            continue;
        }

        // Columns are whitespace separated: ID, Name (may contain spaces), Installed (yes/no)
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 2 {
            let id = parts[0].to_string();
            // The last word is usually "yes" or "no" (installed status)
            let name_words = if parts.len() > 2 && (parts.last() == Some(&"yes") || parts.last() == Some(&"no")) {
                &parts[1..parts.len() - 1]
            } else {
                &parts[1..]
            };
            let name = name_words.join(" ");
            list.push((id, name));
        }
    }

    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_comps_target_detection() {
        assert!(is_comps_target("@workstation-product-environment"));
        assert!(is_comps_target("@core"));
        assert!(is_comps_target("  @gnome-desktop  "));
        assert!(is_comps_target(r#"<package name="@workstation-product-environment"/>"#));
        assert!(is_comps_target(r#"<package name="@core"/>"#));

        assert!(!is_comps_target("core"));
        assert!(!is_comps_target("vim"));
        assert!(!is_comps_target(r#"<package name="vim"/>"#));
    }

    #[test]
    fn test_clean_target_name() {
        assert_eq!(clean_target_name("@core"), "core");
        assert_eq!(clean_target_name("@workstation-product-environment"), "workstation-product-environment");
        assert_eq!(clean_target_name(r#"<package name="@workstation-product-environment"/>"#), "workstation-product-environment");
        assert_eq!(clean_target_name(r#"<package name="vim"/>"#), "vim");
        assert_eq!(clean_target_name("  @base-graphical  "), "base-graphical");
    }

    #[test]
    fn test_extract_package_entry() {
        assert_eq!(extract_package_entry(r#"<package name="@workstation-product-environment"/>"#), "@workstation-product-environment");
        assert_eq!(extract_package_entry(r#"<package name="vim"/>"#), "vim");
        assert_eq!(extract_package_entry(r#"<package name='bash' arch='x86_64'/>"#), "bash");
        assert_eq!(extract_package_entry("@core"), "@core");
        assert_eq!(extract_package_entry("kernel"), "kernel");
    }

    #[test]
    fn test_parse_environment_info_output() {
        let sample = r#"
Updating and loading repositories:
Repositories loaded.
Id                   : workstation-product-environment
Name                 : Fedora Workstation
Description          : Fedora Workstation is a user friendly desktop system for laptops and PCs.
Order                : 2
Installed            : False
Repositories         : fedora, updates
Required groups      : base-graphical
                     : container-management
                     : core
Optional groups      : domain-client
"#;

        let parsed = parse_environment_info_output(sample);
        assert_eq!(parsed.len(), 1);
        let env = &parsed[0];
        assert_eq!(env.id, "workstation-product-environment");
        assert_eq!(env.name, "Fedora Workstation");
        assert_eq!(env.description, "Fedora Workstation is a user friendly desktop system for laptops and PCs.");
        assert_eq!(env.required_groups, vec!["base-graphical", "container-management", "core"]);
        assert_eq!(env.optional_groups, vec!["domain-client"]);
    }

    #[test]
    fn test_parse_group_info_output() {
        let sample = r#"
Id                   : core
Name                 : Core
Description          : Smallest possible installation
Installed            : no
Repositories         : fedora, updates
Mandatory packages   : audit
                     : bash
                     : coreutils
Default packages     : NetworkManager
                     : firewalld
Optional packages    : initial-setup
                     : initscripts
"#;

        let parsed = parse_group_info_output(sample);
        assert_eq!(parsed.len(), 1);
        let grp = &parsed[0];
        assert_eq!(grp.id, "core");
        assert_eq!(grp.name, "Core");
        assert_eq!(grp.mandatory_packages, vec!["audit", "bash", "coreutils"]);
        assert_eq!(grp.default_packages, vec!["NetworkManager", "firewalld"]);
        assert_eq!(grp.optional_packages, vec!["initial-setup", "initscripts"]);
    }

    #[test]
    fn test_parse_environment_or_group_list() {
        let sample = r#"
Updating and loading repositories:
Repositories loaded.
ID                                Name                                 Installed
custom-environment                Fedora Custom Operating System              no
workstation-product-environment   Fedora Workstation                          no
server-product-environment        Fedora Server Edition                       no
"#;

        let list = parse_environment_or_group_list(sample);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], ("custom-environment".to_string(), "Fedora Custom Operating System".to_string()));
        assert_eq!(list[1], ("workstation-product-environment".to_string(), "Fedora Workstation".to_string()));
        assert_eq!(list[2], ("server-product-environment".to_string(), "Fedora Server Edition".to_string()));
    }

    #[test]
    fn test_live_resolve_comps_target() {
        let resolver = CompsResolver::new();
        // Test resolving @core if dnf5 is present
        if let Ok(packages) = resolver.resolve_comps_target("@core", false) {
            assert!(!packages.is_empty());
            assert!(packages.contains(&"bash".to_string()));
            assert!(packages.contains(&"coreutils".to_string()));
        }
    }

    #[test]
    fn test_extract_spec_provided_packages() {
        let temp = tempfile::tempdir().unwrap();
        let spec_file = temp.path().join("linux-firmware.spec");
        let content = r#"
Name:           linux-firmware
Version:        20240909
Release:        1%{?dist}
Summary:        Firmware files used by the Linux kernel

%package -n intel-vsc-firmware
Summary:        Intel Visual Sensing Controller firmware
Provides:       firmware(intel/vsc) = 1.0

%package which
Summary:        Which subpackage

Provides:       kernel-firmware = %{version}
Provides:       /lib/firmware/test.bin
Provides:       bundled(gnulib)
"#;
        std::fs::write(&spec_file, content).unwrap();

        let provided = extract_spec_provided_packages(&spec_file);
        assert!(provided.contains(&"linux-firmware".to_string()));
        assert!(provided.contains(&"intel-vsc-firmware".to_string()));
        assert!(provided.contains(&"linux-firmware-which".to_string()));
        assert!(provided.contains(&"kernel-firmware".to_string()));
        // Ensure file paths and bundled() expressions are omitted
        assert!(!provided.contains(&"/lib/firmware/test.bin".to_string()));
        assert!(!provided.contains(&"bundled(gnulib)".to_string()));
    }

    #[test]
    fn test_scan_and_partition_local_packages_subpackages() {
        let temp = tempfile::tempdir().unwrap();
        let distgit = temp.path();

        // Create a subfolder with linux-firmware.spec providing intel-vsc-firmware
        let fw_dir = distgit.join("linux-firmware");
        std::fs::create_dir_all(&fw_dir).unwrap();
        let spec_file = fw_dir.join("linux-firmware.spec");
        let content = r#"
Name:           linux-firmware
Version:        20240909
Release:        1

%package -n intel-vsc-firmware
Summary:        Intel VSC
"#;
        std::fs::write(&spec_file, content).unwrap();

        let providers = scan_local_spec_providers(distgit);
        assert_eq!(providers.get("intel-vsc-firmware"), Some(&spec_file));
        assert_eq!(providers.get("linux-firmware"), Some(&spec_file));

        // Test partitioning: intel-vsc-firmware must be present, missing-pkg must be missing
        let pkgs = vec![
            "intel-vsc-firmware".to_string(),
            "nonexistent-pkg-xyz".to_string(),
        ];
        let (present, missing) = partition_local_packages(&pkgs, distgit, None);

        assert!(present.contains(&"intel-vsc-firmware".to_string()));
        assert!(!missing.contains(&"intel-vsc-firmware".to_string()));
        assert!(missing.contains(&"nonexistent-pkg-xyz".to_string()));
    }
}
