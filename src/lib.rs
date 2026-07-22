//! Osanwe launches native interactive Codex and Grok Build sessions in Zellij panes,
//! coordinating through a project-local `.osanwe/` file bus.

pub mod cli;
pub mod onboard;
pub mod process;
pub mod project;
pub mod session_launch;
pub mod zellij;
