# Osanwe Interactive Pane Orchestrator Design

## Goal

Osanwe is a Rust terminal workspace that creates a Zellij grid containing an orchestrator pane and native interactive Codex CLI and Grok Build panes. Osanwe coordinates planning, implementation, deterministic checks, and independent verification without using non-interactive model invocations, Codex app-server, or Grok ACP.

## Core constraints

- Every LLM session is a real interactive `codex` or `grok` process in a Zellij pane.
- Osanwe never calls the OpenAI or xAI model APIs directly.
- Codex planner and verifier use separate sessions.
- Grok worker sessions remain alive between assignments and may be resumed after a crash.
- Existing user MCP servers, skills, plugins, hooks, authentication, and memory remain available.
- Osanwe adds a role-scoped local MCP bridge and lifecycle hooks as overlays.
- Structured workflow completion comes from MCP submissions, Git state, and deterministic checks—not terminal text scraping.
- Pane rendering is observed only for attention and recovery hints.
- Original working trees are not modified; run work occurs in Git worktrees.

## MVP architecture

- `osanwe start` initializes a run, worktree, background Zellij session, daemon, orchestrator pane, and Codex planner pane.
- `osanwe daemon` owns persisted run state and a user-only Unix socket.
- `osanwe tui` renders workflow, agents, plan, checks, and attention state in the orchestrator pane.
- `osanwe pane` bootstraps a provider process with role-scoped environment and provider overlays.
- `osanwe bridge` is an MCP stdio server exposing assignment/result tools backed by daemon RPC.
- `osanwe hook` forwards Codex/Grok hook JSON to the daemon without deciding approvals.
- `osanwe checks` runs approved deterministic checks visibly in a Zellij pane.
- Zellij is controlled through its CLI pane-ID APIs; Osanwe does not implement a terminal emulator.

## Workflow

1. Create integration worktree and Zellij session.
2. Start independent Codex planner pane and inject a bootstrap prompt.
3. Planner reads its assignment through MCP and submits a typed plan.
4. User approves the plan in the orchestrator TUI.
5. Daemon creates a worker worktree and interactive Grok worker pane.
6. Worker implements and submits assignment completion through MCP.
7. Daemon checkpoints worker changes and integrates them into the integration branch.
8. A visible checks pane runs plan checks.
9. A fresh interactive Codex verifier pane reviews the task, plan, diff, and check results.
10. Verification passes the run or creates a repair assignment for the existing Grok worker.
