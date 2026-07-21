use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use serde_json::json;
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::daemon;
use crate::hook;
use crate::ipc::IpcClient;
use crate::mcp;
use crate::model::{AgentRecord, AgentRole, AgentState, RunManifest, RunStatus};
use crate::pane;
use crate::process::{CommandSpec, TokioCommandRunner};
use crate::store::RunStore;
use crate::workspace::WorkspaceManager;
use crate::zellij::{PaneHost, PaneSpec, ZellijPaneHost};

#[derive(Debug, Parser)]
#[command(
    name = "osanwe",
    version,
    about = "Interactive Codex and Grok pane orchestrator"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Start a new interactive orchestration run.
    Start {
        /// Task given to the Codex planner.
        task: String,
        /// Git repository to orchestrate.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Create the Zellij session without attaching this terminal.
        #[arg(long)]
        no_attach: bool,
    },
    /// Attach to an existing run and restore its orchestrator pane if needed.
    Attach { run_id: String },
    /// List persisted runs.
    List,
    /// Check required executables and print discovered versions.
    Doctor,
    /// Mark a run cancelled. Panes remain available for inspection.
    Stop { run_id: String },
    /// Resume an exited interactive provider pane.
    Resume { run_id: String, agent_id: String },
    #[command(hide = true)]
    Daemon { run_id: String },
    #[command(hide = true)]
    Tui { run_id: String },
    #[command(hide = true)]
    Pane {
        run_id: String,
        agent_id: String,
        #[arg(long)]
        resume: bool,
    },
    #[command(hide = true)]
    Bridge,
    #[command(hide = true)]
    Hook,
    #[command(hide = true)]
    Checks { run_id: String },
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Start {
            task,
            repo,
            no_attach,
        } => start(task, repo, no_attach).await,
        Commands::Attach { run_id } => attach(&run_id).await,
        Commands::List => list_runs(),
        Commands::Doctor => doctor().await,
        Commands::Stop { run_id } => stop(&run_id).await,
        Commands::Resume { run_id, agent_id } => resume_agent(&run_id, &agent_id).await,
        Commands::Daemon { run_id } => {
            daemon::run_daemon(RunStore::from_environment()?, &run_id).await
        }
        Commands::Tui { run_id } => crate::tui::run(RunStore::from_environment()?, &run_id).await,
        Commands::Pane {
            run_id,
            agent_id,
            resume,
        } => pane::run_agent_pane(RunStore::from_environment()?, &run_id, &agent_id, resume).await,
        Commands::Bridge => mcp::run_stdio().await,
        Commands::Hook => hook::forward_stdin().await,
        Commands::Checks { run_id } => {
            pane::run_check_pane(RunStore::from_environment()?, &run_id).await
        }
    }
}

async fn start(task: String, repo: PathBuf, no_attach: bool) -> anyhow::Result<()> {
    require_unix()?;
    let store = RunStore::from_environment()?;
    let runner = Arc::new(TokioCommandRunner);
    let workspace = WorkspaceManager::new(runner.clone());
    let repository = workspace.preflight(&repo).await?;
    let run_id = Uuid::new_v4().simple().to_string();
    let run_dir = store.create_run_dir(&run_id)?;
    let integration = run_dir.join("worktrees/integration");
    workspace
        .create_worktree(
            &repository.root,
            &integration,
            &format!("osanwe/{}/integration", short_id(&run_id)),
            &repository.base_sha,
        )
        .await?;

    let mut manifest = RunManifest::new_with_id(
        run_id.clone(),
        task,
        repository.root,
        run_dir.clone(),
        repository.base_sha,
        integration.clone(),
    );
    manifest.status = RunStatus::Planning;
    let mut orchestrator =
        AgentRecord::new("orchestrator", AgentRole::Orchestrator, integration.clone());
    orchestrator.state = AgentState::PaneCreating;
    let mut planner = AgentRecord::new("planner", AgentRole::Planner, integration.clone());
    planner.state = AgentState::PaneCreating;
    manifest.agents.insert("orchestrator".into(), orchestrator);
    manifest.agents.insert("planner".into(), planner);
    store.save(&manifest)?;

    spawn_daemon(&store, &manifest).await?;
    wait_for_daemon(&store, &manifest).await?;
    let host = ZellijPaneHost::new(manifest.zellij_session.clone(), runner);
    host.create_session().await?;
    let initial_panes = host.list_panes().await.unwrap_or_default();
    let executable = path_text(&std::env::current_exe()?)?;

    let orchestrator_pane = host
        .create_pane(PaneSpec {
            title: "[O] Osanwe Orchestrator".into(),
            cwd: integration.clone(),
            command: CommandSpec::new(executable.clone()).args(vec![
                "tui".into(),
                "--run-id".into(),
                run_id.clone(),
            ]),
        })
        .await?;
    let planner_pane = host
        .create_pane(PaneSpec {
            title: "[P] Codex Planner".into(),
            cwd: integration,
            command: CommandSpec::new(executable).args(vec![
                "pane".into(),
                "--run-id".into(),
                run_id.clone(),
                "--agent-id".into(),
                "planner".into(),
            ]),
        })
        .await?;

    let client = IpcClient::new(store.socket_path(&run_id), manifest.admin_token.clone());
    register_pane(&client, "orchestrator", orchestrator_pane.as_str()).await?;
    register_pane(&client, "planner", planner_pane.as_str()).await?;
    for pane in initial_panes.into_iter().filter(|pane| !pane.is_plugin) {
        let pane_id = pane.pane_id();
        if pane_id != orchestrator_pane && pane_id != planner_pane {
            host.close(pane_id.as_str()).await.ok();
        }
    }

    println!("Osanwe run created: {run_id}");
    println!("Zellij session: {}", manifest.zellij_session);
    println!(
        "Integration worktree: {}",
        manifest.integration_worktree.display()
    );
    if no_attach {
        println!("Attach with: osanwe attach {run_id}");
        Ok(())
    } else {
        attach_terminal(&manifest.zellij_session).await
    }
}

async fn attach(run_id: &str) -> anyhow::Result<()> {
    require_unix()?;
    let store = RunStore::from_environment()?;
    let manifest = store.load(run_id)?;
    if wait_for_daemon(&store, &manifest).await.is_err() {
        spawn_daemon(&store, &manifest).await?;
        wait_for_daemon(&store, &manifest).await?;
    }
    ensure_orchestrator_pane(&store, &manifest).await?;
    attach_terminal(&manifest.zellij_session).await
}

fn list_runs() -> anyhow::Result<()> {
    let store = RunStore::from_environment()?;
    let runs = store.list()?;
    if runs.is_empty() {
        println!("No Osanwe runs found.");
        return Ok(());
    }
    println!("{:<14} {:<14} {:<24} TASK", "RUN", "STATUS", "SESSION");
    for run in runs {
        println!(
            "{:<14} {:<14} {:<24} {}",
            short_id(&run.run_id),
            format!("{:?}", run.status),
            run.zellij_session,
            run.task
        );
    }
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {
    let commands = [
        ("git", vec!["--version"]),
        ("zellij", vec!["--version"]),
        ("codex", vec!["--version"]),
        ("grok", vec!["version"]),
    ];
    let mut missing = false;
    for (program, args) in commands {
        match Command::new(program).args(args).output().await {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let version = if stdout.trim().is_empty() {
                    stderr.trim()
                } else {
                    stdout.trim()
                };
                println!("✓ {program:<8} {version}");
            }
            Ok(output) => {
                missing = true;
                eprintln!(
                    "✗ {program:<8} exited with {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(error) => {
                missing = true;
                eprintln!("✗ {program:<8} {error}");
            }
        }
    }
    if missing {
        bail!("one or more required interactive clients are unavailable")
    }
    Ok(())
}

async fn stop(run_id: &str) -> anyhow::Result<()> {
    let store = RunStore::from_environment()?;
    let manifest = store.load(run_id)?;
    let client = IpcClient::new(store.socket_path(run_id), manifest.admin_token);
    client.call("run.cancel", json!({})).await?;
    println!("Run {run_id} marked cancelled; Zellij panes were preserved.");
    Ok(())
}

async fn resume_agent(run_id: &str, agent_id: &str) -> anyhow::Result<()> {
    let store = RunStore::from_environment()?;
    let manifest = store.load(run_id)?;
    let agent = manifest
        .agents
        .get(agent_id)
        .with_context(|| format!("unknown agent: {agent_id}"))?;
    if agent.provider_session_id.is_none() {
        bail!("agent {agent_id} does not have a provider session ID to resume")
    }
    let runner = Arc::new(TokioCommandRunner);
    let host = ZellijPaneHost::new(manifest.zellij_session.clone(), runner);
    let executable = path_text(&std::env::current_exe()?)?;
    let pane_id = host
        .create_pane(PaneSpec {
            title: format!("[R] Resume {agent_id}"),
            cwd: agent.worktree.clone(),
            command: CommandSpec::new(executable).args(vec![
                "pane".into(),
                "--run-id".into(),
                run_id.into(),
                "--agent-id".into(),
                agent_id.into(),
                "--resume".into(),
            ]),
        })
        .await?;
    let client = IpcClient::new(store.socket_path(run_id), manifest.admin_token);
    register_pane(&client, agent_id, pane_id.as_str()).await?;
    host.focus(pane_id.as_str()).await?;
    attach_terminal(&manifest.zellij_session).await
}

async fn ensure_orchestrator_pane(store: &RunStore, manifest: &RunManifest) -> anyhow::Result<()> {
    let runner = Arc::new(TokioCommandRunner);
    let host = ZellijPaneHost::new(manifest.zellij_session.clone(), runner);
    let panes = host.list_panes().await.unwrap_or_default();
    let existing = manifest
        .agents
        .get("orchestrator")
        .and_then(|agent| agent.pane_id.as_deref())
        .is_some_and(|id| {
            panes
                .iter()
                .any(|pane| pane.pane_id().as_str() == id && !pane.exited)
        });
    if existing {
        return Ok(());
    }
    let executable = path_text(&std::env::current_exe()?)?;
    let pane_id = host
        .create_pane(PaneSpec {
            title: "[O] Osanwe Orchestrator".into(),
            cwd: manifest.integration_worktree.clone(),
            command: CommandSpec::new(executable).args(vec![
                "tui".into(),
                "--run-id".into(),
                manifest.run_id.clone(),
            ]),
        })
        .await?;
    let client = IpcClient::new(
        store.socket_path(&manifest.run_id),
        manifest.admin_token.clone(),
    );
    register_pane(&client, "orchestrator", pane_id.as_str()).await
}

async fn register_pane(client: &IpcClient, agent_id: &str, pane_id: &str) -> anyhow::Result<()> {
    client
        .call(
            "pane.register",
            json!({"agent_id": agent_id, "pane_id": pane_id}),
        )
        .await?;
    Ok(())
}

async fn spawn_daemon(store: &RunStore, manifest: &RunManifest) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("resolve Osanwe executable")?;
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest.run_dir.join("logs/relayd.stdout.log"))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest.run_dir.join("logs/relayd.stderr.log"))?;
    Command::new(executable)
        .args(["daemon", "--run-id", &manifest.run_id])
        .env("OSANWE_STATE_HOME", store.root())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .context("spawn Osanwe daemon")?;
    Ok(())
}

async fn wait_for_daemon(store: &RunStore, manifest: &RunManifest) -> anyhow::Result<()> {
    let client = IpcClient::new(
        store.socket_path(&manifest.run_id),
        manifest.admin_token.clone(),
    );
    let mut last_error = None;
    for _ in 0..60 {
        match client.call("ping", json!({})).await {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        sleep(Duration::from_millis(100)).await;
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("daemon did not start")))
}

async fn attach_terminal(session: &str) -> anyhow::Result<()> {
    let status = Command::new("zellij")
        .args(["attach", session])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("attach to Zellij session")?;
    if status.success() {
        Ok(())
    } else {
        bail!("Zellij attach exited with status {:?}", status.code())
    }
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

fn short_id(value: &str) -> &str {
    &value[..value.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_command_accepts_task_and_repo() {
        let cli = Cli::try_parse_from([
            "osanwe",
            "start",
            "implement auth",
            "--repo",
            "/tmp/repo",
            "--no-attach",
        ])
        .unwrap();
        match cli.command {
            Commands::Start {
                task,
                repo,
                no_attach,
            } => {
                assert_eq!(task, "implement auth");
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
                assert!(no_attach);
            }
            _ => panic!("expected start command"),
        }
    }
}
