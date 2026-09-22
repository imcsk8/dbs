//! Mock chroot configuration management, discovery, validation, and resolution.
//!
//! Provides first-class support for discovering and using custom chroot configurations
//! (such as `tacos-rolling-x86_64.cfg` with nested templates) across project directories,
//! workspace paths, user configurations, and system defaults.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use eyre::{eyre, Result};

/// Information about a resolved Mock chroot configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedChroot {
    /// Mock chroot profile name (e.g. `tacos-rolling-x86_64`, `fedora-rawhide-x86_64`).
    pub profile_name: String,
    /// Directory containing the chroot configuration files (if custom).
    pub config_dir: Option<PathBuf>,
    /// Full path to the resolved `.cfg` file.
    pub config_path: PathBuf,
}

/// Metadata and status parsed from a Mock chroot configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChrootConfig {
    /// Profile identifier / file stem (e.g. `tacos-rolling-x86_64`).
    pub name: String,
    /// Absolute path to the `.cfg` file.
    pub path: PathBuf,
    /// Directory containing the configuration.
    pub config_dir: PathBuf,
    /// Target architecture (e.g. `x86_64`, `aarch64`).
    pub target_arch: Option<String>,
    /// Package manager engine (e.g. `dnf5`, `dnf`, `yum`).
    pub package_manager: Option<String>,
    /// Distribution release version (e.g. `46`, `10`, `rawhide`).
    pub releasever: Option<String>,
    /// Distribution tag macro (e.g. `.tcrs`, `.fc42`, `.el10`).
    pub dist: Option<String>,
    /// Vendor name macro (e.g. `TacOS`, `Fedora Project`).
    pub vendor: Option<String>,
    /// OCI or base container image used for bootstrapping.
    pub bootstrap_image: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// List of included template paths (relative to configdir).
    pub includes: Vec<String>,
    /// Whether all template dependencies exist and are readable.
    pub valid: bool,
    /// Description of missing templates or validation errors.
    pub validation_error: Option<String>,
}

/// Diagnostic report generated when validating a chroot configuration.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// Resolved chroot configuration metadata.
    pub config: ChrootConfig,
    /// Whether Mock was able to load and parse the configuration.
    pub mock_verified: bool,
    /// Mock root filesystem path reported by `mock --print-root-path`.
    pub mock_root_path: Option<String>,
    /// Error message from Mock if validation failed.
    pub error_message: Option<String>,
}

/// Chroot configuration resolver and manager.
pub struct ChrootResolver;

impl ChrootResolver {
    /// Returns the standard search paths for Mock configuration directories in priority order.
    pub fn default_search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // 1. Environment variable overrides
        if let Ok(env_dir) = std::env::var("DBS_MOCK_CONFIG_DIR") {
            paths.push(PathBuf::from(env_dir));
        }
        if let Ok(env_dir) = std::env::var("MOCK_CONFIG_DIR") {
            paths.push(PathBuf::from(env_dir));
        }

        // 2. Current workspace local mock directories
        paths.push(PathBuf::from("mock"));
        paths.push(PathBuf::from("data/mock"));

        // 3. Known project / sibling workspace directories
        paths.push(PathBuf::from("../tacos/mock"));
        paths.push(PathBuf::from("/home/imcsk8/projects/gemini-workdir/tacos/mock"));

        // 4. User configuration directory ($HOME/.config/mock and $HOME/.config/dbs/mock)
        if let Ok(home) = std::env::var("HOME") {
            let user_home = PathBuf::from(home);
            paths.push(user_home.join(".config").join("dbs").join("mock"));
            paths.push(user_home.join(".config").join("mock"));
        }

        // 5. System directory
        paths.push(PathBuf::from("/etc/mock"));

        paths
    }

    /// Resolves a Mock chroot profile name or file path into a canonical [`ResolvedChroot`].
    ///
    /// Handles:
    /// - Direct `.cfg` file paths (e.g. `/home/user/tacos/mock/tacos-rolling-x86_64.cfg` or `./mock/tacos.cfg`)
    /// - Named profiles (e.g. `tacos-rolling-x86_64`, `fedora-rawhide-x86_64`)
    /// - Explicit custom configuration directory override
    pub fn resolve(root_spec: &str, explicit_config_dir: Option<&Path>) -> Result<ResolvedChroot> {
        let spec_path = Path::new(root_spec);

        // Case 1: Direct file path ending in .cfg or existing on disk
        if root_spec.ends_with(".cfg") || spec_path.is_file() {
            let canonical_path = match fs::canonicalize(spec_path) {
                Ok(p) => p,
                Err(e) => {
                    return Err(eyre!(
                        "Chroot configuration file '{}' could not be resolved: {}",
                        root_spec,
                        e
                    ));
                }
            };

            let config_dir = match explicit_config_dir {
                Some(dir) => {
                    let can_dir = match fs::canonicalize(dir) {
                        Ok(p) => p,
                        Err(e) => return Err(eyre!("Custom config directory '{}' not found: {}", dir.display(), e)),
                    };
                    Some(can_dir)
                }
                None => canonical_path.parent().map(|p| p.to_path_buf()),
            };

            let profile_name = canonical_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("default")
                .to_string();

            return Ok(ResolvedChroot {
                profile_name,
                config_dir,
                config_path: canonical_path,
            });
        }

        // Case 2: Named profile (e.g. "tacos-rolling-x86_64")
        let cfg_filename = if root_spec.ends_with(".cfg") {
            root_spec.to_string()
        } else {
            format!("{}.cfg", root_spec)
        };

        // If explicit config dir was passed, check it first
        if let Some(dir) = explicit_config_dir {
            let target_cfg = dir.join(&cfg_filename);
            if target_cfg.is_file() {
                let canonical_cfg = match fs::canonicalize(&target_cfg) {
                    Ok(p) => p,
                    Err(e) => return Err(eyre!("Failed to canonicalize {}: {}", target_cfg.display(), e)),
                };
                let canonical_dir = match fs::canonicalize(dir) {
                    Ok(p) => p,
                    Err(e) => return Err(eyre!("Failed to canonicalize {}: {}", dir.display(), e)),
                };
                return Ok(ResolvedChroot {
                    profile_name: root_spec.to_string(),
                    config_dir: Some(canonical_dir),
                    config_path: canonical_cfg,
                });
            }
        }

        // Check standard search paths
        let mut searched_locations = Vec::new();
        for base in Self::default_search_paths() {
            if !base.is_dir() {
                continue;
            }
            let target_cfg = base.join(&cfg_filename);
            searched_locations.push(target_cfg.clone());

            if target_cfg.is_file() {
                let canonical_cfg = match fs::canonicalize(&target_cfg) {
                    Ok(p) => p,
                    Err(e) => return Err(eyre!("Failed to canonicalize {}: {}", target_cfg.display(), e)),
                };

                let is_system_etc = canonical_cfg.starts_with("/etc/mock");
                let config_dir = if is_system_etc {
                    None
                } else {
                    canonical_cfg.parent().map(|p| p.to_path_buf())
                };

                return Ok(ResolvedChroot {
                    profile_name: root_spec.to_string(),
                    config_dir,
                    config_path: canonical_cfg,
                });
            }
        }

        let mut err_msg = format!(
            "Could not locate Mock chroot configuration '{}'.\nSearched locations:\n",
            root_spec
        );
        for loc in &searched_locations {
            err_msg.push_str(&format!("  - {}\n", loc.display()));
        }
        err_msg.push_str("\nHint: You can provide a direct path (e.g. -r /path/to/profile.cfg) or use --mock-config-dir.");

        Err(eyre!(err_msg))
    }

    /// Discovers and lists all `.cfg` files across standard and custom search paths.
    pub fn list_all(custom_dir: Option<&Path>, include_system: bool) -> Result<Vec<ChrootConfig>> {
        let mut search_dirs = Vec::new();

        if let Some(dir) = custom_dir {
            search_dirs.push(dir.to_path_buf());
        }

        for p in Self::default_search_paths() {
            let is_etc = p.starts_with("/etc/mock");
            if is_etc && !include_system {
                continue;
            }
            if !search_dirs.contains(&p) {
                search_dirs.push(p);
            }
        }

        let mut configs = Vec::new();
        let mut seen_paths = HashSet::new();

        for dir in search_dirs {
            if !dir.is_dir() {
                continue;
            }

            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("cfg") {
                    let filename = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                    if filename == "site-defaults.cfg" || filename == "logging.ini" {
                        continue;
                    }

                    let canonical = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                    if seen_paths.insert(canonical.clone()) {
                        let parsed = Self::parse_config(&canonical);
                        configs.push(parsed);
                    }
                }
            }
        }

        configs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(configs)
    }

    /// Parses configuration metadata from a `.cfg` file and any included `.tpl` files.
    pub fn parse_config(cfg_path: &Path) -> ChrootConfig {
        let name = cfg_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let config_dir = cfg_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let content = match fs::read_to_string(cfg_path) {
            Ok(c) => c,
            Err(e) => {
                return ChrootConfig {
                    name,
                    path: cfg_path.to_path_buf(),
                    config_dir,
                    target_arch: None,
                    package_manager: None,
                    releasever: None,
                    dist: None,
                    vendor: None,
                    bootstrap_image: None,
                    description: None,
                    includes: Vec::new(),
                    valid: false,
                    validation_error: Some(format!("Failed to read file: {}", e)),
                };
            }
        };

        let mut target_arch = extract_config_val(&content, "target_arch");
        let mut package_manager = extract_config_val(&content, "package_manager");
        let mut releasever = extract_config_val(&content, "releasever");
        let mut dist = extract_macro_val(&content, "%dist");
        let mut vendor = extract_macro_val(&content, "%vendor");
        let mut bootstrap_image = extract_config_val(&content, "bootstrap_image");
        let mut description = extract_config_val(&content, "description");

        let includes = extract_includes(&content);
        let mut valid = true;
        let mut validation_error = None;

        // Inspect and parse each included file
        for inc in &includes {
            let template_path = config_dir.join(inc);
            if !template_path.is_file() {
                valid = false;
                validation_error = Some(format!(
                    "Missing included template: {} (searched at {})",
                    inc,
                    template_path.display()
                ));
                break;
            }

            if let Ok(tpl_content) = fs::read_to_string(&template_path) {
                if target_arch.is_none() {
                    target_arch = extract_config_val(&tpl_content, "target_arch");
                }
                if package_manager.is_none() {
                    package_manager = extract_config_val(&tpl_content, "package_manager");
                }
                if releasever.is_none() {
                    releasever = extract_config_val(&tpl_content, "releasever");
                }
                if dist.is_none() {
                    dist = extract_macro_val(&tpl_content, "%dist");
                }
                if vendor.is_none() {
                    vendor = extract_macro_val(&tpl_content, "%vendor");
                }
                if bootstrap_image.is_none() {
                    bootstrap_image = extract_config_val(&tpl_content, "bootstrap_image");
                }
                if description.is_none() {
                    description = extract_config_val(&tpl_content, "description");
                }
            }
        }

        ChrootConfig {
            name,
            path: cfg_path.to_path_buf(),
            config_dir,
            target_arch,
            package_manager,
            releasever,
            dist,
            vendor,
            bootstrap_image,
            description,
            includes,
            valid,
            validation_error,
        }
    }

    /// Verifies a chroot configuration by testing it with Mock (`mock --print-root-path`).
    pub fn check(root_spec: &str, explicit_config_dir: Option<&Path>) -> Result<CheckReport> {
        let resolved = Self::resolve(root_spec, explicit_config_dir)?;
        let config = Self::parse_config(&resolved.config_path);

        if !config.valid {
            return Ok(CheckReport {
                config: config.clone(),
                mock_verified: false,
                mock_root_path: None,
                error_message: config.validation_error,
            });
        }

        let mut cmd = Command::new("mock");
        if let Some(cfg_dir) = &resolved.config_dir {
            cmd.arg(format!("--configdir={}", cfg_dir.display()));
        }
        cmd.arg(format!("-r={}", resolved.profile_name));
        cmd.arg("--print-root-path");

        let output = match cmd.output() {
            Ok(out) => out,
            Err(e) => {
                return Ok(CheckReport {
                    config,
                    mock_verified: false,
                    mock_root_path: None,
                    error_message: Some(format!("Failed to execute 'mock': {}", e)),
                });
            }
        };

        if output.status.success() {
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let root_path = stdout_str
                .lines()
                .filter(|line| !line.starts_with("WARNING") && !line.starts_with("INFO"))
                .last()
                .map(|s| s.trim().to_string());

            Ok(CheckReport {
                config,
                mock_verified: true,
                mock_root_path: root_path,
                error_message: None,
            })
        } else {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let stdout_str = String::from_utf8_lossy(&output.stdout);
            let combined = format!("{}\n{}", stdout_str, stderr_str);
            let err_line = combined
                .lines()
                .find(|l| l.contains("ERROR"))
                .unwrap_or("Mock command exited with error status")
                .to_string();

            Ok(CheckReport {
                config,
                mock_verified: false,
                mock_root_path: None,
                error_message: Some(err_line),
            })
        }
    }

    /// Imports/copies a `.cfg` file (and referenced `templates/` folder) into the target directory.
    pub fn add(src_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
        if !src_path.is_file() {
            return Err(eyre!("Source path '{}' does not exist or is not a file", src_path.display()));
        }

        let filename = match src_path.file_name() {
            Some(f) => f,
            None => return Err(eyre!("Invalid source filename: {}", src_path.display())),
        };

        fs::create_dir_all(dest_dir)?;
        let target_cfg = dest_dir.join(filename);
        fs::copy(src_path, &target_cfg)?;

        // If the source directory has a templates/ directory, copy it over
        if let Some(parent) = src_path.parent() {
            let src_templates = parent.join("templates");
            if src_templates.is_dir() {
                let dest_templates = dest_dir.join("templates");
                fs::create_dir_all(&dest_templates)?;
                if let Ok(entries) = fs::read_dir(&src_templates) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_file() {
                            if let Some(t_name) = p.file_name() {
                                let _ = fs::copy(&p, dest_templates.join(t_name));
                            }
                        }
                    }
                }
            }
        }

        Ok(target_cfg)
    }

    /// Initializes a starter template for a new distribution chroot.
    pub fn init(name: &str, target_arch: &str, dest_dir: &Path) -> Result<PathBuf> {
        fs::create_dir_all(dest_dir)?;
        let templates_dir = dest_dir.join("templates");
        fs::create_dir_all(&templates_dir)?;

        let cfg_path = dest_dir.join(format!("{}.cfg", name));
        let tpl_path = templates_dir.join(format!("{}.tpl", name));

        let cfg_content = format!(
            "config_opts['target_arch'] = '{arch}'\nconfig_opts['legal_host_arches'] = ('{arch}',)\n\ninclude('templates/{name}.tpl')\n",
            arch = target_arch,
            name = name,
        );
        fs::write(&cfg_path, cfg_content)?;

        let tpl_content = format!(
            r#"config_opts['root'] = '{name}-{{{{ target_arch }}}}'
config_opts['chroot_setup_cmd'] = 'install @buildsys-build'
config_opts['dist'] = '.custom'
config_opts['releasever'] = 'rawhide'
config_opts['package_manager'] = 'dnf5'
config_opts['bootstrap_image'] = 'registry.fedoraproject.org/fedora:rawhide'
config_opts['bootstrap_image_ready'] = True
config_opts['description'] = 'Custom Distribution Chroot for {name}'

config_opts['macros']['%dist'] = '.custom'
config_opts['macros']['%vendor'] = '{name}'

config_opts['dnf.conf'] = """
[main]
keepcache=1
system_cachedir=/var/cache/dnf
debuglevel=2
reposdir=/dev/null
logfile=/var/log/yum.log
retries=20
obsoletes=1
gpgcheck=0
assumeyes=1
syslog_ident=mock
install_weak_deps=0
best=1

[fedora]
name=Fedora Rawhide
metalink=https://mirrors.fedoraproject.org/metalink?repo=rawhide&arch=$basearch
gpgcheck=0
enabled=1
"""
"#,
            name = name
        );
        fs::write(&tpl_path, tpl_content)?;

        Ok(cfg_path)
    }
}

/// Helper to extract string values assigned to `config_opts['key'] = 'val'`.
fn extract_config_val(content: &str, key: &str) -> Option<String> {
    let pattern_single = format!("config_opts['{}'] = '", key);
    let pattern_double = format!("config_opts['{}'] = \"", key);

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find(&pattern_single) {
            let rest = &trimmed[pos + pattern_single.len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        }
        if let Some(pos) = trimmed.find(&pattern_double) {
            let rest = &trimmed[pos + pattern_double.len()..];
            if let Some(end) = rest.find('"') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Helper to extract macro values like `config_opts['macros']['%key'] = 'val'`.
fn extract_macro_val(content: &str, macro_key: &str) -> Option<String> {
    let search = format!("['macros']['{}'] = '", macro_key);
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find(&search) {
            let rest = &trimmed[pos + search.len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Helper to extract `include('...')` directives.
fn extract_includes(content: &str) -> Vec<String> {
    let mut includes = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("include(") {
            let rest = &trimmed["include(".len()..];
            if let Some(quote_char) = rest.chars().next() {
                if quote_char == '\'' || quote_char == '"' {
                    let after_quote = &rest[1..];
                    if let Some(end) = after_quote.find(quote_char) {
                        includes.push(after_quote[..end].to_string());
                    }
                }
            }
        }
    }
    includes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_extract_config_and_macros() {
        let cfg = r#"
config_opts['target_arch'] = 'x86_64'
config_opts['package_manager'] = 'dnf5'
config_opts['macros']['%dist'] = '.tcrs'
config_opts['macros']['%vendor'] = 'TacOS'
include('templates/tacos-rolling.tpl')
"#;
        assert_eq!(extract_config_val(cfg, "target_arch"), Some("x86_64".to_string()));
        assert_eq!(extract_config_val(cfg, "package_manager"), Some("dnf5".to_string()));
        assert_eq!(extract_macro_val(cfg, "%dist"), Some(".tcrs".to_string()));
        assert_eq!(extract_macro_val(cfg, "%vendor"), Some("TacOS".to_string()));
        assert_eq!(extract_includes(cfg), vec!["templates/tacos-rolling.tpl"]);
    }

    #[test]
    fn test_resolve_tacos_mock_direct_path() {
        let tacos_cfg = PathBuf::from("/home/imcsk8/projects/gemini-workdir/tacos/mock/tacos-rolling-x86_64.cfg");
        if tacos_cfg.is_file() {
            let resolved = ChrootResolver::resolve(tacos_cfg.to_str().unwrap(), None)
                .expect("Failed to resolve direct tacos cfg path");
            assert_eq!(resolved.profile_name, "tacos-rolling-x86_64");
            assert!(resolved.config_dir.is_some());
            assert_eq!(
                resolved.config_dir.unwrap(),
                PathBuf::from("/home/imcsk8/projects/gemini-workdir/tacos/mock")
            );
        }
    }

    #[test]
    fn test_resolve_tacos_mock_by_name() {
        let resolved = ChrootResolver::resolve("tacos-rolling-x86_64", None)
            .expect("Failed to resolve tacos-rolling-x86_64 by name");
        assert_eq!(resolved.profile_name, "tacos-rolling-x86_64");
        assert!(resolved.config_dir.is_some());
        assert!(resolved.config_path.exists());
    }

    #[test]
    fn test_parse_tacos_mock_config() {
        let tacos_cfg = PathBuf::from("/home/imcsk8/projects/gemini-workdir/tacos/mock/tacos-rolling-x86_64.cfg");
        if tacos_cfg.is_file() {
            let config = ChrootResolver::parse_config(&tacos_cfg);
            assert_eq!(config.name, "tacos-rolling-x86_64");
            assert_eq!(config.target_arch, Some("x86_64".to_string()));
            assert_eq!(config.package_manager, Some("dnf5".to_string()));
            assert_eq!(config.dist, Some(".tcrs".to_string()));
            assert_eq!(config.vendor, Some("TacOS".to_string()));
            assert!(config.valid);
        }
    }

    #[test]
    fn test_init_and_parse_temp_chroot() {
        let temp_dir = std::env::temp_dir().join("dbs_chroot_test");
        let _ = fs::remove_dir_all(&temp_dir);

        let created_cfg = ChrootResolver::init("my-distro", "aarch64", &temp_dir)
            .expect("Failed to initialize test chroot");
        assert!(created_cfg.is_file());

        let parsed = ChrootResolver::parse_config(&created_cfg);
        assert_eq!(parsed.name, "my-distro");
        assert_eq!(parsed.target_arch, Some("aarch64".to_string()));
        assert_eq!(parsed.package_manager, Some("dnf5".to_string()));
        assert!(parsed.valid);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
