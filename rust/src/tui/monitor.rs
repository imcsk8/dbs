//! Interactive TUI System Monitor for DBS using Charmed Rust.
//!
//! Provides a live multi-tab dashboard displaying active distribution repository
//! status, lookaside CAS storage, mock chroot configurations, and database metrics.

use std::path::{Path, PathBuf};
use std::time::Instant;
use bubbletea::{Cmd, KeyMsg, KeyType, Message, Model, Program, WindowSizeMsg, quit};
use eyre::Result;
use lipgloss::{Position, Style};

use crate::chroot::{ChrootConfig, ChrootResolver};
use crate::config::DbsConfig;
use crate::db::{
    establish_connection, get_build_counts, get_database_url, list_active_builds,
    list_operating_systems, list_packages, list_recent_builds, BuildCounts,
};
use crate::distro::{get_distro_status, DistroStatus};
use crate::lookaside::{LookasideManager, LookasideStatus};
use crate::models::Package;
use crate::tui::theme::{render_badge, render_card, render_footer, render_header, render_tabs, BadgeKind, Palette};
use crate::types::BuildStatus;

/// Snapshot of all DBS subsystem metrics.
#[derive(Debug, Clone)]
pub struct MonitorSnapshot {
    pub distro_name: String,
    pub distro_dest: PathBuf,
    pub distro_status: Option<DistroStatus>,
    pub distro_error: Option<String>,
    pub lookaside_status: Option<LookasideStatus>,
    pub lookaside_error: Option<String>,
    pub chroots: Vec<ChrootConfig>,
    pub db_url_masked: Option<String>,
    pub db_connected: bool,
    pub db_os_count: usize,
    pub db_pkg_count: usize,
    pub active_builds: Vec<Package>,
    pub recent_builds: Vec<Package>,
    pub build_counts: BuildCounts,
    pub last_refreshed: Instant,
}

impl MonitorSnapshot {
    /// Collects a live snapshot of all DBS subsystems.
    pub fn collect(dbs_cfg: &DbsConfig, distro_name: &str) -> Self {
        let distro_dest = dbs_cfg.distro.dest.clone();
        let (distro_status, distro_error) = match get_distro_status(distro_name, &distro_dest, "x86_64") {
            Ok(st) => (Some(st), None),
            Err(e) => (None, Some(e.to_string())),
        };

        let lookaside_mgr = LookasideManager::resolve_default(Some(&dbs_cfg.distgit.lookaside_dir));
        let (lookaside_status, lookaside_error) = match lookaside_mgr.status() {
            Ok(st) => (Some(st), None),
            Err(e) => (None, Some(e.to_string())),
        };

        let chroots = ChrootResolver::list_all(dbs_cfg.chroot.config_dir.as_deref(), true).unwrap_or_default();

        let raw_db_url = get_database_url();
        let db_url_masked = raw_db_url.as_ref().map(|u| {
            if let Some((proto, rest)) = u.split_once("://") {
                if let Some((creds, host_part)) = rest.split_once('@') {
                    let user = creds.split(':').next().unwrap_or("user");
                    format!("{}://{}:****@{}", proto, user, host_part)
                } else {
                    u.clone()
                }
            } else {
                u.clone()
            }
        });

        let mut db_connected = false;
        let mut db_os_count = 0;
        let mut db_pkg_count = 0;
        let mut active_builds = Vec::new();
        let mut recent_builds = Vec::new();
        let mut build_counts = BuildCounts::default();

        if let Ok(mut conn) = establish_connection() {
            db_connected = true;
            if let Ok(oses) = list_operating_systems(&mut conn) {
                db_os_count = oses.len();
            }
            if let Ok(pkgs) = list_packages(&mut conn, 1000) {
                db_pkg_count = pkgs.len();
            }
            if let Ok(active) = list_active_builds(&mut conn) {
                active_builds = active;
            }
            if let Ok(recent) = list_recent_builds(&mut conn, 20) {
                recent_builds = recent;
            }
            if let Ok(counts) = get_build_counts(&mut conn) {
                build_counts = counts;
            }
        }

        Self {
            distro_name: distro_name.to_string(),
            distro_dest,
            distro_status,
            distro_error,
            lookaside_status,
            lookaside_error,
            chroots,
            db_url_masked,
            db_connected,
            db_os_count,
            db_pkg_count,
            active_builds,
            recent_builds,
            build_counts,
            last_refreshed: Instant::now(),
        }
    }
}

/// Message returned when snapshot refresh finishes.
#[derive(Clone)]
struct RefreshDoneMsg(MonitorSnapshot);

/// Heartbeat message for periodic auto-refresh.
#[derive(Clone)]
struct TickMsg(MonitorSnapshot);

/// Interactive TUI Dashboard Model.
pub struct MonitorModel {
    dbs_cfg: DbsConfig,
    distro_name: String,
    snapshot: MonitorSnapshot,
    active_tab: usize,
    status_message: Option<String>,
    width: usize,
    height: usize,
}

impl MonitorModel {
    pub fn new(dbs_cfg: DbsConfig, distro_name: String) -> Self {
        let snapshot = MonitorSnapshot::collect(&dbs_cfg, &distro_name);
        Self {
            dbs_cfg,
            distro_name,
            snapshot,
            active_tab: 0,
            status_message: None,
            width: 100,
            height: 30,
        }
    }

    fn tick_cmd(cfg: DbsConfig, distro_name: String) -> Cmd {
        Cmd::blocking(move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let snap = MonitorSnapshot::collect(&cfg, &distro_name);
            Message::new(TickMsg(snap))
        })
    }

    fn trigger_refresh(&mut self) -> Option<Cmd> {
        self.status_message = Some("Refreshing system metrics...".to_string());
        let cfg = self.dbs_cfg.clone();
        let name = self.distro_name.clone();
        Some(Cmd::blocking(move || {
            let snap = MonitorSnapshot::collect(&cfg, &name);
            Message::new(RefreshDoneMsg(snap))
        }))
    }
}

impl Model for MonitorModel {
    fn init(&self) -> Option<Cmd> {
        Some(Self::tick_cmd(self.dbs_cfg.clone(), self.distro_name.clone()))
    }

    fn update(&mut self, msg: Message) -> Option<Cmd> {
        if let Some(size) = msg.downcast_ref::<WindowSizeMsg>() {
            self.width = (size.width as usize).max(40);
            self.height = (size.height as usize).max(15);
            return None;
        }

        if let Some(tick) = msg.downcast_ref::<TickMsg>() {
            self.snapshot = tick.0.clone();
            return Some(Self::tick_cmd(self.dbs_cfg.clone(), self.distro_name.clone()));
        }

        if let Some(refresh_msg) = msg.downcast_ref::<RefreshDoneMsg>() {
            self.snapshot = refresh_msg.0.clone();
            self.status_message = Some("✓ Metrics updated".to_string());
            return None;
        }

        if let Some(key) = msg.downcast_ref::<KeyMsg>() {
            match key.key_type {
                KeyType::CtrlC | KeyType::Esc => return Some(quit()),
                KeyType::Tab => {
                    self.active_tab = (self.active_tab + 1) % 6;
                }
                KeyType::ShiftTab => {
                    self.active_tab = (self.active_tab + 5) % 6;
                }
                KeyType::Runes => {
                    if let Some(&ch) = key.runes.first() {
                        match ch {
                            'q' | 'Q' => return Some(quit()),
                            'r' | 'R' => return self.trigger_refresh(),
                            '1' => self.active_tab = 0,
                            '2' => self.active_tab = 1,
                            '3' => self.active_tab = 2,
                            '4' => self.active_tab = 3,
                            '5' => self.active_tab = 4,
                            '6' => self.active_tab = 5,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        None
    }

    fn view(&self) -> String {
        let width = self.width.max(60);
        let header = render_header(
            "🍱 DBS SYSTEM MONITOR",
            &format!("Distribution: {}  •  Target: {}", self.distro_name, self.dbs_cfg.distro.dest.display()),
            width,
        );

        let tabs = ["Overview", "Builds", "Distro Repo", "Lookaside CAS", "Mock Chroots", "Database"];
        let tabs_bar = render_tabs(&tabs, self.active_tab, width);

        let body = match self.active_tab {
            0 => self.render_overview(width),
            1 => self.render_builds_tab(width),
            2 => self.render_distro_tab(width),
            3 => self.render_lookaside_tab(width),
            4 => self.render_chroots_tab(width),
            5 => self.render_db_tab(width),
            _ => "Unknown tab".to_string(),
        };

        let shortcuts = [
            ("1-6/Tab", "Switch View"),
            ("r", "Refresh"),
            ("q/Esc", "Quit"),
        ];
        let footer = render_footer(&shortcuts, self.status_message.as_deref(), width);

        format!("{}\n{}\n{}\n{}", header, tabs_bar, body, footer)
    }
}

impl MonitorModel {
    /// Renders Tab 1: Overview (subsystem status grid with build pipeline banner).
    fn render_overview(&self, width: usize) -> String {
        let card_w = (width.saturating_sub(4) / 2).max(35);

        // Build Pipeline Telemetry Banner
        let build_banner = {
            let counts = &self.snapshot.build_counts;
            let active_badge = if counts.building > 0 {
                render_badge(&format!("{} BUILDING", counts.building), BadgeKind::Warning)
            } else {
                render_badge("IDLE", BadgeKind::Dim)
            };
            let success_text = Style::new().bold().foreground(Palette::GREEN).render(&counts.success.to_string());
            let failed_text = Style::new().bold().foreground(Palette::RED).render(&counts.failed.to_string());
            let building_text = Style::new().bold().foreground(Palette::YELLOW).render(&counts.building.to_string());
            let total_text = Style::new().bold().foreground(Palette::CYAN).render(&counts.total.to_string());

            let line1 = format!(
                "• Pipeline:     {}   Building: {}   Success: {}   Failed: {}   Tracked: {}",
                active_badge, building_text, success_text, failed_text, total_text
            );

            let line2 = if self.snapshot.active_builds.is_empty() {
                "• Active Tasks: No compilation jobs currently in progress.".to_string()
            } else {
                let pkg_names: Vec<String> = self.snapshot.active_builds.iter()
                    .take(4)
                    .map(|p| {
                        let w = p.worker_id.map(|wid| format!(" (W{})", wid)).unwrap_or_default();
                        format!("{}{}", Style::new().bold().foreground(Palette::YELLOW).render(&p.name), w)
                    })
                    .collect();
                format!("• Active Tasks: {}", pkg_names.join(", "))
            };

            render_card("⚡ Live Build Pipeline Telemetry", &format!("{}\n{}", line1, line2), width.saturating_sub(4).max(60), false)
        };

        // Card 1: Distro
        let distro_card = {
            let mut lines = Vec::new();
            if let Some(st) = &self.snapshot.distro_status {
                let badge = if st.repodata_present {
                    render_badge("ONLINE", BadgeKind::Success)
                } else {
                    render_badge("INIT", BadgeKind::Warning)
                };
                lines.push(format!("• Status:       {}", badge));
                lines.push(format!("• Repository:   {}", st.name));
                lines.push(format!("• Binary RPMs:  {}", Style::new().bold().foreground(Palette::CYAN).render(&st.binary_count.to_string())));
                lines.push(format!("• Source SRPMs: {}", st.source_count));
                lines.push(format!("• GPG Signed:   {}", if st.gpg_signed { "Yes" } else { "No" }));
                lines.push(format!("• Root Dir:     {}", st.root_dir.display()));
            } else {
                lines.push(format!("• Status:       {}", render_badge("NOT FOUND", BadgeKind::Dim)));
                if let Some(err) = &self.snapshot.distro_error {
                    lines.push(format!("• Error:        {}", err));
                }
            }
            render_card("📦 Distribution Repository", &lines.join("\n"), card_w, self.active_tab == 0)
        };

        // Card 2: Lookaside
        let lookaside_card = {
            let mut lines = Vec::new();
            if let Some(st) = &self.snapshot.lookaside_status {
                let fs_badge = if st.is_btrfs {
                    render_badge("BTRFS COW", BadgeKind::Success)
                } else {
                    render_badge("POSIX", BadgeKind::Info)
                };
                let mb = (st.total_cas_bytes as f64) / (1024.0 * 1024.0);
                lines.push(format!("• Filesystem:   {}", fs_badge));
                lines.push(format!("• CAS Objects:  {}", Style::new().bold().foreground(Palette::PURPLE_LIGHT).render(&st.total_cas_objects.to_string())));
                lines.push(format!("• Total Stored: {:.2} MB", mb));
                lines.push(format!("• Cached Pkgs:  {}", st.total_packages));
                lines.push(format!("• Storage Root: {}", st.root.display()));
            } else {
                lines.push(format!("• Status:       {}", render_badge("OFFLINE", BadgeKind::Dim)));
            }
            render_card("🗄 Lookaside CAS Storage", &lines.join("\n"), card_w, false)
        };

        // Card 3: Mock Chroots
        let chroot_card = {
            let mut lines = Vec::new();
            let count = self.snapshot.chroots.len();
            let badge = if count > 0 {
                render_badge("READY", BadgeKind::Success)
            } else {
                render_badge("EMPTY", BadgeKind::Warning)
            };
            lines.push(format!("• Status:       {}", badge));
            lines.push(format!("• Profiles:     {} discovered", count));
            if let Some(first) = self.snapshot.chroots.first() {
                lines.push(format!("• Primary:      {}", Style::new().bold().foreground(Palette::CYAN).render(&first.name)));
                lines.push(format!("• Architecture: {}", first.target_arch.as_deref().unwrap_or("x86_64")));
                lines.push(format!("• Engine:       {}", first.package_manager.as_deref().unwrap_or("dnf5")));
            }
            lines.push(format!("• Search Dir:   {}", self.dbs_cfg.chroot.config_dir.as_deref().unwrap_or(Path::new("mock")).display()));
            render_card("🛡 Mock Chroot Environments", &lines.join("\n"), card_w, false)
        };

        // Card 4: Database
        let db_card = {
            let mut lines = Vec::new();
            let badge = if self.snapshot.db_connected {
                render_badge("CONNECTED", BadgeKind::Success)
            } else {
                render_badge("OFFLINE", BadgeKind::Dim)
            };
            lines.push(format!("• Connection:   {}", badge));
            if let Some(masked) = &self.snapshot.db_url_masked {
                lines.push(format!("• Database URI: {}", Style::new().foreground(Palette::TEXT_MUTED).render(masked)));
            }
            lines.push(format!("• OS Targets:   {}", self.snapshot.db_os_count));
            lines.push(format!("• Catalog Pkgs: {}", self.snapshot.db_pkg_count));
            render_card("🗃 PostgreSQL Metadata Database", &lines.join("\n"), card_w, false)
        };

        let top_row = lipgloss::join_horizontal(Position::Top, &[&distro_card, &lookaside_card]);
        let bottom_row = lipgloss::join_horizontal(Position::Top, &[&chroot_card, &db_card]);

        format!("{}\n{}\n{}", build_banner, top_row, bottom_row)
    }

    /// Renders Tab 2: Live Builds & Compilation History.
    fn render_builds_tab(&self, width: usize) -> String {
        let card_w = width.saturating_sub(4).max(60);
        let mut lines = Vec::new();

        let counts = &self.snapshot.build_counts;
        let summary_line = format!(
            "Total Tracked: {}  •  Building: {}  •  Success: {}  •  Failed: {}  •  Pending: {}  •  Heartbeat: 2s",
            Style::new().bold().foreground(Palette::CYAN).render(&counts.total.to_string()),
            Style::new().bold().foreground(Palette::YELLOW).render(&counts.building.to_string()),
            Style::new().bold().foreground(Palette::GREEN).render(&counts.success.to_string()),
            Style::new().bold().foreground(Palette::RED).render(&counts.failed.to_string()),
            counts.pending,
        );
        lines.push(summary_line);
        lines.push(String::new());

        // Section 1: Active Builds
        lines.push(Style::new().bold().foreground(Palette::YELLOW).render("▶ ACTIVE COMPILATION JOBS"));
        if self.snapshot.active_builds.is_empty() {
            lines.push(Style::new().foreground(Palette::TEXT_MUTED).render("  (No packages currently building. Ready for jobs.)"));
        } else {
            lines.push(format!("  {:<24} {:<12} {:<10} {:<12} {}", "Package", "Version", "Worker", "Status", "Log Path"));
            lines.push(format!("  {}", "─".repeat(card_w.saturating_sub(6))));
            for b in &self.snapshot.active_builds {
                let worker_str = b.worker_id.map(|w| format!("Worker-{}", w)).unwrap_or_else(|| "default".to_string());
                let status_badge = render_badge("BUILDING", BadgeKind::Warning);
                let log = b.build_log_path.as_deref().unwrap_or("-");
                lines.push(format!("  {:<24} {:<12} {:<10} {:<12} {}",
                    Style::new().bold().foreground(Palette::CYAN_LIGHT).render(&b.name),
                    b.version,
                    worker_str,
                    status_badge,
                    log
                ));
            }
        }

        lines.push(String::new());

        // Section 2: Recent Builds
        lines.push(Style::new().bold().foreground(Palette::CYAN).render("▶ RECENT BUILD HISTORY"));
        if self.snapshot.recent_builds.is_empty() {
            lines.push(Style::new().foreground(Palette::TEXT_MUTED).render("  (No build history found in database.)"));
        } else {
            lines.push(format!("  {:<24} {:<12} {:<12} {:<10} {:<10} {}", "Package", "Version", "Status", "Duration", "Worker", "Details / Error"));
            lines.push(format!("  {}", "─".repeat(card_w.saturating_sub(6))));
            for b in &self.snapshot.recent_builds {
                let badge = match b.build_status {
                    BuildStatus::BUILDING => render_badge("BUILDING", BadgeKind::Warning),
                    BuildStatus::SUCCESS => render_badge("SUCCESS", BadgeKind::Success),
                    BuildStatus::FAILED => render_badge("FAILED", BadgeKind::Error),
                    BuildStatus::PENDING => render_badge("PENDING", BadgeKind::Info),
                    BuildStatus::SKIPPED => render_badge("SKIPPED", BadgeKind::Dim),
                };
                let dur = b.build_duration_seconds.map(|d| format!("{:.1}s", d)).unwrap_or_else(|| "-".to_string());
                let worker_str = b.worker_id.map(|w| format!("W{}", w)).unwrap_or_else(|| "-".to_string());
                let details = if let Some(err) = &b.error_summary {
                    Style::new().foreground(Palette::RED).render(&err.lines().next().unwrap_or("error").chars().take(40).collect::<String>())
                } else if let Some(log) = &b.build_log_path {
                    Style::new().foreground(Palette::TEXT_MUTED).render(log)
                } else {
                    "-".to_string()
                };

                lines.push(format!("  {:<24} {:<12} {:<12} {:<10} {:<10} {}",
                    b.name,
                    b.version,
                    badge,
                    dur,
                    worker_str,
                    details
                ));
            }
        }

        render_card("🔨 Package Build Pipeline & Real-Time Telemetry", &lines.join("\n"), card_w, true)
    }

    /// Renders Tab 3: Distribution Details.
    fn render_distro_tab(&self, width: usize) -> String {
        let card_w = width.saturating_sub(4).max(50);
        let mut lines = Vec::new();

        lines.push(format!("Distribution Name:    {}", self.distro_name));
        lines.push(format!("Base Destination:     {}", self.snapshot.distro_dest.display()));
        lines.push(String::new());

        if let Some(st) = &self.snapshot.distro_status {
            lines.push(Style::new().bold().foreground(Palette::CYAN).render("─ Repository Metrics ─"));
            lines.push(format!("• Binary Packages:    {} (.rpm files in {}/x86_64)", st.binary_count, self.distro_name));
            lines.push(format!("• Source Packages:    {} (.src.rpm files in {}/source/SRPMS)", st.source_count, self.distro_name));
            lines.push(format!("• Metadata Present:   {}", if st.repodata_present { "Yes (repomd.xml active)" } else { "No (run 'dbs distro init')" }));
            lines.push(format!("• GPG Detached Sig:   {}", if st.gpg_signed { "Yes (repomd.xml.asc verified)" } else { "No (unsigned)" }));
            if let Some(p) = &st.client_repo_file {
                lines.push(format!("• Client Config:      {} generated", p.display()));
            }
        } else {
            lines.push(Style::new().foreground(Palette::YELLOW).render("Repository has not been initialized yet."));
            lines.push(format!("Run 'dbs distro init --name {}' to generate repository structures.", self.distro_name));
        }

        render_card("Distribution Repository Lifecycle", &lines.join("\n"), card_w, true)
    }

    /// Renders Tab 4: Lookaside CAS Details.
    fn render_lookaside_tab(&self, width: usize) -> String {
        let card_w = width.saturating_sub(4).max(50);
        let mut lines = Vec::new();

        if let Some(st) = &self.snapshot.lookaside_status {
            let mb = (st.total_cas_bytes as f64) / (1024.0 * 1024.0);
            lines.push(format!("• Lookaside Root:     {}", st.root.display()));
            lines.push(format!("• Filesystem Type:    {}", if st.is_btrfs { "BTRFS (Reflink CoW Deduplication Enabled)" } else { "Standard POSIX" }));
            lines.push(format!("• Total CAS Objects:  {}", st.total_cas_objects));
            lines.push(format!("• Total CAS Storage:  {:.2} MB ({} bytes)", mb, st.total_cas_bytes));
            lines.push(format!("• Tracked Packages:   {} packages in lookaside manifest", st.total_packages));
            lines.push(format!("• Total Package Files:{}", st.total_package_entries));
        } else {
            lines.push("Lookaside cache directory is not accessible.".to_string());
        }

        render_card("Lookaside Content-Addressable Storage (CAS)", &lines.join("\n"), card_w, true)
    }

    /// Renders Tab 5: Mock Chroots List.
    fn render_chroots_tab(&self, width: usize) -> String {
        let card_w = width.saturating_sub(4).max(50);
        let mut lines = Vec::new();

        if self.snapshot.chroots.is_empty() {
            lines.push("No Mock chroot profiles found.".to_string());
            lines.push(format!("Search paths: {}, /etc/mock", self.dbs_cfg.chroot.config_dir.as_deref().unwrap_or(Path::new("mock")).display()));
        } else {
            lines.push(format!("{:<28} {:<10} {:<8} {}", "Profile Name", "Target Arch", "Engine", "Path"));
            lines.push("─".repeat(card_w.saturating_sub(4)));
            for c in &self.snapshot.chroots {
                let arch = c.target_arch.as_deref().unwrap_or("x86_64");
                let engine = c.package_manager.as_deref().unwrap_or("dnf5");
                lines.push(format!("{:<28} {:<10} {:<8} {}", c.name, arch, engine, c.path.display()));
            }
        }

        render_card("Discovered Mock Chroot Profiles", &lines.join("\n"), card_w, true)
    }

    /// Renders Tab 6: Database Status.
    fn render_db_tab(&self, width: usize) -> String {
        let card_w = width.saturating_sub(4).max(50);
        let mut lines = Vec::new();

        if self.snapshot.db_connected {
            lines.push(format!("• Connection:       {}", render_badge("CONNECTED", BadgeKind::Success)));
            if let Some(masked) = &self.snapshot.db_url_masked {
                lines.push(format!("• Database URI:     {}", masked));
            }
            lines.push(format!("• Operating Systems:{}", self.snapshot.db_os_count));
            lines.push(format!("• Tracked Packages: {}", self.snapshot.db_pkg_count));
        } else {
            lines.push(format!("• Connection:       {}", render_badge("OFFLINE", BadgeKind::Dim)));
            lines.push("• Message:          Database is offline or DATABASE_URL is not set.".to_string());
            lines.push("• Fallback:         DBS operates in standalone mode without database.".to_string());
        }

        render_card("PostgreSQL Metadata & Build Records", &lines.join("\n"), card_w, true)
    }
}

/// Runs the interactive system monitor TUI dashboard using Bubbletea.
pub async fn run_monitor(dbs_cfg: DbsConfig, distro_name: Option<String>) -> Result<()> {
    let distro = distro_name.unwrap_or_else(|| dbs_cfg.distro.name.clone());
    let model = MonitorModel::new(dbs_cfg, distro);
    let program = Program::new(model).with_alt_screen();
    program.run_async().await.map_err(|e| eyre::eyre!("Monitor TUI exited with error: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_model_tabs_and_view() {
        let cfg = DbsConfig::default();
        let mut model = MonitorModel::new(cfg, "tacos".to_string());

        // Initial view is Overview
        assert_eq!(model.active_tab, 0);
        let view0 = model.view();
        assert!(view0.contains("DBS SYSTEM MONITOR"));
        assert!(view0.contains("Overview"));
        assert!(view0.contains("Live Build Pipeline Telemetry"));

        // Switch to Tab 2 (Builds) via '2' key
        model.update(Message::new(KeyMsg::from_char('2')));
        assert_eq!(model.active_tab, 1);
        let view1 = model.view();
        assert!(view1.contains("Package Build Pipeline"));

        // Switch to Tab 3 (Distro Repo) via Tab key
        model.update(Message::new(KeyMsg::from_type(KeyType::Tab)));
        assert_eq!(model.active_tab, 2);
        let view2 = model.view();
        assert!(view2.contains("Distribution Repository Lifecycle"));

        // Switch to Tab 4 (Lookaside) via '4' key
        model.update(Message::new(KeyMsg::from_char('4')));
        assert_eq!(model.active_tab, 3);
        let view3 = model.view();
        assert!(view3.contains("Lookaside Content-Addressable Storage"));

        // Switch to Tab 5 (Mock Chroots) via '5' key
        model.update(Message::new(KeyMsg::from_char('5')));
        assert_eq!(model.active_tab, 4);
        let view4 = model.view();
        assert!(view4.contains("Mock Chroot Profiles"));

        // Switch to Tab 6 (Database) via '6' key
        model.update(Message::new(KeyMsg::from_char('6')));
        assert_eq!(model.active_tab, 5);
        let view5 = model.view();
        assert!(view5.contains("PostgreSQL Metadata"));
    }

    #[test]
    fn test_monitor_builds_rendering() {
        let cfg = DbsConfig::default();
        let mut model = MonitorModel::new(cfg, "tacos".to_string());

        // Inject mock active and recent builds into snapshot
        model.snapshot.active_builds.push(Package {
            id: 101,
            name: "glibc".to_string(),
            epoch: 0,
            version: "2.41".to_string(),
            release: "1".to_string(),
            architecture: 1,
            package_size: "15 MB".to_string(),
            file_size_bytes: 15_000_000,
            source: "glibc.src.rpm".to_string(),
            repository: "build".to_string(),
            summary: "GNU C Library".to_string(),
            url: "https://gnu.org".to_string(),
            license: "LGPLv2+".to_string(),
            description: "Core C library".to_string(),
            in_repo: Some(false),
            created: Some(false),
            vulnerable: Some(false),
            build_status: BuildStatus::BUILDING,
            build_duration_seconds: None,
            build_log_path: Some("/tmp/glibc.log".to_string()),
            error_summary: None,
            worker_id: Some(3),
            sourcerpm: None,
            dist_git_url: None,
            dist_git_branch: None,
            dist_git_commit: None,
            spec_file: None,
        });

        model.snapshot.recent_builds.push(Package {
            id: 100,
            name: "systemd".to_string(),
            epoch: 0,
            version: "262".to_string(),
            release: "1".to_string(),
            architecture: 1,
            package_size: "25 MB".to_string(),
            file_size_bytes: 25_000_000,
            source: "systemd.src.rpm".to_string(),
            repository: "build".to_string(),
            summary: "System and Service Manager".to_string(),
            url: "https://systemd.io".to_string(),
            license: "LGPLv2+".to_string(),
            description: "Init system".to_string(),
            in_repo: Some(true),
            created: Some(true),
            vulnerable: Some(false),
            build_status: BuildStatus::SUCCESS,
            build_duration_seconds: Some(42.5),
            build_log_path: Some("/tmp/systemd.log".to_string()),
            error_summary: None,
            worker_id: Some(1),
            sourcerpm: None,
            dist_git_url: None,
            dist_git_branch: None,
            dist_git_commit: None,
            spec_file: None,
        });

        model.snapshot.build_counts = BuildCounts {
            building: 1,
            success: 1,
            failed: 0,
            pending: 0,
            total: 2,
        };

        // Render Overview tab
        let overview = model.render_overview(100);
        assert!(overview.contains("glibc"));
        assert!(overview.contains("(W3)"));
        assert!(overview.contains("1 BUILDING"));

        // Render Builds tab
        let builds_view = model.render_builds_tab(100);
        assert!(builds_view.contains("ACTIVE COMPILATION JOBS"));
        assert!(builds_view.contains("glibc"));
        assert!(builds_view.contains("Worker-3"));
        assert!(builds_view.contains("RECENT BUILD HISTORY"));
        assert!(builds_view.contains("systemd"));
        assert!(builds_view.contains("42.5s"));
    }
}
