//! Distribution repository lifecycle management for DBS.
//!
//! Provides automated workflows to initialize, build, index, sign, publish,
//! and serve complete, production-grade Linux RPM distribution repositories
//! (such as TacOS, Fedora ELN derivatives, or custom appliances).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use eyre::{eyre, Result};

/// Configuration options for initializing a new distribution repository.
#[derive(Debug, Clone)]
pub struct DistroInitOptions {
    /// Distribution identifier (e.g. `tacos-stable-x86_64`).
    pub name: String,
    /// Target CPU architecture (e.g. `x86_64`, `aarch64`).
    pub arch: String,
    /// Distribution release channel (`stable`, `rolling`, `testing`).
    pub channel: String,
    /// Upstream release version base (e.g. `46`, `10`).
    pub releasever: String,
    /// RPM distribution macro tag (e.g. `tcst`, `tcrs`).
    pub dist_tag: String,
    /// Base directory for distribution repositories.
    pub dest_root: PathBuf,
    /// Directory to store Mock chroot configuration files.
    pub mock_dir: PathBuf,
}

/// Options for publishing, signing, and indexing build artifacts into a distribution repository.
#[derive(Debug, Clone)]
pub struct DistroPublishOptions {
    /// Distribution identifier (e.g. `tacos-stable-x86_64`).
    pub name: String,
    /// Source staging directory where built RPMs are located.
    pub staging_dir: PathBuf,
    /// Base directory for distribution repositories.
    pub dest_root: PathBuf,
    /// Target CPU architecture.
    pub arch: String,
    /// Base URL for client `.repo` configuration file (e.g. `http://repos.tacos.org.mx`).
    pub base_url: String,
    /// Optional GPG key identifier or email for RPM and metadata signing.
    pub sign_key: Option<String>,
    /// Number of concurrent worker threads for `createrepo_c`.
    pub workers: usize,
}

/// Summary report of distribution repository publication.
#[derive(Debug, Clone)]
pub struct DistroPublishReport {
    /// Target distribution directory.
    pub distro_dir: PathBuf,
    /// Path to binary RPM repository.
    pub binary_repo: PathBuf,
    /// Path to source RPM repository.
    pub source_repo: PathBuf,
    /// Number of binary RPMs published.
    pub binary_count: usize,
    /// Number of source RPMs published.
    pub source_count: usize,
    /// Path to generated client `.repo` file.
    pub client_repo_file: PathBuf,
    /// Whether packages and metadata were GPG-signed.
    pub gpg_signed: bool,
}

/// Status and metadata metrics for an existing distribution repository.
#[derive(Debug, Clone)]
pub struct DistroStatus {
    /// Distribution identifier.
    pub name: String,
    /// Repository root directory on disk.
    pub root_dir: PathBuf,
    /// Target architecture.
    pub arch: String,
    /// Total binary RPM packages.
    pub binary_count: usize,
    /// Total source RPM packages.
    pub source_count: usize,
    /// Whether `repodata/repomd.xml` exists.
    pub repodata_present: bool,
    /// Whether GPG signature `repomd.xml.asc` is present.
    pub gpg_signed: bool,
    /// Path to client `.repo` configuration file if generated.
    pub client_repo_file: Option<PathBuf>,
}

/// Initializes the directory structure and Mock chroot profile for a new distribution.
pub fn init_distro(opts: &DistroInitOptions) -> Result<PathBuf> {
    let distro_dir = opts.dest_root.join(&opts.name);
    let arch_dir = distro_dir.join(&opts.arch);
    let srpms_dir = distro_dir.join("source").join("SRPMS");

    if let Err(e) = fs::create_dir_all(&arch_dir) {
        return Err(eyre!("Failed to create architecture directory {}: {}", arch_dir.display(), e));
    }
    if let Err(e) = fs::create_dir_all(&srpms_dir) {
        return Err(eyre!("Failed to create source RPM directory {}: {}", srpms_dir.display(), e));
    }

    if let Err(e) = fs::create_dir_all(&opts.mock_dir) {
        return Err(eyre!("Failed to create mock directory {}: {}", opts.mock_dir.display(), e));
    }

    let mock_cfg_path = opts.mock_dir.join(format!("{}.cfg", opts.name));
    if !mock_cfg_path.exists() {
        let content = format!(
            "config_opts['target_arch'] = '{}'\n\
             config_opts['legal_host_arches'] = ('{}',)\n\
             config_opts['dist'] = '{}'\n\
             config_opts['releasever'] = '{}'\n\
             config_opts['channel'] = '{}'\n\n\
             include('templates/tacos-rolling.tpl')\n",
            opts.arch, opts.arch, opts.dist_tag, opts.releasever, opts.channel
        );

        if let Err(e) = fs::write(&mock_cfg_path, content) {
            return Err(eyre!("Failed to write Mock chroot profile {}: {}", mock_cfg_path.display(), e));
        }
    }

    Ok(distro_dir)
}

/// Discovers all `.rpm` and `.src.rpm` files within a directory recursively.
fn discover_rpms(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut binaries = Vec::new();
    let mut sources = Vec::new();

    if !dir.exists() {
        return (binaries, sources);
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let (sub_bin, sub_src) = discover_rpms(&p);
                binaries.extend(sub_bin);
                sources.extend(sub_src);
            } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if ext == "rpm" {
                    if p.to_string_lossy().ends_with(".src.rpm") {
                        sources.push(p);
                    } else {
                        binaries.push(p);
                    }
                }
            }
        }
    }

    (binaries, sources)
}

/// Publishes build artifacts into the distribution repository, signs packages, generates repodata, and writes client config.
pub fn publish_distro(opts: &DistroPublishOptions) -> Result<DistroPublishReport> {
    let distro_dir = opts.dest_root.join(&opts.name);
    let binary_repo = distro_dir.join(&opts.arch);
    let source_repo = distro_dir.join("source").join("SRPMS");

    if let Err(e) = fs::create_dir_all(&binary_repo) {
        return Err(eyre!("Failed to create binary repository {}: {}", binary_repo.display(), e));
    }
    if let Err(e) = fs::create_dir_all(&source_repo) {
        return Err(eyre!("Failed to create source repository {}: {}", source_repo.display(), e));
    }

    // 1. Discover and stage RPMs from staging directory
    let (staged_binaries, staged_sources) = discover_rpms(&opts.staging_dir);
    let mut published_binaries = Vec::new();
    let mut published_sources = Vec::new();

    for rpm in staged_binaries {
        if let Some(filename) = rpm.file_name() {
            let dest = binary_repo.join(filename);
            let _ = fs::copy(&rpm, &dest);
            published_binaries.push(dest);
        }
    }

    for srpm in staged_sources {
        if let Some(filename) = srpm.file_name() {
            let dest = source_repo.join(filename);
            let _ = fs::copy(&srpm, &dest);
            published_sources.push(dest);
        }
    }

    // 2. Optional GPG Signing of RPM packages
    let mut gpg_signed = false;
    if let Some(key_id) = &opts.sign_key {
        println!("Signing binary RPM packages with GPG key '{}'...", key_id);
        for rpm in &published_binaries {
            let _ = Command::new("rpm")
                .arg(format!("--define=%_gpg_name {}", key_id))
                .arg("--addsign")
                .arg(rpm)
                .status();
        }

        // Export public GPG key into repository root
        let gpg_key_file = distro_dir.join(format!("RPM-GPG-KEY-{}", opts.name));
        let export_out = Command::new("gpg")
            .arg("--armor")
            .arg("--export")
            .arg(key_id)
            .output();

        if let Ok(out) = export_out {
            if out.status.success() && !out.stdout.is_empty() {
                let _ = fs::write(&gpg_key_file, out.stdout);
                println!("✓ Exported distribution GPG public key to {}", gpg_key_file.display());
            }
        }

        gpg_signed = true;
    }

    // 3. Generate repository metadata with createrepo_c
    println!("Indexing binary repository metadata with createrepo_c...");
    let mut cmd_bin = Command::new("createrepo_c");
    cmd_bin.arg("--update");
    cmd_bin.arg(format!("--workers={}", opts.workers));
    cmd_bin.arg(&binary_repo);
    let status_bin = match cmd_bin.status() {
        Ok(st) => st,
        Err(e) => return Err(eyre!("Failed to execute createrepo_c: {}", e)),
    };
    if !status_bin.success() {
        return Err(eyre!("createrepo_c failed on {}", binary_repo.display()));
    }

    if !published_sources.is_empty() {
        println!("Indexing source repository metadata with createrepo_c...");
        let mut cmd_src = Command::new("createrepo_c");
        cmd_src.arg("--update");
        cmd_src.arg(format!("--workers={}", opts.workers));
        cmd_src.arg(&source_repo);
        let _ = cmd_src.status();
    }

    // 4. Optional signing of repomd.xml
    if let Some(key_id) = &opts.sign_key {
        let repomd_bin = binary_repo.join("repodata").join("repomd.xml");
        if repomd_bin.exists() {
            println!("Signing repository metadata {}...", repomd_bin.display());
            let _ = Command::new("gpg")
                .arg("--detach-sign")
                .arg("--armor")
                .arg("--batch")
                .arg("--yes")
                .arg("-u")
                .arg(key_id)
                .arg(&repomd_bin)
                .status();
        }
    }

    // 5. Generate client .repo configuration file
    let client_repo_file = distro_dir.join(format!("{}.repo", opts.name));
    let mut repo_content = format!(
        "[{}]\n\
         name=TacOS Linux - {} ($basearch)\n\
         baseurl={}/{}/$basearch\n\
         enabled=1\n\
         gpgcheck={}\n\
         repo_gpgcheck={}\n",
        opts.name,
        opts.name,
        opts.base_url.trim_end_matches('/'),
        opts.name,
        if gpg_signed { "1" } else { "0" },
        if gpg_signed { "1" } else { "0" }
    );

    if gpg_signed {
        repo_content.push_str(&format!(
            "gpgkey={}/{}/RPM-GPG-KEY-{}\n",
            opts.base_url.trim_end_matches('/'),
            opts.name,
            opts.name
        ));
    }
    repo_content.push_str("metadata_expire=300\nskip_if_unavailable=False\n\n");

    repo_content.push_str(&format!(
        "[{}-source]\n\
         name=TacOS Linux - {} (Source)\n\
         baseurl={}/{}/source/SRPMS\n\
         enabled=0\n\
         gpgcheck={}\n\
         metadata_expire=300\n\
         skip_if_unavailable=False\n",
        opts.name,
        opts.name,
        opts.base_url.trim_end_matches('/'),
        opts.name,
        if gpg_signed { "1" } else { "0" }
    ));

    if let Err(e) = fs::write(&client_repo_file, repo_content) {
        return Err(eyre!("Failed to write client repo file {}: {}", client_repo_file.display(), e));
    }

    Ok(DistroPublishReport {
        distro_dir,
        binary_repo,
        source_repo,
        binary_count: published_binaries.len(),
        source_count: published_sources.len(),
        client_repo_file,
        gpg_signed,
    })
}

/// Inspects an existing distribution repository on disk and reports metrics and health.
pub fn get_distro_status(distro_name: &str, dest_root: &Path, arch: &str) -> Result<DistroStatus> {
    let root_dir = dest_root.join(distro_name);
    if !root_dir.exists() {
        return Err(eyre!("Distribution repository does not exist at {}", root_dir.display()));
    }

    let binary_dir = root_dir.join(arch);
    let source_dir = root_dir.join("source").join("SRPMS");

    let (binaries, _) = discover_rpms(&binary_dir);
    let (_, sources) = discover_rpms(&source_dir);

    let repomd_path = binary_dir.join("repodata").join("repomd.xml");
    let repodata_present = repomd_path.exists();

    let repomd_asc = binary_dir.join("repodata").join("repomd.xml.asc");
    let gpg_signed = repomd_asc.exists();

    let client_repo = root_dir.join(format!("{}.repo", distro_name));
    let client_repo_file = if client_repo.exists() {
        Some(client_repo)
    } else {
        None
    };

    Ok(DistroStatus {
        name: distro_name.to_string(),
        root_dir,
        arch: arch.to_string(),
        binary_count: binaries.len(),
        source_count: sources.len(),
        repodata_present,
        gpg_signed,
        client_repo_file,
    })
}

/// Generates an optimized Nginx virtual host configuration string for the distribution repository.
pub fn generate_nginx_config(root_path: &Path, server_name: &str, port: u16) -> String {
    format!(
        "# TacOS Distribution Repository Web Server Configuration\n\
         # Generated automatically by DBS\n\
         server {{\n\
             listen {};\n\
             server_name {};\n\n\
             root {};\n\n\
             # Enable directory listing\n\
             autoindex on;\n\
             autoindex_exact_size off;\n\
             autoindex_localtime on;\n\n\
             # High-throughput streaming settings for large binary payloads\n\
             sendfile on;\n\
             tcp_nopush on;\n\
             tcp_nodelay on;\n\n\
             # Repository metadata must NEVER be cached by proxies or browsers\n\
             location ~* /repodata/.*$ {{\n\
                 expires -1;\n\
                 add_header Cache-Control \"no-cache, no-store, must-revalidate\";\n\
             }}\n\n\
             # Client .repo and GPG keys should refresh periodically\n\
             location ~* \\.(repo|asc|cer)$ {{\n\
                 expires 1h;\n\
                 add_header Cache-Control \"public, must-revalidate\";\n\
             }}\n\n\
             # Version-immutable binary RPMs can be cached long-term\n\
             location ~* \\.rpm$ {{\n\
                 expires 30d;\n\
                 add_header Cache-Control \"public\";\n\
             }}\n\n\
             access_log /var/log/nginx/{}_access.log;\n\
             error_log  /var/log/nginx/{}_error.log;\n\
         }}\n",
        port,
        server_name,
        root_path.display(),
        server_name.replace('.', "_"),
        server_name.replace('.', "_")
    )
}

/// Starts an asynchronous, embedded HTTP repository server for immediate local client consumption.
pub async fn run_http_server(root_path: PathBuf, host: &str, port: u16) -> Result<()> {
    use tokio::net::TcpListener;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let bind_addr = format!("{}:{}", host, port);
    let listener = match TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => return Err(eyre!("Failed to bind HTTP server to {}: {}", bind_addr, e)),
    };

    println!("===========================================================");
    println!(" DBS Embedded Distribution Repository Web Server");
    println!(" Listening on:   http://{}", bind_addr);
    println!(" Serving Root:   {}", root_path.display());
    println!(" Press Ctrl+C to terminate");
    println!("===========================================================");

    loop {
        let (mut socket, _peer) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };

        let base_dir = root_path.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            let n = match socket.read(&mut buf).await {
                Ok(bytes) if bytes > 0 => bytes,
                _ => return,
            };

            let req_str = String::from_utf8_lossy(&buf[..n]);
            let first_line = req_str.lines().next().unwrap_or("");
            let parts: Vec<&str> = first_line.split_whitespace().collect();

            if parts.len() < 2 || (parts[0] != "GET" && parts[0] != "HEAD") {
                let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n").await;
                return;
            }

            let req_path = parts[1].split('?').next().unwrap_or("/");
            let safe_rel = req_path.trim_start_matches('/');
            let target_path = base_dir.join(safe_rel);

            if target_path.is_file() {
                let content_type = if safe_rel.ends_with(".rpm") {
                    "application/x-rpm"
                } else if safe_rel.ends_with(".xml") || safe_rel.ends_with(".xml.gz") {
                    "application/xml"
                } else if safe_rel.ends_with(".repo") || safe_rel.ends_with(".asc") {
                    "text/plain"
                } else {
                    "application/octet-stream"
                };

                let data = match fs::read(&target_path) {
                    Ok(d) => d,
                    Err(_) => {
                        let _ = socket.write_all(b"HTTP/1.1 500 Internal Error\r\n\r\n").await;
                        return;
                    }
                };

                let header = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: {}\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    content_type,
                    data.len()
                );

                let _ = socket.write_all(header.as_bytes()).await;
                if parts[0] == "GET" {
                    let _ = socket.write_all(&data).await;
                }
            } else if target_path.is_dir() {
                // Generate simple directory listing
                let mut html = format!("<html><head><title>Index of {}</title></head><body><h1>Index of {}</h1><hr><pre><a href=\"../\">../</a>\n", req_path, req_path);
                if let Ok(entries) = fs::read_dir(&target_path) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let is_dir = entry.path().is_dir();
                        let slash = if is_dir { "/" } else { "" };
                        html.push_str(&format!("<a href=\"{}\">{}{}</a>\n", name, name, slash));
                    }
                }
                html.push_str("</pre><hr></body></html>");

                let header = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/html; charset=utf-8\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    html.len()
                );

                let _ = socket.write_all(header.as_bytes()).await;
                if parts[0] == "GET" {
                    let _ = socket.write_all(html.as_bytes()).await;
                }
            } else {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found").await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_init_distro() {
        let dir = tempdir().unwrap();
        let mock_dir = dir.path().join("mock");
        let dest_root = dir.path().join("distro");

        let opts = DistroInitOptions {
            name: "tacos-stable-x86_64".to_string(),
            arch: "x86_64".to_string(),
            channel: "stable".to_string(),
            releasever: "46".to_string(),
            dist_tag: "tcst".to_string(),
            dest_root: dest_root.clone(),
            mock_dir: mock_dir.clone(),
        };

        let res = init_distro(&opts).unwrap();
        assert!(res.exists());
        assert!(dest_root.join("tacos-stable-x86_64").join("x86_64").is_dir());
        assert!(dest_root.join("tacos-stable-x86_64").join("source").join("SRPMS").is_dir());
        assert!(mock_dir.join("tacos-stable-x86_64.cfg").is_file());
    }

    #[test]
    fn test_generate_nginx_config() {
        let path = Path::new("/srv/dbs/tacos/distro");
        let conf = generate_nginx_config(path, "repos.tacos.org.mx", 80);
        assert!(conf.contains("listen 80;"));
        assert!(conf.contains("server_name repos.tacos.org.mx;"));
        assert!(conf.contains("root /srv/dbs/tacos/distro;"));
        assert!(conf.contains("location ~* /repodata/.*$"));
        assert!(conf.contains("application/x-rpm") == false); // mime handled by extensions
    }

    #[test]
    fn test_distro_status_not_found() {
        let dir = tempdir().unwrap();
        assert!(get_distro_status("nonexistent", dir.path(), "x86_64").is_err());
    }
}
