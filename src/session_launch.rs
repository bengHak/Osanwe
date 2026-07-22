//! Build native interactive launch specs and start Zellij multi-pane sessions.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context};
use tokio::process::Command;
use uuid::Uuid;

use crate::process::{CommandRunner, CommandSpec, TokioCommandRunner};
use crate::project::{load_config, osanwe_dir, scaffold, ClientKind, ProjectConfig, RoleChoice};
use crate::zellij::{PaneSpec, ZellijPaneHost};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleLaunchSpec {
    pub role: String,
    pub client: ClientKind,
    pub model: String,
    pub command: CommandSpec,
    pub title: String,
}

/// Build interactive launch specs for every active role from project config.
pub fn build_role_launch_specs(
    project_root: &Path,
    config: &ProjectConfig,
) -> anyhow::Result<Vec<RoleLaunchSpec>> {
    let osanwe = osanwe_dir(project_root);
    let mut specs = Vec::new();
    for (role, choice) in config.active_roles() {
        specs.push(build_one_spec(project_root, &osanwe, role, choice)?);
    }
    Ok(specs)
}

fn build_one_spec(
    project_root: &Path,
    osanwe: &Path,
    role: &str,
    choice: &RoleChoice,
) -> anyhow::Result<RoleLaunchSpec> {
    let bootstrap = bootstrap_for_role(osanwe, role)?;
    let command = interactive_command(project_root, osanwe, role, choice)?.arg(&bootstrap);
    let title = format!(
        "[{}] {} ({})",
        role_label(role),
        choice.client,
        if choice.model.is_empty() {
            "default"
        } else {
            choice.model.as_str()
        }
    );
    Ok(RoleLaunchSpec {
        role: role.to_owned(),
        client: choice.client,
        model: choice.model.clone(),
        command,
        title,
    })
}

/// Native interactive command for a role, cwd at project root, env pointing at `.osanwe`.
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
            // Orchestrator and worker need write access; planner/verifier stay read-only by default.
            let sandbox = match role {
                "planner" | "verifier" => "read-only",
                _ => "workspace-write",
            };
            c = c.args(["-s", sandbox]);
            c
        }
        ClientKind::Grok => {
            let mut c = CommandSpec::new("grok").args(["--cwd", &project]);
            if !choice.model.trim().is_empty() {
                c = c.args(["-m", choice.model.trim()]);
            }
            // First launch: --session-id (new conversation only).
            // Relaunch: --resume <same id> so Grok does not reject an existing session.
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum GrokSessionBinding {
    New(String),
    Resume(String),
}

/// Persist a per-role Grok session UUID. Valid markers resume; stale or missing ones mint a new id.
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

fn grok_session_exists(sessions: Option<&Path>, session_id: &str) -> bool {
    // ponytail: scan project dirs; add an index only if session counts make this measurable.
    sessions
        .and_then(|root| fs::read_dir(root).ok())
        .is_some_and(|projects| {
            projects
                .filter_map(Result::ok)
                .any(|project| project.path().join(session_id).is_dir())
        })
}

fn bootstrap_for_role(osanwe: &Path, role: &str) -> anyhow::Result<String> {
    let path = osanwe.join("prompts").join(format!("{role}.md"));
    if path.is_file() {
        let body = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        return Ok(format!(
            "Osanwe role: {role}. Shared file bus is at `.osanwe/` (absolute: {}).\n\n{body}",
            osanwe.display()
        ));
    }
    Ok(format!(
        "You are the Osanwe {role}. Coordinate using the project `.osanwe/` file bus at {}.",
        osanwe.display()
    ))
}

fn role_label(role: &str) -> &'static str {
    match role {
        "orchestrator" => "O",
        "planner" => "P",
        "worker" => "W",
        "verifier" => "V",
        _ => "?",
    }
}

/// Ensure scaffold, start Zellij panes, and optionally attach.
pub async fn launch_project_session(project_root: &Path, no_attach: bool) -> anyhow::Result<()> {
    require_unix()?;
    scaffold(project_root)?;
    let config = load_config(project_root)?;
    let specs = build_role_launch_specs(project_root, &config)?;
    if specs.is_empty() {
        bail!("no roles configured to launch");
    }

    write_board_status(project_root, &config, &specs)?;

    let runner = Arc::new(TokioCommandRunner);
    let host = ZellijPaneHost::new(config.zellij_session.clone(), runner);
    host.create_session().await?;

    let initial_panes = host.list_panes().await.unwrap_or_default();
    let executable = path_text(&std::env::current_exe()?)?;

    let mut created = Vec::new();
    for spec in &specs {
        let pane_id = host
            .create_pane(PaneSpec {
                title: spec.title.clone(),
                cwd: project_root.to_path_buf(),
                command: spec.command.clone(),
            })
            .await?;
        created.push(pane_id);
    }

    // Board pane: lightweight Osanwe status viewer.
    let board_pane = host
        .create_pane(PaneSpec {
            title: "[Board] Osanwe".into(),
            cwd: project_root.to_path_buf(),
            command: CommandSpec::new(executable).args([
                "board".into(),
                "--repo".into(),
                path_text(project_root)?,
            ]),
        })
        .await?;
    created.push(board_pane);

    for pane in initial_panes.into_iter().filter(|p| !p.is_plugin) {
        let id = pane.pane_id();
        if !created.iter().any(|c| c.as_str() == id.as_str()) {
            host.close(id.as_str()).await.ok();
        }
    }

    println!("Osanwe session: {}", config.zellij_session);
    println!("Project: {}", project_root.display());
    println!("File bus: {}", osanwe_dir(project_root).display());
    for spec in &specs {
        println!(
            "  {} → {} {}",
            spec.role,
            spec.client,
            if spec.model.is_empty() {
                "(default model)"
            } else {
                spec.model.as_str()
            }
        );
    }
    println!("Quit all and delete session: Ctrl+Q");

    if no_attach {
        println!("Attach with: osanwe attach");
        Ok(())
    } else {
        attach_terminal(&config.zellij_session).await
    }
}

pub async fn attach_project_session(project_root: &Path) -> anyhow::Result<()> {
    require_unix()?;
    let config = load_config(project_root)?;
    attach_terminal(&config.zellij_session).await
}

pub async fn stop_project_session(project_root: &Path) -> anyhow::Result<()> {
    require_unix()?;
    let config = load_config(project_root)?;
    let output = Command::new("zellij")
        .args(["kill-session", &config.zellij_session])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => {
            println!("Stopped Zellij session {}", config.zellij_session);
            Ok(())
        }
        Ok(output)
            if missing_session_error(&String::from_utf8_lossy(&output.stdout))
                || missing_session_error(&String::from_utf8_lossy(&output.stderr)) =>
        {
            println!("Zellij session {} is already absent", config.zellij_session,);
            Ok(())
        }
        Ok(output) => bail!(
            "stop Zellij session {} failed with status {:?}: {}",
            config.zellij_session,
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => Err(error).context("run zellij kill-session"),
    }
}

fn missing_session_error(output: &str) -> bool {
    let message = output.to_ascii_lowercase();
    message.contains("session not found")
        || (message.contains("no session named") && message.contains("found"))
}

fn write_board_status(
    project_root: &Path,
    config: &ProjectConfig,
    specs: &[RoleLaunchSpec],
) -> anyhow::Result<()> {
    let mut body = String::from("# Osanwe board\n\n");
    body.push_str(&format!("Session: `{}`\n\n", config.zellij_session));
    body.push_str("| Role | Client | Model |\n| --- | --- | --- |\n");
    for spec in specs {
        body.push_str(&format!(
            "| {} | {} | {} |\n",
            spec.role,
            spec.client,
            if spec.model.is_empty() {
                "default"
            } else {
                &spec.model
            }
        ));
    }
    body.push_str("\nPut todos in `.osanwe/todos/`, plans in `.osanwe/plans/`.\n");
    let path = osanwe_dir(project_root).join("board/status.md");
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub async fn attach_terminal(session: &str) -> anyhow::Result<()> {
    let status = Command::new("zellij")
        .args(["attach", session])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("attach to Zellij session")?;
    if status.success() {
        cleanup_session_after_attach(session, &TokioCommandRunner).await
    } else {
        bail!("Zellij attach exited with status {:?}", status.code())
    }
}

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

fn require_unix() -> anyhow::Result<()> {
    if cfg!(unix) {
        Ok(())
    } else {
        bail!("Osanwe currently requires Linux, macOS, or WSL")
    }
}

fn path_text(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn board_snapshot(project_root: &Path) -> anyhow::Result<String> {
    let status_path = osanwe_dir(project_root).join("board/status.md");
    let todos = osanwe_dir(project_root).join("todos");
    let mut output = format!(
        "Osanwe board — project {}\nFile bus: {}\n---\n",
        project_root.display(),
        osanwe_dir(project_root).display()
    );
    if status_path.is_file() {
        output.push_str(&fs::read_to_string(&status_path)?);
    }
    output.push_str("---\nTodos:\n");
    if todos.is_dir() {
        let mut entries: Vec<_> = fs::read_dir(&todos)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()?;
        entries.retain(|path| path.is_file());
        entries.sort();
        if entries.is_empty() {
            output.push_str("  (none yet — add files under .osanwe/todos/)\n");
        } else {
            for entry in entries {
                output.push_str(&format!(
                    "  - {}\n",
                    entry.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
    }
    Ok(output)
}

/// Refresh board status until the pane is closed.
pub fn run_board(project_root: &Path) -> anyhow::Result<()> {
    loop {
        print!("\x1b[2J\x1b[H{}", board_snapshot(project_root)?);
        io::stdout().flush()?;
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::process::{CommandOutput, CommandRunner};
    use crate::project::{scaffold_with_config, ProjectConfig, RoleChoice};
    use async_trait::async_trait;
    use tempfile::tempdir;

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

    #[test]
    fn board_snapshot_reads_current_status_and_sorted_todos() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let osanwe = osanwe_dir(root);
        fs::create_dir_all(osanwe.join("board")).unwrap();
        fs::create_dir_all(osanwe.join("todos")).unwrap();
        fs::write(osanwe.join("board/status.md"), "status body\n").unwrap();
        fs::write(osanwe.join("todos/b.md"), "b").unwrap();
        fs::write(osanwe.join("todos/a.md"), "a").unwrap();

        let text = board_snapshot(root).unwrap();

        assert!(text.contains("status body"));
        assert!(text.find("a.md").unwrap() < text.find("b.md").unwrap());
    }

    #[test]
    fn stop_ignores_only_a_missing_session() {
        assert!(missing_session_error("Session not found"));
        assert!(missing_session_error(
            "No session named \"osanwe-missing\" found."
        ));
        assert!(!missing_session_error("configuration file not found"));
        assert!(!missing_session_error("permission denied"));
    }

    #[tokio::test]
    async fn cleanup_preserves_an_active_detached_session() {
        let runner = CleanupRunner::new(vec![CommandOutput::success("[]")]);

        cleanup_session_after_attach("osanwe-test", &runner)
            .await
            .unwrap();

        assert_eq!(runner.commands().len(), 1);
    }

    #[tokio::test]
    async fn cleanup_deletes_an_exited_session() {
        let runner = CleanupRunner::new(vec![
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "inactive".into(),
            },
            CommandOutput::success("deleted"),
        ]);

        cleanup_session_after_attach("osanwe-test", &runner)
            .await
            .unwrap();

        assert!(runner
            .commands()
            .iter()
            .any(|command| command.args == ["delete-session", "osanwe-test"]));
    }

    #[tokio::test]
    async fn cleanup_accepts_an_already_absent_session() {
        let runner = CleanupRunner::new(vec![
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "inactive".into(),
            },
            CommandOutput {
                status: 2,
                stdout: String::new(),
                stderr: "Session not found".into(),
            },
        ]);

        cleanup_session_after_attach("osanwe-test", &runner)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn cleanup_reports_an_unexpected_delete_failure() {
        let runner = CleanupRunner::new(vec![
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "inactive".into(),
            },
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "permission denied".into(),
            },
        ]);

        let error = cleanup_session_after_attach("osanwe-test", &runner)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("permission denied"));
    }

    #[test]
    fn launch_specs_reference_project_osanwe_and_models() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let mut config = ProjectConfig::defaults_for_repo(root);
        config.roles.orchestrator = RoleChoice::new(ClientKind::Grok, "grok-fast");
        config.roles.planner = RoleChoice::new(ClientKind::Codex, "o3");
        config.roles.worker = RoleChoice::new(ClientKind::Grok, "grok-4.5");
        config.roles.verifier = Some(RoleChoice::new(ClientKind::Codex, "o4-mini"));
        scaffold_with_config(root, &config).unwrap();

        let specs = build_role_launch_specs(root, &config).unwrap();
        assert_eq!(specs.len(), 4);

        let orch = specs.iter().find(|s| s.role == "orchestrator").unwrap();
        assert_eq!(orch.client, ClientKind::Grok);
        assert_eq!(orch.command.program, "grok");
        assert!(orch.command.args.iter().any(|a| a == "--cwd"));
        assert!(orch.command.args.iter().any(|a| a == "grok-fast"));
        assert_eq!(
            orch.command.env.get("OSANWE_DIR").map(String::as_str),
            Some(osanwe_dir(root).to_str().unwrap())
        );
        assert!(orch.command.args.iter().any(|arg| arg.contains(".osanwe")));

        let planner = specs.iter().find(|s| s.role == "planner").unwrap();
        assert_eq!(planner.command.program, "codex");
        assert!(planner.command.args.iter().any(|a| a == "-C"));
        assert!(planner.command.args.iter().any(|a| a == "o3"));
        assert!(planner.command.args.iter().any(|a| a == "read-only"));
        assert_eq!(
            planner.command.env.get("OSANWE_ROLE").map(String::as_str),
            Some("planner")
        );

        let worker = specs.iter().find(|s| s.role == "worker").unwrap();
        assert_eq!(worker.command.program, "grok");
        assert!(worker.command.args.iter().any(|a| a == "grok-4.5"));
        assert_eq!(worker.command.cwd.as_deref(), Some(root));
    }

    #[test]
    fn launch_specs_skip_verifier_when_disabled() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let mut config = ProjectConfig::defaults_for_repo(root);
        config.roles.verifier = None;
        scaffold_with_config(root, &config).unwrap();
        let specs = build_role_launch_specs(root, &config).unwrap();
        assert_eq!(specs.len(), 3);
        assert!(specs.iter().all(|s| s.role != "verifier"));
    }

    #[test]
    fn empty_model_omits_model_flag() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let choice = RoleChoice::new(ClientKind::Codex, "");
        let osanwe = osanwe_dir(root);
        fs::create_dir_all(&osanwe).unwrap();
        let cmd = interactive_command(root, &osanwe, "worker", &choice).unwrap();
        assert!(!cmd.args.iter().any(|a| a == "-m"));
        assert_eq!(cmd.env.get("OSANWE_MODEL").map(String::as_str), Some(""));
    }

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

    #[test]
    fn grok_first_launch_uses_session_id_relaunch_uses_resume() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let osanwe = osanwe_dir(root);
        let grok_sessions = root.join("grok-sessions");
        fs::create_dir_all(osanwe.join("sessions")).unwrap();
        fs::create_dir_all(&grok_sessions).unwrap();
        let choice = RoleChoice::new(ClientKind::Grok, "grok-4.5");

        let first = interactive_command_with_grok_sessions(
            root,
            &osanwe,
            "worker",
            &choice,
            Some(&grok_sessions),
        )
        .unwrap();
        assert!(
            first.args.windows(2).any(|w| w[0] == "--session-id"),
            "first launch should mint --session-id: {:?}",
            first.args
        );
        assert!(
            !first.args.iter().any(|a| a == "--resume"),
            "first launch must not --resume: {:?}",
            first.args
        );
        let session_id = first
            .args
            .windows(2)
            .find(|w| w[0] == "--session-id")
            .map(|w| w[1].clone())
            .expect("session id");
        assert!(
            osanwe.join("sessions/worker.session-id").is_file(),
            "marker should be persisted"
        );
        fs::create_dir_all(grok_sessions.join("project").join(&session_id)).unwrap();

        let second = interactive_command_with_grok_sessions(
            root,
            &osanwe,
            "worker",
            &choice,
            Some(&grok_sessions),
        )
        .unwrap();
        assert!(
            second
                .args
                .windows(2)
                .any(|w| w == ["--resume".to_owned(), session_id.clone()]
                    || (w[0] == "--resume" && w[1] == session_id)),
            "relaunch should --resume same id {session_id}: {:?}",
            second.args
        );
        assert!(
            !second.args.iter().any(|a| a == "--session-id"),
            "relaunch must not re-pass --session-id: {:?}",
            second.args
        );
    }
}
