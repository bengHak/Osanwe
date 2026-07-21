# Quit and Purge Session Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Ctrl+Q` terminate every Osanwe pane and remove the Zellij session's resurrectable metadata while preserving sessions that were only detached.

**Architecture:** Keep Zellij's native `Ctrl+Q` action. After interactive attach returns, query the named session through the existing `CommandRunner`; preserve it when still active, otherwise delete its saved session record and report unexpected cleanup failures.

**Tech Stack:** Rust 2021, Tokio, anyhow, Zellij 0.44+, existing `CommandRunner` test seam.

## Global Constraints

- Do not add a dependency or custom Zellij configuration.
- `Ctrl+O, D` detach must preserve the active session.
- `Ctrl+Q` must remove exited-session metadata.
- Use the existing `CommandRunner` abstraction for non-interactive Zellij commands.

---

### Task 1: Purge exited sessions after attach

**Files:**
- Modify: `src/session_launch.rs`
- Test: unit tests in `src/session_launch.rs`

**Interfaces:**
- Consumes: `CommandRunner::run(&CommandSpec)` and `TokioCommandRunner` from `src/process.rs`.
- Produces: private `cleanup_session_after_attach(session: &str, runner: &dyn CommandRunner) -> anyhow::Result<()>`.

- [ ] **Step 1: Write failing tests**

Add a scripted `CommandRunner` in `session_launch.rs` tests. One test returns success for `list-panes` and asserts no `delete-session` command was sent. A second returns failure for `list-panes`, success for `delete-session`, and asserts the exact session name was deleted. A third verifies an already-absent session is clean.

```rust
use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use crate::process::CommandOutput;

struct CleanupRunner {
    outputs: Mutex<VecDeque<CommandOutput>>,
    commands: Mutex<Vec<CommandSpec>>,
}

impl CleanupRunner {
    fn new(outputs: Vec<CommandOutput>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into()),
            commands: Mutex::new(Vec::new()),
        }
    }

    fn commands(&self) -> Vec<CommandSpec> {
        self.commands.lock().unwrap().clone()
    }
}

#[async_trait]
impl CommandRunner for CleanupRunner {
    async fn run(&self, spec: &CommandSpec) -> anyhow::Result<CommandOutput> {
        self.commands.lock().unwrap().push(spec.clone());
        Ok(self.outputs.lock().unwrap().pop_front().unwrap())
    }
}

#[tokio::test]
async fn cleanup_preserves_an_active_detached_session() {
    let runner = CleanupRunner::new(vec![CommandOutput::success("[]")]);
    cleanup_session_after_attach("osanwe-test", &runner).await.unwrap();
    assert_eq!(runner.commands().len(), 1);
}

#[tokio::test]
async fn cleanup_deletes_an_exited_session() {
    let runner = CleanupRunner::new(vec![
        CommandOutput { status: 1, stdout: String::new(), stderr: "inactive".into() },
        CommandOutput::success("deleted"),
    ]);
    cleanup_session_after_attach("osanwe-test", &runner).await.unwrap();
    assert!(runner.commands().iter().any(|command| {
        command.args == ["delete-session", "osanwe-test"]
    }));
}

#[tokio::test]
async fn cleanup_accepts_an_already_absent_session() {
    let runner = CleanupRunner::new(vec![
        CommandOutput { status: 1, stdout: String::new(), stderr: "inactive".into() },
        CommandOutput { status: 2, stdout: String::new(), stderr: "Session not found".into() },
    ]);
    cleanup_session_after_attach("osanwe-test", &runner).await.unwrap();
}
```

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test cleanup_ --lib`

Expected: compilation failure because `cleanup_session_after_attach` and `CleanupRunner` do not exist yet.

- [ ] **Step 3: Implement the minimum cleanup flow**

Add `CommandRunner` to the existing process imports and implement:

```rust
async fn cleanup_session_after_attach(
    session: &str,
    runner: &dyn CommandRunner,
) -> anyhow::Result<()> {
    let active = runner
        .run(&CommandSpec::new("zellij").args([
            "--session",
            session,
            "action",
            "list-panes",
            "--json",
        ]))
        .await?;
    if active.status == 0 {
        return Ok(());
    }
    let deleted = runner
        .run(&CommandSpec::new("zellij").args(["delete-session", session]))
        .await?;
    if deleted.status == 0 || deleted.stderr.to_ascii_lowercase().contains("not found") {
        return Ok(());
    }
    deleted.require_success("delete exited Zellij session")?;
    Ok(())
}
```

After `zellij attach` exits successfully, call the helper with `TokioCommandRunner`. Print `Quit all and delete session: Ctrl+Q` with the existing launch summary.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run: `cargo test cleanup_ --lib`

Expected: all three cleanup tests pass.

- [ ] **Step 5: Commit the behavior**

```bash
git add src/session_launch.rs
git commit -m "feat: purge sessions on Ctrl-Q"
```

### Task 2: Document and verify the shortcut

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: the `Ctrl+Q` behavior from Task 1.
- Produces: user-facing shortcut documentation.

- [ ] **Step 1: Document the behavior**

Add this sentence after the launch walkthrough:

```markdown
Press **Ctrl+Q** to stop every pane and permanently delete the Zellij session; use **Ctrl+O, D** to detach without stopping it.
```

- [ ] **Step 2: Run the full CI-equivalent verification**

Run:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
sh -n install.sh tests/install.sh
sh tests/install.sh
```

Expected: all commands exit 0, with all Rust tests and installer tests passing.

- [ ] **Step 3: Verify the real terminal lifecycle**

Run `target/debug/osanwe`, press `Ctrl+Q`, then run `zellij list-sessions`. Expected: the configured Osanwe session name is absent rather than marked `EXITED`.

- [ ] **Step 4: Commit the documentation**

```bash
git add README.md
git commit -m "docs: document quit-all shortcut"
```
