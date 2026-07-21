//! Osanwe orchestrates native interactive Codex and Grok Build sessions in Zellij panes.

pub mod checks;
pub mod cli;
pub mod daemon;
pub mod hook;
pub mod ipc;
pub mod mcp;
pub mod model;
pub mod pane;
pub mod process;
pub mod provider;
pub mod state;
pub mod store;
pub mod tui;
pub mod workspace;
pub mod zellij;
