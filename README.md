# Osanwe

Osanwe is a Rust terminal orchestrator for **native interactive** Codex CLI and Grok Build sessions. It creates a Zellij workspace whose panes are real provider TUIs, while a local daemon coordinates planning, implementation, deterministic checks, and independent verification.

Osanwe does not call model APIs directly and does not use `codex exec`, Grok `-p`/`--single`, Codex app-server, or Grok ACP.

## Status

This repository contains the first functional MVP. The orchestration state machine, Zellij pane control, native provider launchers, additive Codex/Grok overlays, role-scoped MCP bridge, hook forwarding, Git worktree integration, visible checks pane, and Ratatui orchestrator are implemented.

The MVP targets Linux, macOS, and WSL. Zellij 0.44 or newer is required for pane-ID automation.

## Prerequisites

- Linux, macOS, or WSL
- Git
- Zellij 0.44 or newer
- Codex CLI, already authenticated
- Grok Build CLI, already authenticated
- `curl` and `tar` for binary installation

Osanwe deliberately reuses each client's existing authentication, configuration, MCP servers, skills, plugins, hooks, and session storage.

## Install with curl

This repository is private, so export a GitHub token that can read the repository and run:

```bash
export OSANWE_GITHUB_TOKEN=github_pat_...
curl -fsSL \
  -H "Authorization: Bearer $OSANWE_GITHUB_TOKEN" \
  -H "Accept: application/vnd.github.raw+json" \
  https://api.github.com/repos/bengHak/Osanwe/contents/install.sh \
  | sh
```

The installer downloads the release archive for the current operating system and CPU, verifies it against the published `SHA256SUMS`, and installs `osanwe` to `~/.local/bin`.

When the repository is public, the shorter command works without a token:

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/Osanwe/main/install.sh | sh
```

Installation options:

```bash
# Install a specific release.
OSANWE_VERSION=v0.1.0 sh install.sh

# Install somewhere already on PATH.
OSANWE_INSTALL_DIR=/usr/local/bin sh install.sh

# Print the detected release target without installing.
sh install.sh --print-target
```

After installation:

```bash
export PATH="$HOME/.local/bin:$PATH"
osanwe doctor
```

`osanwe doctor` checks that `git`, `zellij`, `codex`, and `grok` are executable.

Developers can still build from source with Rust 1.82 or newer:

```bash
cargo install --path .
```

## Start a run

Run Osanwe from a clean Git repository:

```bash
osanwe start "Implement refresh-token rotation and add regression tests"
```

To create the background Zellij session without immediately attaching:

```bash
osanwe start "Implement refresh-token rotation" --no-attach
osanwe list
osanwe attach <run-id>
```

A new run creates:

1. an isolated integration worktree;
2. a background Zellij session;
3. an Osanwe orchestrator pane;
4. a native interactive Codex planner pane.

When the planner submits a structured plan through the Osanwe MCP bridge, press `a` in the orchestrator pane to approve it. Osanwe then creates a native interactive Grok worker pane. After the worker submits completion, Osanwe checkpoints and integrates the worker branch, opens a visible checks pane, and finally starts a fresh native interactive Codex verifier pane.

## Orchestrator controls

| Key | Action |
| --- | --- |
| `a` | Approve a submitted plan |
| `Enter` | Focus the selected agent pane and hand control to the user |
| `m` | Toggle the selected pane between user and orchestrator control |
| `j` / `k` | Select next / previous agent |
| `r` | Refresh state |
| `q` | Close only the orchestrator TUI |

Closing the orchestrator pane does not terminate provider panes or the run daemon. Reopen it with `osanwe attach <run-id>`.

## Interactive provider contract

The actual provider commands are equivalent to:

```bash
# Planner and verifier
codex --cd <worktree> --profile <osanwe-profile> --sandbox read-only

# Worker
grok --cwd <worker-worktree> --plugin-dir <osanwe-plugin> --session-id <uuid>
```

Resume commands use native interactive resume modes:

```bash
codex ... resume <session-id>
grok ... --resume <session-id>
```

No initial prompt is passed as a CLI argument. Osanwe waits for the pane session and injects the bootstrap message through Zellij's pane-targeted paste and key APIs.

## Structured coordination

Osanwe adds a local MCP server named `osanwe` without replacing existing client extensions. Tools are role-scoped:

- Planner: `get_assignment`, `report_progress`, `request_input`, `submit_plan`
- Worker: `get_assignment`, `report_progress`, `request_input`, `report_deviation`, `complete_assignment`
- Verifier: `get_assignment`, `report_progress`, `request_input`, `submit_verification`

Lifecycle hooks provide state hints such as session start, tool activity, approval requests, and stop events. Hook failures are not treated as a security boundary, and the hook forwarder intentionally emits no approval decision so the native provider approval UI remains visible.

## Workspace and verification model

- The original working tree must have no tracked modifications.
- Integration and worker work happen in separate Git worktrees.
- Osanwe creates relay-owned checkpoint commits on worker branches.
- Worker commits are cherry-picked into the integration worktree.
- Checks are shell-free `program` plus `args` specifications and run visibly in a pane.
- A `pass` result requires required checks to pass and a fresh verifier report without release-blocking findings.
- Terminal screen text is not parsed to determine completion.

Run artifacts are stored under:

```text
${OSANWE_STATE_HOME:-${XDG_STATE_HOME:-$HOME/.local/state}/osanwe}/runs/<run-id>/
```

The directory contains `manifest.json`, `events.ndjson`, plan and verification reports, provider overlays, assignment results, check results, and integrated patches. Run directories and Unix sockets are user-only.

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

Most integration tests use fake command runners and temporary Git repositories, so subscriptions are not consumed in CI. A manual live smoke test requires installed and authenticated Codex, Grok Build, and Zellij clients.

## Current MVP limitations

- One Grok worker is scheduled at a time.
- IPC currently uses Unix domain sockets, so native Windows is not yet supported; WSL is supported.
- Pane-screen subscription is not yet used for semantic completion and remains an optional recovery signal.
- Provider CLI compatibility is checked by `doctor`; live end-to-end behavior depends on the installed client versions and user configuration.

## License

MIT
