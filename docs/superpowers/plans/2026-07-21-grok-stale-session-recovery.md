# Grok Stale-Session Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Start Grok with a new UUID when Osanwe's saved UUID has no matching local Grok session, while preserving resume for valid sessions.

**Architecture:** Keep session selection in `session_launch.rs`. Resolve `~/.grok/sessions`, scan its per-project directories for the saved UUID, and only emit `--resume` when a matching directory exists; otherwise overwrite the marker and emit `--session-id`.

**Tech Stack:** Rust standard library, existing `uuid` and `tempfile` dependencies.

## Global Constraints

- Do not delete existing Grok session data.
- Add no dependency.
- Treat an unavailable or unreadable Grok session store as a stale marker and start a new session.

---

### Task 1: Validate Grok session markers before resume

**Files:**
- Modify: `src/session_launch.rs:65-140`
- Test: `src/session_launch.rs:615-660`

**Interfaces:**
- Consumes: `.osanwe/sessions/<role>.session-id` and `~/.grok/sessions/<encoded-project>/<uuid>`.
- Produces: `interactive_command_with_grok_sessions(..., Option<&Path>) -> anyhow::Result<CommandSpec>` for deterministic unit tests; the public `interactive_command(...)` keeps its current signature.

- [ ] **Step 1: Write the failing stale-marker test**

Add a test-only call path that accepts a temporary Grok sessions root, then add this regression test:

```rust
#[test]
fn grok_stale_session_marker_starts_a_new_session() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let osanwe = osanwe_dir(root);
    let grok_sessions = root.join("grok-sessions");
    fs::create_dir_all(osanwe.join("sessions")).unwrap();
    fs::create_dir_all(&grok_sessions).unwrap();
    let stale = "76ea9f12-2bcb-4e4a-b5b2-27cfccac95dd";
    fs::write(osanwe.join("sessions/worker.session-id"), stale).unwrap();
    let choice = RoleChoice::new(ClientKind::Grok, "grok-4.5");

    let command = interactive_command_with_grok_sessions(
        root,
        &osanwe,
        "worker",
        &choice,
        Some(&grok_sessions),
    )
    .unwrap();

    assert!(!command.args.iter().any(|arg| arg == "--resume"));
    let replacement = command
        .args
        .windows(2)
        .find(|args| args[0] == "--session-id")
        .map(|args| args[1].as_str())
        .expect("new session id");
    assert_ne!(replacement, stale);
    assert_eq!(
        fs::read_to_string(osanwe.join("sessions/worker.session-id")).unwrap(),
        replacement
    );
}
```

- [ ] **Step 2: Run the regression test and verify RED**

Run: `cargo test grok_stale_session_marker_starts_a_new_session -- --exact`

Expected: compilation fails because `interactive_command_with_grok_sessions` does not exist yet.

- [ ] **Step 3: Implement the minimum validation**

Change the path import to `use std::path::{Path, PathBuf};`. Keep `interactive_command` as the public entry point and route it through a deterministic helper:

```rust
pub fn interactive_command(
    project_root: &Path,
    osanwe: &Path,
    role: &str,
    choice: &RoleChoice,
) -> anyhow::Result<CommandSpec> {
    let grok_sessions = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".grok/sessions"));
    interactive_command_with_grok_sessions(
        project_root,
        osanwe,
        role,
        choice,
        grok_sessions.as_deref(),
    )
}
```

Rename the existing command-construction body to this private helper and pass `grok_sessions` to the binding lookup:

```rust
fn interactive_command_with_grok_sessions(
    project_root: &Path,
    osanwe: &Path,
    role: &str,
    choice: &RoleChoice,
    grok_sessions: Option<&Path>,
) -> anyhow::Result<CommandSpec> {
    let project = path_text(project_root)?;
    let osanwe_text = path_text(osanwe)?;
    let mut command = match choice.client {
        ClientKind::Codex => {
            let mut c = CommandSpec::new("codex").args(["-C", &project]);
            if !choice.model.trim().is_empty() {
                c = c.args(["-m", choice.model.trim()]);
            }
            let sandbox = match role {
                "planner" | "verifier" => "read-only",
                _ => "workspace-write",
            };
            c.args(["-s", sandbox])
        }
        ClientKind::Grok => {
            let mut c = CommandSpec::new("grok").args(["--cwd", &project]);
            if !choice.model.trim().is_empty() {
                c = c.args(["-m", choice.model.trim()]);
            }
            match role_session_binding(osanwe, role, grok_sessions)? {
                GrokSessionBinding::New(session_id) => {
                    c = c.args(["--session-id", &session_id]);
                }
                GrokSessionBinding::Resume(session_id) => {
                    c = c.args(["--resume", &session_id]);
                }
            }
            c
        }
    };

    command = command
        .cwd(project_root)
        .env("OSANWE_PROJECT", project)
        .env("OSANWE_DIR", osanwe_text)
        .env("OSANWE_ROLE", role)
        .env("OSANWE_CLIENT", choice.client.as_str())
        .env("OSANWE_MODEL", choice.model.clone());

    Ok(command)
}
```

Change the binding function so a marker is resumable only when its local Grok session exists:

```rust
fn role_session_binding(
    osanwe: &Path,
    role: &str,
    grok_sessions: Option<&Path>,
) -> anyhow::Result<GrokSessionBinding> {
    let marker = osanwe.join("sessions").join(format!("{role}.session-id"));
    if let Ok(existing) = fs::read_to_string(&marker) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() && grok_session_exists(grok_sessions, trimmed) {
            return Ok(GrokSessionBinding::Resume(trimmed.to_owned()));
        }
    }
    let id = Uuid::new_v4().to_string();
    fs::create_dir_all(osanwe.join("sessions"))
        .with_context(|| format!("create {}", osanwe.join("sessions").display()))?;
    fs::write(&marker, &id).with_context(|| format!("write {}", marker.display()))?;
    Ok(GrokSessionBinding::New(id))
}
```

Validate saved IDs with:

```rust
fn grok_session_exists(sessions: Option<&Path>, session_id: &str) -> bool {
    sessions
        .and_then(|root| fs::read_dir(root).ok())
        .is_some_and(|projects| {
            projects
                .filter_map(Result::ok)
                .any(|project| project.path().join(session_id).is_dir())
        })
}
```

- [ ] **Step 4: Preserve the valid-resume test**

Update `grok_first_launch_uses_session_id_relaunch_uses_resume` to call `interactive_command_with_grok_sessions`. After extracting the first UUID, create `grok_sessions/project/<uuid>` before the second call:

```rust
fs::create_dir_all(grok_sessions.join("project").join(&session_id)).unwrap();
```

- [ ] **Step 5: Run focused and full verification**

Run: `cargo test grok_`

Expected: both focused tests pass.

Run: `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features`

Expected: formatting, clippy, and all tests pass with zero failures.

- [ ] **Step 6: Build and install the fixed binary**

Run: `cargo build --release && cp target/release/osanwe "$HOME/.local/bin/osanwe"`

Expected: release build succeeds and the installed binary hash matches `target/release/osanwe`.

- [ ] **Step 7: Verify the real stale marker is replaced**

Run: `target/release/osanwe onboard --defaults --repo . --force --launch --no-attach`

Inspect the worker pane with `zellij --session osanwe-6cc77ca0 action list-panes --json` and `dump-screen`, and confirm the old UUID is replaced and Grok remains running without the 404. Stop the temporary session with `target/release/osanwe stop --repo .` after inspection.

- [ ] **Step 8: Commit**

```bash
git add src/session_launch.rs
git commit -m "fix: recover stale Grok sessions"
```
