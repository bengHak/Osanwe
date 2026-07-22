# Osanwe

**Version:** 0.1.1

Osanwe launches **native interactive** Codex CLI and Grok Build sessions in a Zellij workspace and coordinates them through a **project-local `.osanwe/` file bus**.

It does not call model APIs directly and does not use `codex exec`, Grok `-p`/`--single`, Codex app-server, or Grok ACP as the primary path.

## Status

Current source version is **v0.1.1**. Product direction (**Approach A**): bare `osanwe` onboarding → choose **client + model** per role → scaffold `.osanwe/` → Zellij multi-pane session launch.

- Targets: Linux, macOS, and WSL (`x86_64` and `aarch64`)
- Requires Zellij **0.44+** for pane-ID automation
- Native Windows is not supported (Unix domain sockets / Zellij)

## Prerequisites

- Linux, macOS, or WSL
- Git (recommended for locating the project root via `.git`; without it, Osanwe falls back to the nearest existing `.osanwe/config.toml` or the current directory)
- Zellij 0.44 or newer
- Codex CLI and/or Grok Build CLI, already authenticated
- `curl` and `tar` for binary installation

Osanwe reuses each client's existing authentication, configuration, MCP servers, skills, plugins, hooks, and session storage.

## Install

### Binary (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/bengHak/Osanwe/main/install.sh | sh
```

The installer downloads the release archive for the current OS and CPU, verifies it against the published `SHA256SUMS`, and installs `osanwe` to `~/.local/bin`.

Optional environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `OSANWE_VERSION` | `latest` | Release tag, e.g. `v0.1.1` |
| `OSANWE_INSTALL_DIR` | `~/.local/bin` | Install destination |
| `OSANWE_GITHUB_TOKEN` / `GH_TOKEN` | (empty) | Auth for private clones or higher API limits |

After installation:

```bash
export PATH="$HOME/.local/bin:$PATH"
osanwe doctor
```

### From source

Requires a recent stable Rust toolchain (see `rust-toolchain.toml`).

```bash
cargo install --path .
```

## Quick start

From a project directory:

```bash
osanwe
```

1. **First run** opens a two-phase onboarding TUI:
   - **Phase 1 — Client:** for each role (orchestrator → planner → worker → optional verifier), pick `codex` or `grok`.
   - **Phase 2 — Model:** for each enabled role, pick a model from that client's catalog (↑/↓), e.g. Codex `gpt-5.6-sol` / Grok `grok-4.5`. Catalog entries are hard-coded in Osanwe and may lag behind each CLI.
2. Osanwe writes **`.osanwe/config.toml`** and scaffolds the file-bus tree.
3. A **Zellij** session starts with one native interactive pane per role, plus a **board** pane that summarizes `.osanwe/board/` and related status.
4. Type todos/goals in the **orchestrator** pane; all roles share `.osanwe/`.

Press **Ctrl+Q** to stop every pane and permanently delete the Zellij session; use **Ctrl+O, D** to detach without stopping it.

### Resume, re-onboard, and stop

```bash
osanwe              # relaunch / attach using saved .osanwe/config.toml
osanwe attach       # attach only (session must already exist)
osanwe stop         # kill the project's Zellij session
osanwe onboard --force   # re-run onboarding TUI and overwrite config
```

### Non-interactive defaults (CI / scripts)

```bash
osanwe onboard --defaults --repo . --force
# then, when Zellij is available:
osanwe
# or
osanwe onboard --defaults --launch --no-attach
```

## CLI

Public commands:

| Command | Description |
| --- | --- |
| `osanwe` | Default: onboard if needed, else launch the project Zellij session |
| `osanwe onboard` | Re-run onboarding TUI (use `--force` if config exists) |
| `osanwe onboard --defaults` | Write default client/model config without TUI (`--force` if config exists) |
| `osanwe attach` | Attach to the project's Zellij session |
| `osanwe stop` | Kill the project's Zellij session |
| `osanwe doctor` | Require Zellij and at least one provider; report Git and the other provider when absent |

## `.osanwe/` file bus

Created under the project root:

```text
.osanwe/
  config.toml          # role → client + model
  README.md            # file-bus guide
  todos/               # goals / todo items
  plans/               # planner output
  assignments/         # work items
  workers/             # worker status
  sessions/            # per-role Grok session-id markers, etc.
  prompts/             # role bootstrap prompts
  board/               # human-readable status (board pane)
  logs/
```

Example `.osanwe/config.toml` after onboarding:

```toml
schema_version = "1"
zellij_session = "osanwe-6cc77ca0"

[roles.orchestrator]
client = "codex"
model = "gpt-5.6-sol"

[roles.planner]
client = "codex"
model = "gpt-5.6-sol"

[roles.worker]
client = "grok"
model = "grok-4.5"

[roles.verifier]
client = "codex"
model = "gpt-5.6-sol"
```

Role panes receive bootstrap text pointing at this tree. Coordination is **file-based**: write todos, plans, and assignments under `.osanwe/`.

## Role launch

Each role runs a **native interactive** client according to config.

### Codex

```bash
codex -C <project> -m <model> -s <sandbox>
```

Default sandbox by role:

| Role | Sandbox |
| --- | --- |
| orchestrator, worker | `workspace-write` |
| planner, verifier | `read-only` |

### Grok

```bash
# First launch for a role
grok --cwd <project> -m <model> --session-id <uuid>

# Later launches when that session still exists under ~/.grok/sessions
grok --cwd <project> -m <model> --resume <uuid>
```

Per-role UUIDs are stored in `.osanwe/sessions/<role>.session-id`. If the marker is stale or the Grok session is gone, Osanwe mints a new id and uses `--session-id` again.

### Environment

Variables set on every pane:

- `OSANWE_PROJECT`, `OSANWE_DIR` (absolute `.osanwe` path)
- `OSANWE_ROLE`, `OSANWE_CLIENT`, `OSANWE_MODEL`

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
# optional: requires Zellij 0.44+ on PATH
cargo test --test zellij_live -- --ignored
```

Most tests cover scaffolding, config persistence, and launch-spec construction on temporary projects (no live Zellij or provider subscriptions required).

## Current limitations

- Authenticated provider smoke still needs Zellij plus signed-in `codex`/`grok` clients.
- One worker role pane by default (no multi-worker fleet).
- Native Windows is not supported.

## Security

See [SECURITY.md](SECURITY.md) for reporting and trust boundaries (launched CLIs, project permissions, local state).

## License

MIT
