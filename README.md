# Osanwe

Osanwe launches **native interactive** Codex CLI and Grok Build sessions in a Zellij workspace and coordinates them through a **project-local `.osanwe/` file bus**.

It does not call model APIs directly and does not use `codex exec`, Grok `-p`/`--single`, Codex app-server, or Grok ACP as the primary path.

## Status

**Approach A (v0.2 direction):** bare `osanwe` onboarding → choose **client + model** per role → scaffold `.osanwe/` → Zellij multi-pane session launch.

- Targets: Linux, macOS, and WSL (`x86_64` and `aarch64`)
- Requires Zellij **0.44+** for pane-ID automation
- Native Windows is not supported (Unix domain sockets / Zellij)

## Prerequisites

- Linux, macOS, or WSL
- Git (recommended; used to locate the project root)
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

After installation:

```bash
export PATH="$HOME/.local/bin:$PATH"
osanwe doctor
```

### From source

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
   - **Phase 2 — Model:** for each enabled role, pick a model from that client's catalog (↑/↓), e.g. Codex `gpt-5.6-sol` / Grok `grok-4.5`.
2. Osanwe writes **`.osanwe/config.toml`** and scaffolds the file-bus tree.
3. A **Zellij** session starts with one native interactive pane per role (plus a board pane).
4. Type todos/goals in the **orchestrator** pane; all roles share `.osanwe/`.

Re-run bare `osanwe` later to resume/relaunch using the saved config (no re-prompt unless you re-onboard).

### Non-interactive defaults (CI / scripts)

```bash
osanwe onboard --defaults --repo . --force
# then, when Zellij is available:
osanwe
# or
osanwe onboard --defaults --launch --no-attach
```

## CLI

| Command | Description |
| --- | --- |
| `osanwe` | Default: onboard if needed, else launch the project Zellij session |
| `osanwe onboard` | Re-run onboarding TUI (use `--force` if config exists) |
| `osanwe onboard --defaults` | Write default client/model config without TUI |
| `osanwe attach` | Attach to the project's Zellij session |
| `osanwe stop` | Kill the project's Zellij session |
| `osanwe doctor` | Check `git`, `zellij`, `codex`, `grok` |

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
  sessions/            # session metadata
  prompts/             # role bootstrap prompts
  board/               # human-readable status
  logs/
```

Role panes receive bootstrap text pointing at this tree. Coordination is **file-based**: write todos, plans, and assignments under `.osanwe/` rather than depending on a home-directory daemon state machine.

## Role launch

Each role runs a **native interactive** client according to config, for example:

```bash
# Codex role
codex -C <project> -m <model> -s <sandbox>

# Grok role
grok --cwd <project> -m <model> --session-id <uuid>
```

Environment variables set on every pane:

- `OSANWE_PROJECT`, `OSANWE_DIR` (absolute `.osanwe` path)
- `OSANWE_ROLE`, `OSANWE_CLIENT`, `OSANWE_MODEL`

## Development

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

Most tests cover scaffolding, config persistence, and launch-spec construction on temporary projects (no live Zellij or provider subscriptions required).

## Current limitations

- Live multi-pane smoke needs Zellij + authenticated `codex`/`grok`.
- One worker role pane by default (no multi-worker fleet).
- Native Windows is not supported.
- Legacy `osanwe start "<task>"` daemon path may still exist as a hidden command; it is not the primary product entry.

## License

MIT
