//! Interactive TUI Package Explorer for DBS using Charmed Rust.
//!
//! Provides a full-screen, searchable package explorer to inspect dist-git
//! repositories, view spec details, and trigger one-key cloning/pulling.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use bubbletea::{Cmd, KeyMsg, KeyType, Message, Model, Program, WindowSizeMsg, quit};
use eyre::Result;
use lipgloss::{Position, Style};

use crate::distgit::client::{DiscoveredProject, DistGitClient};
use crate::distgit::provider::DistroConfig;
use crate::distgit::spec::{parse_spec_file, SpecMetadata};
use crate::tui::theme::{render_badge, render_card, render_footer, render_header, BadgeKind, Palette};

/// Message received when a package clone or pull completes.
struct CloneResultMsg {
    package: String,
    result: Result<SpecMetadata, String>,
}

/// Message received when switching distributions or refreshing packages.
struct FetchResultMsg {
    distro_name: String,
    projects: Result<Vec<DiscoveredProject>, String>,
}

/// Interactive Package Explorer Model for Bubbletea.
pub struct ExplorerModel {
    /// Active distribution configuration preset.
    distro: DistroConfig,
    /// Destination root directory for cloned RPM packages (e.g. `/srv/dbs/tacos/rpm`).
    target_rpm_dir: PathBuf,
    /// All discovered projects for this distribution.
    projects: Vec<DiscoveredProject>,
    /// Filtered indices into `projects` matching `search_query`.
    filtered_indices: Vec<usize>,
    /// Currently selected item index in `filtered_indices`.
    cursor: usize,
    /// Viewport scroll offset.
    scroll_offset: usize,
    /// Current search query string.
    search_query: String,
    /// Whether user is currently typing in search input mode.
    is_searching: bool,
    /// Local packages that exist on disk in `target_rpm_dir`.
    cloned_packages: HashSet<String>,
    /// Cached spec metadata for current selection if cloned.
    cached_spec: Option<(String, Option<SpecMetadata>)>,
    /// Transient status or toast notification message.
    status_message: Option<String>,
    /// Terminal width.
    width: usize,
    /// Terminal height.
    height: usize,
}

impl ExplorerModel {
    /// Creates a new ExplorerModel with initial projects and config.
    pub fn new(
        distro: DistroConfig,
        target_rpm_dir: PathBuf,
        projects: Vec<DiscoveredProject>,
        initial_search: Option<&str>,
    ) -> Self {
        let mut cloned = HashSet::new();
        if target_rpm_dir.exists() {
            if let Ok(entries) = fs::read_dir(&target_rpm_dir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_dir() {
                            cloned.insert(entry.file_name().to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        let search_query = initial_search.unwrap_or("").to_string();
        let mut model = Self {
            distro,
            target_rpm_dir,
            projects,
            filtered_indices: Vec::new(),
            cursor: 0,
            scroll_offset: 0,
            search_query,
            is_searching: false,
            cloned_packages: cloned,
            cached_spec: None,
            status_message: None,
            width: 100,
            height: 30,
        };

        model.apply_filter();
        model.update_cached_spec();
        model
    }

    /// Recomputes `filtered_indices` based on `search_query`.
    fn apply_filter(&mut self) {
        let q = self.search_query.trim().to_lowercase();
        if q.is_empty() {
            self.filtered_indices = (0..self.projects.len()).collect();
        } else {
            self.filtered_indices = self.projects
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.name.to_lowercase().contains(&q)
                        || p.description.as_deref().unwrap_or("").to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }

        if self.cursor >= self.filtered_indices.len() {
            self.cursor = self.filtered_indices.len().saturating_sub(1);
        }
        self.scroll_offset = 0;
        self.update_cached_spec();
    }

    /// Retrieves currently selected project.
    pub fn selected_project(&self) -> Option<&DiscoveredProject> {
        self.filtered_indices
            .get(self.cursor)
            .and_then(|&idx| self.projects.get(idx))
    }

    /// Refreshes the cached `.spec` metadata for the currently selected package.
    fn update_cached_spec(&mut self) {
        let Some(p) = self.selected_project() else {
            self.cached_spec = None;
            return;
        };

        let pkg_name = p.name.clone();
        if let Some((cached_name, _)) = &self.cached_spec {
            if cached_name == &pkg_name {
                return;
            }
        }

        let pkg_dir = self.target_rpm_dir.join(&pkg_name);
        if pkg_dir.exists() {
            let spec_path = pkg_dir.join(format!("{}.spec", pkg_name));
            if spec_path.exists() {
                let meta = parse_spec_file(&spec_path).ok();
                self.cached_spec = Some((pkg_name, meta));
                return;
            }
        }

        self.cached_spec = Some((pkg_name, None));
    }

    /// Spawns a background task to clone or pull the selected package.
    fn clone_selected_package(&mut self) -> Option<Cmd> {
        let Some(p) = self.selected_project() else {
            self.status_message = Some("No package selected to clone".to_string());
            return None;
        };

        let pkg_name = p.name.clone();
        let target_dir = self.target_rpm_dir.clone();
        let distro_cfg = self.distro.clone();

        self.status_message = Some(format!("Cloning '{}' into {}...", pkg_name, target_dir.display()));

        Some(Cmd::blocking(move || {
            let client = DistGitClient::new(distro_cfg);
            match client.clone_or_pull(&pkg_name, &target_dir) {
                Ok(status) => Message::new(CloneResultMsg {
                    package: pkg_name,
                    result: Ok(status.spec_meta),
                }),
                Err(err) => Message::new(CloneResultMsg {
                    package: pkg_name,
                    result: Err(err.to_string()),
                }),
            }
        }))
    }

    /// Cycles to the next distribution preset.
    fn cycle_distro(&mut self) -> Option<Cmd> {
        let presets = ["fedora-rawhide", "centos-stream-10", "centos-stream-9", "tacos"];
        let current = self.distro.name.as_str();
        let next_idx = match presets.iter().position(|&p| p == current) {
            Some(idx) => (idx + 1) % presets.len(),
            None => 0,
        };
        let next_name = presets[next_idx];
        let next_cfg = DistroConfig::from_preset(next_name)?;

        self.distro = next_cfg.clone();
        self.status_message = Some(format!("Fetching packages for distribution '{}'...", next_name));

        Some(Cmd::blocking(move || {
            let rt = tokio::runtime::Handle::current();
            let client = DistGitClient::new(next_cfg);
            let res = rt.block_on(client.explore(None, Some(100)));
            Message::new(FetchResultMsg {
                distro_name: next_name.to_string(),
                projects: res.map_err(|e| e.to_string()),
            })
        }))
    }
}

impl Model for ExplorerModel {
    fn init(&self) -> Option<Cmd> {
        None
    }

    fn update(&mut self, msg: Message) -> Option<Cmd> {
        if let Some(size) = msg.downcast_ref::<WindowSizeMsg>() {
            self.width = (size.width as usize).max(40);
            self.height = (size.height as usize).max(15);
            return None;
        }

        if let Some(clone_msg) = msg.downcast_ref::<CloneResultMsg>() {
            match &clone_msg.result {
                Ok(meta) => {
                    self.cloned_packages.insert(clone_msg.package.clone());
                    self.status_message = Some(format!(
                        "✓ Cloned '{}' ({}-{}) into {}",
                        clone_msg.package,
                        meta.version,
                        meta.release,
                        self.target_rpm_dir.display()
                    ));
                    self.cached_spec = Some((clone_msg.package.clone(), Some(meta.clone())));
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Clone failed for '{}': {}", clone_msg.package, e));
                }
            }
            return None;
        }

        if let Some(fetch_msg) = msg.downcast_ref::<FetchResultMsg>() {
            match &fetch_msg.projects {
                Ok(projs) => {
                    self.status_message = Some(format!("✓ Loaded {} packages from {}", projs.len(), fetch_msg.distro_name));
                    self.projects = projs.clone();
                    self.apply_filter();
                }
                Err(e) => {
                    self.status_message = Some(format!("✗ Failed to load {}: {}", fetch_msg.distro_name, e));
                }
            }
            return None;
        }

        if let Some(key) = msg.downcast_ref::<KeyMsg>() {
            if self.is_searching {
                match key.key_type {
                    KeyType::Esc | KeyType::Enter => {
                        self.is_searching = false;
                    }
                    KeyType::Backspace => {
                        self.search_query.pop();
                        self.apply_filter();
                    }
                    KeyType::Runes => {
                        for c in &key.runes {
                            self.search_query.push(*c);
                        }
                        self.apply_filter();
                    }
                    KeyType::CtrlC => {
                        return Some(quit());
                    }
                    _ => {}
                }
                return None;
            }

            match key.key_type {
                KeyType::CtrlC => return Some(quit()),
                KeyType::Esc => {
                    if !self.search_query.is_empty() {
                        self.search_query.clear();
                        self.apply_filter();
                    } else {
                        return Some(quit());
                    }
                }
                KeyType::Up => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        if self.cursor < self.scroll_offset {
                            self.scroll_offset = self.cursor;
                        }
                        self.update_cached_spec();
                    }
                }
                KeyType::Down => {
                    if self.cursor + 1 < self.filtered_indices.len() {
                        self.cursor += 1;
                        let visible_rows = self.height.saturating_sub(12).max(5);
                        if self.cursor >= self.scroll_offset + visible_rows {
                            self.scroll_offset = self.cursor.saturating_sub(visible_rows - 1);
                        }
                        self.update_cached_spec();
                    }
                }
                KeyType::PgUp => {
                    let page = self.height.saturating_sub(12).max(5);
                    self.cursor = self.cursor.saturating_sub(page);
                    self.scroll_offset = self.scroll_offset.saturating_sub(page);
                    self.update_cached_spec();
                }
                KeyType::PgDown => {
                    let page = self.height.saturating_sub(12).max(5);
                    let max_idx = self.filtered_indices.len().saturating_sub(1);
                    self.cursor = (self.cursor + page).min(max_idx);
                    self.scroll_offset = (self.scroll_offset + page).min(max_idx);
                    self.update_cached_spec();
                }
                KeyType::Home => {
                    self.cursor = 0;
                    self.scroll_offset = 0;
                    self.update_cached_spec();
                }
                KeyType::End => {
                    let max_idx = self.filtered_indices.len().saturating_sub(1);
                    self.cursor = max_idx;
                    let visible_rows = self.height.saturating_sub(12).max(5);
                    self.scroll_offset = max_idx.saturating_sub(visible_rows - 1);
                    self.update_cached_spec();
                }
                KeyType::Runes => {
                    if let Some(&ch) = key.runes.first() {
                        match ch {
                            'q' | 'Q' => return Some(quit()),
                            '/' => {
                                self.is_searching = true;
                            }
                            'c' | 'C' => {
                                return self.clone_selected_package();
                            }
                            's' | 'S' => {
                                return self.cycle_distro();
                            }
                            'k' => {
                                if self.cursor > 0 {
                                    self.cursor -= 1;
                                    if self.cursor < self.scroll_offset {
                                        self.scroll_offset = self.cursor;
                                    }
                                    self.update_cached_spec();
                                }
                            }
                            'j' => {
                                if self.cursor + 1 < self.filtered_indices.len() {
                                    self.cursor += 1;
                                    let visible_rows = self.height.saturating_sub(12).max(5);
                                    if self.cursor >= self.scroll_offset + visible_rows {
                                        self.scroll_offset = self.cursor.saturating_sub(visible_rows - 1);
                                    }
                                    self.update_cached_spec();
                                }
                            }
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
        let height = self.height.max(18);

        let header = render_header(
            "🍱 DBS REMOTE PACKAGE EXPLORER",
            &format!("Distribution: {} ({})  •  Target: {}", self.distro.name, self.distro.dist_git_branch, self.target_rpm_dir.display()),
            width,
        );

        // Search bar
        let search_style = if self.is_searching {
            Style::new().bold().foreground(Palette::YELLOW)
        } else {
            Style::new().foreground(Palette::TEXT_MUTED)
        };
        let search_row = if self.is_searching {
            format!("🔍 Filter: {}▏ (Press Enter or Esc to finish)\n", search_style.render(&self.search_query))
        } else {
            let filter_display = if self.search_query.is_empty() {
                "all packages"
            } else {
                &self.search_query
            };
            format!("🔍 Filter: [{}] (Press '/' to search, 'c' to clone, 's' to switch distro)\n", search_style.render(filter_display))
        };

        // Two-pane layout
        let left_w = (width * 38 / 100).max(30).min(45);
        let right_w = width.saturating_sub(left_w + 3).max(30);
        let list_h = height.saturating_sub(10).max(6);

        // Left Pane: Package List
        let mut list_rows = Vec::new();
        let end_idx = (self.scroll_offset + list_h).min(self.filtered_indices.len());
        
        if self.filtered_indices.is_empty() {
            list_rows.push(Style::new().foreground(Palette::TEXT_MUTED).render("  No matching packages found."));
        } else {
            for i in self.scroll_offset..end_idx {
                let proj_idx = self.filtered_indices[i];
                let p = &self.projects[proj_idx];
                let is_selected = i == self.cursor;
                let is_cloned = self.cloned_packages.contains(&p.name);

                let badge = if is_cloned {
                    render_badge("CLONED", BadgeKind::Success)
                } else {
                    render_badge("REMOTE", BadgeKind::Dim)
                };

                let name_truncated = if p.name.len() > 16 {
                    format!("{}…", &p.name[..15])
                } else {
                    p.name.clone()
                };

                let row_str = if is_selected {
                    let cursor_sym = Style::new().bold().foreground(Palette::CYAN).render("▸ ");
                    let name_styled = Style::new().bold().foreground(Palette::WHITE).render(&format!("{:<16}", name_truncated));
                    format!("{}{}{}", cursor_sym, name_styled, badge)
                } else {
                    let pad = "  ";
                    let name_styled = Style::new().foreground(Palette::TEXT_MUTED).render(&format!("{:<16}", name_truncated));
                    format!("{}{}{}", pad, name_styled, badge)
                };
                list_rows.push(row_str);
            }
        }

        let list_content = list_rows.join("\n");
        let left_card_title = format!("Packages ({}/{})", self.filtered_indices.len(), self.projects.len());
        let left_card = render_card(&left_card_title, &list_content, left_w, false);

        // Right Pane: Details Card
        let right_content = if let Some(p) = self.selected_project() {
            let is_cloned = self.cloned_packages.contains(&p.name);
            let mut lines = Vec::new();

            lines.push(format!("• Name:        {}", Style::new().bold().foreground(Palette::CYAN).render(&p.name)));
            lines.push(format!("• Preset:      {} ({})", self.distro.name, self.distro.dist_git_branch));
            lines.push(format!("• Git Remote:  {}", Style::new().foreground(Palette::PURPLE_LIGHT).render(&p.git_url)));
            if let Some(web) = &p.web_url {
                lines.push(format!("• Web URL:     {}", Style::new().foreground(Palette::TEXT_MUTED).render(web)));
            }
            if let Some(desc) = &p.description {
                let trimmed = desc.trim();
                if !trimmed.is_empty() {
                    lines.push(format!("• Summary:     {}", trimmed));
                }
            }

            lines.push(String::new());
            lines.push(Style::new().bold().foreground(Palette::YELLOW).render("─ Local RPM Repository Status ─"));

            if is_cloned {
                let local_path = self.target_rpm_dir.join(&p.name);
                lines.push(format!("• Local Path:  {}", local_path.display()));
                if let Some((_, Some(meta))) = &self.cached_spec {
                    lines.push(format!("• Version:     {}-{}", meta.version, meta.release));
                    lines.push(format!("• License:     {}", meta.license));
                    lines.push(format!("• Upstream:    {}", meta.url));
                    lines.push(format!("• BuildReqs:   {} defined", meta.build_requires.len()));
                    lines.push(format!("• Requires:    {} defined", meta.requires.len()));
                    if !meta.sources.is_empty() {
                        lines.push(format!("• Source0:     {}", meta.sources[0]));
                    }
                } else {
                    lines.push("• Spec File:   Cloned (metadata not yet loaded)".to_string());
                }
            } else {
                lines.push(format!("• Status:      {}", render_badge("NOT CLONED", BadgeKind::Warning)));
                lines.push("• Action:      Press [c] to clone repository and parse .spec file.".to_string());
            }

            lines.join("\n")
        } else {
            "Select a package from the left pane to view details.".to_string()
        };

        let right_card = render_card("Package Details & Spec Metadata", &right_content, right_w, true);

        let content_split = lipgloss::join_horizontal(Position::Top, &[&left_card, &right_card]);

        let shortcuts = [
            ("↑/↓, j/k", "Navigate"),
            ("/", "Filter"),
            ("c", "Clone Package"),
            ("s", "Switch Distro"),
            ("q/Esc", "Quit"),
        ];
        let footer = render_footer(&shortcuts, self.status_message.as_deref(), width);

        format!("{}{}\n{}\n{}", header, search_row, content_split, footer)
    }
}

/// Runs the interactive explorer TUI using Bubbletea.
pub async fn run_explorer(
    distro: DistroConfig,
    target_rpm_dir: PathBuf,
    projects: Vec<DiscoveredProject>,
    initial_search: Option<&str>,
) -> Result<()> {
    let model = ExplorerModel::new(distro, target_rpm_dir, projects, initial_search);
    let program = Program::new(model).with_alt_screen();
    program.run_async().await.map_err(|e| eyre::eyre!("Explorer TUI exited with error: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_projects() -> Vec<DiscoveredProject> {
        vec![
            DiscoveredProject {
                name: "glibc".to_string(),
                description: Some("The GNU libc libraries".to_string()),
                web_url: Some("https://src.fedoraproject.org/rpms/glibc".to_string()),
                git_url: "https://src.fedoraproject.org/rpms/glibc.git".to_string(),
            },
            DiscoveredProject {
                name: "kernel".to_string(),
                description: Some("The Linux kernel".to_string()),
                web_url: Some("https://src.fedoraproject.org/rpms/kernel".to_string()),
                git_url: "https://src.fedoraproject.org/rpms/kernel.git".to_string(),
            },
            DiscoveredProject {
                name: "strace".to_string(),
                description: Some("System call tracer".to_string()),
                web_url: Some("https://src.fedoraproject.org/rpms/strace".to_string()),
                git_url: "https://src.fedoraproject.org/rpms/strace.git".to_string(),
            },
        ]
    }

    #[test]
    fn test_explorer_model_navigation() {
        let distro = DistroConfig::from_preset("fedora-rawhide").unwrap();
        let mut model = ExplorerModel::new(distro, PathBuf::from("/tmp"), sample_projects(), None);

        assert_eq!(model.filtered_indices.len(), 3);
        assert_eq!(model.selected_project().unwrap().name, "glibc");

        // Navigate Down
        model.update(Message::new(KeyMsg::from_type(KeyType::Down)));
        assert_eq!(model.selected_project().unwrap().name, "kernel");

        // Navigate Down again
        model.update(Message::new(KeyMsg::from_type(KeyType::Down)));
        assert_eq!(model.selected_project().unwrap().name, "strace");

        // Navigate Up
        model.update(Message::new(KeyMsg::from_type(KeyType::Up)));
        assert_eq!(model.selected_project().unwrap().name, "kernel");

        // View rendering
        let rendered = model.view();
        assert!(rendered.contains("DBS REMOTE PACKAGE EXPLORER"));
        assert!(rendered.contains("kernel"));
        assert!(rendered.contains("glibc"));
    }

    #[test]
    fn test_explorer_model_filtering() {
        let distro = DistroConfig::from_preset("fedora-rawhide").unwrap();
        let mut model = ExplorerModel::new(distro, PathBuf::from("/tmp"), sample_projects(), None);

        // Enter search mode by sending '/'
        model.update(Message::new(KeyMsg::from_char('/')));
        assert!(model.is_searching);

        // Type 's', 't', 'r'
        model.update(Message::new(KeyMsg::from_char('s')));
        model.update(Message::new(KeyMsg::from_char('t')));
        model.update(Message::new(KeyMsg::from_char('r')));

        assert_eq!(model.filtered_indices.len(), 1);
        assert_eq!(model.selected_project().unwrap().name, "strace");

        // Exit search mode with Enter
        model.update(Message::new(KeyMsg::from_type(KeyType::Enter)));
        assert!(!model.is_searching);

        // View rendering
        let view = model.view();
        assert!(view.contains("strace"));
        assert!(!view.contains("glibc"));
    }
}
