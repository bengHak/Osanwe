# Interactive Pane Orchestrator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Build a testable Rust MVP that orchestrates native interactive Codex and Grok sessions in Zellij panes and coordinates them through a local daemon, MCP bridge, hooks, Git worktrees, checks, and a Ratatui control pane.

**Architecture:** A single Rust package is split into focused modules. A run-scoped daemon persists state and serves JSON-line RPC over a Unix socket; Zellij hosts all interactive panes; provider drivers generate native interactive commands and additive overlays; the MCP bridge submits typed workflow artifacts.

**Tech Stack:** Rust 2021, Tokio, Clap, Serde, Ratatui, Crossterm, UUID, SHA-2, TOML, Git CLI, Zellij CLI.

## Global Constraints

- Do not use `codex exec`, `grok -p`, Grok `--single`, Codex app-server, or Grok ACP.
- Do not call provider model APIs directly.
- Preserve base user configuration and add only namespaced overlays.
- Treat screen parsing as observational fallback, never as the completion or security boundary.
- Support macOS, Linux, and WSL in the MVP; fail clearly on non-Unix platforms.
- Require Zellij 0.44 or newer for pane IDs and subscriptions.
- Use test-first development for domain behavior and command construction.

---

### Task 1: Domain state and workflow transitions

**Files:** `src/model.rs`, `src/state.rs`

**Interfaces:** Produces `RunManifest`, `AgentRecord`, `PlanSpec`, `VerificationReport`, `WorkflowEvent`, and validated transition methods used by every later task.

- [x] Write failing transition and authorization tests.
- [x] Implement serializable domain types and transition rules.
- [x] Run focused tests and then the full test suite.

### Task 2: Zellij pane host

**Files:** `src/process.rs`, `src/zellij.rs`

**Interfaces:** Produces `CommandRunner`, `ZellijPaneHost`, `PaneSpec`, `PaneInfo`, pane creation, input injection, focus, listing, and snapshot APIs.

- [x] Write failing command-construction tests with a recording runner.
- [x] Implement shell-free Zellij CLI invocation and JSON decoding.
- [x] Run focused tests and the full test suite.

### Task 3: Provider drivers and overlays

**Files:** `src/provider.rs`

**Interfaces:** Produces native interactive Codex/Grok commands, resume commands, role bootstrap prompts, Codex profile overlays, and Grok plugin overlays.

- [x] Write failing tests proving prohibited non-interactive flags are absent and role flags are correct.
- [x] Implement Codex and Grok drivers plus overlay generation.
- [x] Run focused tests and the full test suite.

### Task 4: Persistence, IPC daemon, and role-scoped RPC

**Files:** `src/store.rs`, `src/ipc.rs`, `src/daemon.rs`

**Interfaces:** Produces atomic manifest persistence, NDJSON event append, Unix-socket request/response protocol, authentication, and workflow mutation handlers.

- [x] Write failing store round-trip and RPC authorization tests.
- [x] Implement run store, client, server, and daemon dispatch.
- [x] Run focused tests and the full test suite.

### Task 5: MCP bridge and lifecycle hook forwarder

**Files:** `src/mcp.rs`, `src/hook.rs`

**Interfaces:** Produces MCP `initialize`, `tools/list`, and `tools/call`; maps role tools to daemon RPC; forwards hook JSON while preserving native approval UI.

- [x] Write failing tool-list and tool-call mapping tests.
- [x] Implement line-delimited JSON-RPC stdio server and hook forwarding.
- [x] Run focused tests and the full test suite.

### Task 6: Git worktrees, checkpoints, integration, and checks

**Files:** `src/workspace.rs`, `src/checks.rs`

**Interfaces:** Produces repository preflight, integration/worker worktree creation, relay-owned checkpoint commits, cherry-pick integration, diff summaries, and deterministic check execution.

- [x] Write failing temporary-repository integration tests.
- [x] Implement Git command execution and check reporting.
- [x] Run focused tests and the full test suite.

### Task 7: CLI start flow and interactive pane lifecycle

**Files:** `src/cli.rs`, `src/pane.rs`, `src/main.rs`

**Interfaces:** Produces `start`, `attach`, `list`, `doctor`, `daemon`, `tui`, `pane`, `bridge`, `hook`, and `checks` subcommands.

- [x] Write failing argument and bootstrap tests.
- [x] Implement run initialization, daemon spawn, Zellij session/pane creation, prompt injection, and attach.
- [x] Run focused tests and the full test suite.

### Task 8: Orchestrator TUI

**Files:** `src/tui.rs`

**Interfaces:** Produces overview rendering, plan approval/rejection, pane focus, manual refresh, and attention display.

- [x] Write failing Ratatui buffer snapshot tests.
- [x] Implement event loop and daemon actions.
- [x] Run focused tests and the full test suite.

### Task 9: Documentation and CI

**Files:** `README.md`, `LICENSE`, `.github/workflows/ci.yml`, `.gitignore`, `rust-toolchain.toml`

**Interfaces:** Documents installation, prerequisites, workflow, security model, limitations, and live smoke-test procedure.

- [x] Add CI for formatting, clippy, tests, and release build.
- [x] Add user documentation and example commands.
- [x] Run formatting, clippy, tests, and release build.
