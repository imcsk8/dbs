//! Interactive Terminal User Interface (TUI) for DBS using Charmed Rust.
//!
//! Provides rich, full-screen interactive dashboards and package browsers using
//! `charmed-bubbletea`, `charmed-lipgloss`, and `charmed-bubbles`.

pub mod theme;
pub mod explorer;
pub mod monitor;

pub use explorer::run_explorer;
pub use monitor::run_monitor;
