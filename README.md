# Osanwe

Osanwe is a Rust terminal orchestrator for **native interactive** Codex CLI and Grok Build sessions. It creates a Zellij workspace whose panes are real provider TUIs, while a local daemon coordinates planning, implementation, deterministic checks, and independent verification.

Osanwe does not call model APIs directly and does not use `codex exec`, Grok `-p`/`--single`, Codex app-server, or Grok ACP.

## Status

**v0.1.0** is released. The orchestration state machine, Zellij pane control, native provider launchers, additive Codex/Grok overlays, role-scoped MCP bridge, hook forwarding, Git worktree integration, visible checks pane, Ratatui orchestrator, and checksum-verified binary installer are implemented.

- Targets: Linux, macOS, and WSL (`x86_64` and `aarch64`)
- Requires Zellij **0.44+** for pane-ID automation
- Native Windows is not supported (Unix domain sockets)

## Prerequisites

- Linux, macOS, or WSL
- Git
- Zellij 0.44 or newer
- Codex CLI, already authenticated
- Grok Build CLI, already authenticated
- `curl` and `tar` for binary installation

Osanwe reuses each client's existing authentication, configuration, MCP servers, skills, plugins, hooks, and session storage.

## Install

### Binary (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/Osanwe/main/install.sh | sh
```

The installer downloads the release archive for the current OS and CPU, verifies it against the published `SHA256SUMS`, and installs `osanwe` to `~/.local/bin`.

Release assets:

| Target | Archive |
| --- | --- |
| Linux x86_64 | `osanwe-x86_64-unknown-linux-gnu.tar.gz` |
| Linux aarch64 | `osanwe-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x86_64 | `osanwe-x86_64-apple-darwin.tar.gz` |
| macOS aarch64 | `osanwe-aarch64-apple-darwin.tar.gz` |

Options:

```bash
# Install a specific release.
OSANWE_VERSION=v0.1.0 sh install.sh

# Install somewhere already on PATH.
OSANWE_INSTALL_DIR=/usr/local/bin sh install.sh

# Print the detected release target without installing.
sh install.sh --print-target
```

Environment variables: `OSANWE_VERSION`, `OSANWE_INSTALL_DIR`, `OSANWE_GITHUB_TOKEN` / `GH_TOKEN` (private forks), `OSANWE_REPOSITORY`, `OSANWE_OS`, `OSANWE_ARCH`, `OSANWE_DOWNLOAD_BASE`.

After installation:

```bash
export PATH="$HOME/.local/bin:$PATH"
osanwe doctor
```

`osanwe doctor` checks that `git`, `zellij`, `codex`, and `grok` are executable.

### From source

Rust **stable** is pinned via `rust-toolchain.toml` (clippy and rustfmt included):

```bash
cargo install --path .
```

## CLI

| Command | Description |
| --- | --- |
| `osanwe start "<task>"` | Create a run, worktrees, daemon, Zellij session, orchestrator, and planner pane |
| `osanwe start "<task>" --repo <path>` | Start against a repository other than `.` |
| `osanwe start "<task>" --no-attach` | Create the session without attaching this terminal |
| `osanwe list` | List persisted runs |
| `osanwe attach <run-id>` | Attach and restore the orchestrator pane if needed |
| `osanwe stop <run-id>` | Mark the run cancelled (panes stay open for inspection) |
| `osanwe resume <run-id> <agent-id>` | Resume an exited interactive provider pane |
| `osanwe doctor` | Check required executables and print versions |

## Start a run

Run Osanwe from a clean Git repository (no tracked modifications):

```bash
osanwe start "Implement refresh-token rotation and add regression tests"
```

Background session without attaching:

```bash
osanwe start "Implement refresh-token rotation" --no-attach
osanwe list
osanwe attach <run-id>
```

A new run creates:

1. an isolated integration worktree;
2. a background daemon and Zellij session;
3. an Osanwe orchestrator pane;
4. a native interactive Codex planner pane.

When the planner submits a structured plan through the Osanwe MCP bridge, press `a` in the orchestrator pane to approve it. Osanwe then creates a native interactive Grok worker pane. After the worker submits completion, Osanwe checkpoints and integrates the worker branch, opens a visible checks pane, and finally starts a fresh native interactive Codex verifier pane. Failed verification can produce a repair assignment for the existing worker.

## Orchestrator controls

| Key | Action |
| --- | --- |
| `a` | Approve a submitted plan (only in plan review) |
| `Enter` | Focus the selected agent pane and hand control to the user |
| `m` | Toggle the selected pane between user and orchestrator control |
| `j` / `k` or arrows | Select next / previous agent |
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

| Role | Tools |
| --- | --- |
| Planner | `get_assignment`, `report_progress`, `request_input`, `submit_plan` |
| Worker | `get_assignment`, `report_progress`, `request_input`, `report_deviation`, `complete_assignment` |
| Verifier | `get_assignment`, `report_progress`, `request_input`, `submit_verification` |

Lifecycle hooks provide state hints such as session start, tool activity, approval requests, and stop events. Hook failures are not treated as a security boundary, and the hook forwarder intentionally emits no approval decision so the native provider approval UI remains visible.

## Workspace and verification model

- The original working tree must have no tracked modifications.
- Integration and worker work happen in separate Git worktrees.
- Osanwe creates relay-owned checkpoint commits on worker branches.
- Worker commits are cherry-picked into the integration worktree.
- Checks are shell-free `program` plus `args` specifications and run visibly in a pane.
- When a plan omits checks, defaults apply (`cargo fmt` / `cargo test` for Rust repos, `npm test` for Node).
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
sh tests/install.sh   # installer regression suite
```

CI runs the Rust checks above plus POSIX shell validation of `install.sh`. Release builds publish four platform archives and `SHA256SUMS` for `v<package-version>` from `main`.

Most integration tests use fake command runners and temporary Git repositories, so provider subscriptions are not consumed in CI. A manual live smoke test requires installed and authenticated Codex, Grok Build, and Zellij clients.

## Current limitations

- One Grok worker is scheduled at a time.
- IPC uses Unix domain sockets only (WSL yes, native Windows no).
- Pane-screen subscription is not used for semantic completion; it remains an optional recovery signal.
- Provider CLI compatibility is checked by `doctor`; live end-to-end behavior depends on installed client versions and user configuration.

## License

MIT
