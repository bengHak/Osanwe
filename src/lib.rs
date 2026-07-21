//! Osanwe launches native interactive Codex and Grok Build sessions in Zellij panes,
//! coordinating through a project-local `.osanwe/` file bus.

pub mod checks;
pub mod cli;
pub mod daemon;
pub mod hook;
pub mod ipc;
pub mod mcp;
pub mod model;
pub mod onboard;
pub mod pane;
pub mod process;
pub mod project;
pub mod provider;
pub mod session_launch;
pub mod state;
pub mod store;
pub mod tui;
pub mod workspace;
pub mod zellij;
