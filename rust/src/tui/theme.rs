use lipgloss::{Border, Style};

/// Badge display category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Success,
    Info,
    Warning,
    Error,
    Dim,
    Highlight,
}

/// DBS Color Palette
pub struct Palette;

impl Palette {
    pub const PURPLE: &'static str = "#7D56F4";
    pub const PURPLE_LIGHT: &'static str = "#9D7BFC";
    pub const CYAN: &'static str = "#00D2FF";
    pub const CYAN_LIGHT: &'static str = "#5CE1E6";
    pub const GREEN: &'static str = "#50FA7B";
    pub const YELLOW: &'static str = "#F1FA8C";
    pub const ORANGE: &'static str = "#FFB86C";
    pub const RED: &'static str = "#FF5555";
    pub const WHITE: &'static str = "#F8F8F2";
    pub const BG_DARK: &'static str = "#1E1E2E";
    pub const BG_CARD: &'static str = "#282A36";
    pub const BORDER_MUTED: &'static str = "#44475A";
    pub const TEXT_MUTED: &'static str = "#6272A4";
}

/// Central Lipgloss Theme for DBS interactive interfaces.
#[derive(Debug, Clone)]
pub struct Theme {
    pub title: Style,
    pub subtitle: Style,
    pub primary: Style,
    pub secondary: Style,
    pub success: Style,
    pub warning: Style,
    pub error: Style,
    pub muted: Style,
    pub card: Style,
    pub card_active: Style,
    pub tab_active: Style,
    pub tab_inactive: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            title: Style::new()
                .bold()
                .foreground(Palette::WHITE)
                .background(Palette::PURPLE)
                .padding_left(1)
                .padding_right(1),
            subtitle: Style::new()
                .foreground(Palette::CYAN)
                .padding_left(1),
            primary: Style::new().bold().foreground(Palette::PURPLE),
            secondary: Style::new().bold().foreground(Palette::CYAN),
            success: Style::new().bold().foreground(Palette::GREEN),
            warning: Style::new().bold().foreground(Palette::YELLOW),
            error: Style::new().bold().foreground(Palette::RED),
            muted: Style::new().foreground(Palette::TEXT_MUTED),
            card: Style::new()
                .border(Border::rounded())
                .border_foreground(Palette::BORDER_MUTED)
                .padding_left(1)
                .padding_right(1),
            card_active: Style::new()
                .border(Border::rounded())
                .border_foreground(Palette::CYAN)
                .padding_left(1)
                .padding_right(1),
            tab_active: Style::new()
                .bold()
                .foreground(Palette::WHITE)
                .background(Palette::PURPLE)
                .padding_left(1)
                .padding_right(1),
            tab_inactive: Style::new()
                .foreground(Palette::TEXT_MUTED)
                .padding_left(1)
                .padding_right(1),
        }
    }
}

/// Renders a standardized badge.
pub fn render_badge(text: &str, kind: BadgeKind) -> String {
    let style = match kind {
        BadgeKind::Success => Style::new()
            .bold()
            .foreground(Palette::BG_DARK)
            .background(Palette::GREEN)
            .padding_left(1)
            .padding_right(1),
        BadgeKind::Info => Style::new()
            .bold()
            .foreground(Palette::BG_DARK)
            .background(Palette::CYAN)
            .padding_left(1)
            .padding_right(1),
        BadgeKind::Warning => Style::new()
            .bold()
            .foreground(Palette::BG_DARK)
            .background(Palette::YELLOW)
            .padding_left(1)
            .padding_right(1),
        BadgeKind::Error => Style::new()
            .bold()
            .foreground(Palette::WHITE)
            .background(Palette::RED)
            .padding_left(1)
            .padding_right(1),
        BadgeKind::Dim => Style::new()
            .foreground(Palette::WHITE)
            .background(Palette::BORDER_MUTED)
            .padding_left(1)
            .padding_right(1),
        BadgeKind::Highlight => Style::new()
            .bold()
            .foreground(Palette::WHITE)
            .background(Palette::PURPLE)
            .padding_left(1)
            .padding_right(1),
    };
    style.render(text)
}

/// Renders a top header bar with branding and subtitle.
pub fn render_header(title: &str, subtitle: &str, width: usize) -> String {
    let theme = Theme::default();
    let title_rendered = theme.title.render(title);
    let sub_rendered = theme.subtitle.render(subtitle);
    let header_line = format!("{} {}", title_rendered, sub_rendered);
    
    let total_w = width.max(40);
    let border_style = Style::new()
        .foreground(Palette::BORDER_MUTED)
        .width(total_w as u16);
    let line = "─".repeat(total_w.saturating_sub(2));
    
    format!("{}\n{}\n", header_line, border_style.render(&line))
}

/// Renders navigation tabs with an active indicator.
pub fn render_tabs(tabs: &[&str], active_idx: usize, _width: usize) -> String {
    let theme = Theme::default();
    let mut parts = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i == active_idx {
            parts.push(theme.tab_active.render(&format!("[{}] {}", i + 1, tab)));
        } else {
            parts.push(theme.tab_inactive.render(&format!(" {}  {}", i + 1, tab)));
        }
    }
    format!("{}\n", parts.join(" "))
}

/// Renders a styled bordered card with a title.
pub fn render_card(title: &str, content: &str, width: usize, active: bool) -> String {
    let theme = Theme::default();
    let title_style = if active {
        Style::new().bold().foreground(Palette::CYAN)
    } else {
        Style::new().bold().foreground(Palette::WHITE)
    };
    
    let header = title_style.render(title);
    let inner = format!("{}\n\n{}", header, content);
    
    let card_style = if active {
        theme.card_active.width(width as u16)
    } else {
        theme.card.width(width as u16)
    };
    
    card_style.render(&inner)
}

/// Renders a footer with keyboard shortcuts and optional status/toast notification.
pub fn render_footer(shortcuts: &[(&str, &str)], status_msg: Option<&str>, width: usize) -> String {
    let key_style = Style::new().bold().foreground(Palette::YELLOW);
    let desc_style = Style::new().foreground(Palette::TEXT_MUTED);
    let toast_style = Style::new().bold().foreground(Palette::GREEN).padding_left(1);
    
    let mut shortcut_strs = Vec::new();
    for (key, desc) in shortcuts {
        shortcut_strs.push(format!("{}{}", key_style.render(key), desc_style.render(&format!(":{}", desc))));
    }
    let shortcuts_line = shortcut_strs.join("  ");
    
    let total_w = width.max(40);
    let line = "─".repeat(total_w.saturating_sub(2));
    let border_style = Style::new().foreground(Palette::BORDER_MUTED);
    
    let status_line = if let Some(msg) = status_msg {
        toast_style.render(msg)
    } else {
        String::new()
    };
    
    format!("{}\n{}\n{}", border_style.render(&line), shortcuts_line, status_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_badges() {
        let ok = render_badge("READY", BadgeKind::Success);
        assert!(!ok.is_empty());
        let err = render_badge("FAIL", BadgeKind::Error);
        assert!(!err.is_empty());
    }

    #[test]
    fn test_theme_header_and_card() {
        let hdr = render_header("DBS", "TacOS Builder", 80);
        assert!(hdr.contains("DBS"));
        let card = render_card("Status", "All systems operational", 40, true);
        assert!(card.contains("Status"));
    }
}
