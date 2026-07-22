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
use crate::onboard;
use crate::pane;
use crate::process::{CommandSpec, TokioCommandRunner};
use crate::project::{self, config_exists};
use crate::session_launch;
use crate::store::RunStore;
use crate::workspace::WorkspaceManager;
use crate::zellij::{PaneHost, PaneSpec, ZellijPaneHost};

#[derive(Debug, Parser)]
#[command(
    name = "osanwe",
    version,
    about = "Launch Codex/Grok sessions with a project-local .osanwe file bus"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run first-time (or forced) onboarding: pick client + model per role.
    Onboard {
        /// Git repository / project root.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Write default role choices without opening the TUI.
        #[arg(long)]
        defaults: bool,
        /// Overwrite existing `.osanwe/config.toml`.
        #[arg(long)]
        force: bool,
        /// After onboarding, start the Zellij session.
        #[arg(long)]
        launch: bool,
        /// Create the session without attaching this terminal.
        #[arg(long)]
        no_attach: bool,
    },
    /// Attach to the project's Zellij session.
    Attach {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Legacy: attach by home-state run id.
        run_id: Option<String>,
    },
    /// Stop the project's Zellij session.
    Stop {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Legacy: cancel a home-state run by id.
        run_id: Option<String>,
    },
    /// Check required executables and print discovered versions.
    Doctor,
    /// Print the project board (used as a Zellij pane).
    #[command(hide = true)]
    Board {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Legacy: start a daemon-driven run with an explicit task string.
    #[command(hide = true)]
    Start {
        /// Task given to the Codex planner.
        task: String,
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        no_attach: bool,
    },
    /// Legacy: list home-state runs.
    #[command(hide = true)]
    List,
    /// Legacy: resume an exited interactive provider pane.
    #[command(hide = true)]
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
        None => default_entry(PathBuf::from("."), false).await,
        Some(Commands::Onboard {
            repo,
            defaults,
            force,
            launch,
            no_attach,
        }) => onboard_command(repo, defaults, force, launch, no_attach).await,
        Some(Commands::Attach { repo, run_id }) => {
            if let Some(run_id) = run_id {
                attach_legacy(&run_id).await
            } else {
                let root = project::find_project_root(&repo)?;
                session_launch::attach_project_session(&root).await
            }
        }
        Some(Commands::Stop { repo, run_id }) => {
            if let Some(run_id) = run_id {
                stop_legacy(&run_id).await
            } else {
                let root = project::find_project_root(&repo)?;
                session_launch::stop_project_session(&root).await
            }
        }
        Some(Commands::Doctor) => doctor().await,
        Some(Commands::Board { repo }) => {
            let root = project::find_project_root(&repo)?;
            session_launch::run_board(&root)
        }
        Some(Commands::Start {
            task,
            repo,
            no_attach,
        }) => start_legacy(task, repo, no_attach).await,
        Some(Commands::List) => list_runs(),
        Some(Commands::Resume { run_id, agent_id }) => resume_agent(&run_id, &agent_id).await,
        Some(Commands::Daemon { run_id }) => {
            daemon::run_daemon(RunStore::from_environment()?, &run_id).await
        }
        Some(Commands::Tui { run_id }) => {
            crate::tui::run(RunStore::from_environment()?, &run_id).await
        }
        Some(Commands::Pane {
            run_id,
            agent_id,
            resume,
        }) => pane::run_agent_pane(RunStore::from_environment()?, &run_id, &agent_id, resume).await,
        Some(Commands::Bridge) => mcp::run_stdio().await,
        Some(Commands::Hook) => hook::forward_stdin().await,
        Some(Commands::Checks { run_id }) => {
            pane::run_check_pane(RunStore::from_environment()?, &run_id).await
        }
    }
}

/// Bare `osanwe`: onboard if needed, otherwise launch the project session.
async fn default_entry(repo: PathBuf, no_attach: bool) -> anyhow::Result<()> {
    require_unix()?;
    let root = project::find_project_root(&repo)?;
    if !config_exists(&root) {
        println!("No `.osanwe/config.toml` found — starting onboarding…");
        let config = onboard::run_onboarding(&root)?;
        println!(
            "Onboarding complete (session {}). Launching…",
            config.zellij_session
        );
    } else {
        // Ensure scaffold dirs still exist.
        project::scaffold(&root)?;
    }
    session_launch::launch_project_session(&root, no_attach).await
}

async fn onboard_command(
    repo: PathBuf,
    defaults: bool,
    force: bool,
    launch: bool,
    no_attach: bool,
) -> anyhow::Result<()> {
    let root = project::find_project_root(&repo)?;
    if config_exists(&root) && !force {
        bail!(
            "`.osanwe/config.toml` already exists; pass --force to re-onboard or run bare `osanwe` to launch"
        );
    }
    let config = if defaults {
        onboard::apply_defaults(&root)?
    } else {
        onboard::run_onboarding(&root)?
    };
    println!(
        "Wrote {} (session {})",
        project::config_path(&root).display(),
        config.zellij_session
    );
    if launch {
        require_unix()?;
        session_launch::launch_project_session(&root, no_attach).await?;
    }
    Ok(())
}

async fn start_legacy(task: String, repo: PathBuf, no_attach: bool) -> anyhow::Result<()> {
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
        session_launch::attach_terminal(&manifest.zellij_session).await
    }
}

async fn attach_legacy(run_id: &str) -> anyhow::Result<()> {
    require_unix()?;
    let store = RunStore::from_environment()?;
    let manifest = store.load(run_id)?;
    if wait_for_daemon(&store, &manifest).await.is_err() {
        spawn_daemon(&store, &manifest).await?;
        wait_for_daemon(&store, &manifest).await?;
    }
    ensure_orchestrator_pane(&store, &manifest).await?;
    session_launch::attach_terminal(&manifest.zellij_session).await
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
    println!("Osanwe checks required runtime tools (this binary does not bundle them).\n");

    let checks = [
        (
            "git",
            vec!["--version"],
            "project root detection",
            "Install Git for your OS (e.g. `brew install git` or distro package `git`).",
        ),
        (
            "zellij",
            vec!["--version"],
            "multi-pane session host (0.44+ required)",
            "Install Zellij 0.44+: macOS `brew install zellij`; Linux `cargo install --locked zellij` or https://github.com/zellij-org/zellij/releases",
        ),
        (
            "codex",
            vec!["--version"],
            "interactive Codex CLI (authenticate after install)",
            "Install Codex CLI and sign in: https://github.com/openai/codex (or your usual Codex install path).",
        ),
        (
            "grok",
            vec!["version"],
            "interactive Grok Build CLI (authenticate after install)",
            "Install Grok Build and sign in: https://x.ai/cli",
        ),
    ];

    let mut missing: Vec<(&str, &str)> = Vec::new();
    for (program, args, purpose, hint) in checks {
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
                println!("         {purpose}");
            }
            Ok(output) => {
                missing.push((program, hint));
                eprintln!(
                    "✗ {program:<8} exited with {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                eprintln!("         {purpose}");
            }
            Err(error) => {
                missing.push((program, hint));
                eprintln!("✗ {program:<8} {error}");
                eprintln!("         {purpose}");
            }
        }
    }

    println!();
    if missing.is_empty() {
        println!(
            "All required tools are available. Run `osanwe` in a project to onboard and launch."
        );
        Ok(())
    } else {
        println!("Missing or broken tools — install guidance:\n");
        for (program, hint) in &missing {
            println!("  • {program}");
            println!("      {hint}");
        }
        println!();
        println!("After installing, re-run: osanwe doctor");
        println!("Then from a git project:   osanwe");
        bail!("one or more required runtime tools are unavailable")
    }
}

async fn stop_legacy(run_id: &str) -> anyhow::Result<()> {
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
                "pane",
                "--run-id",
                run_id,
                "--agent-id",
                agent_id,
                "--resume",
            ]),
        })
        .await?;
    let client = IpcClient::new(store.socket_path(run_id), manifest.admin_token);
    register_pane(&client, agent_id, pane_id.as_str()).await?;
    host.focus(pane_id.as_str()).await?;
    session_launch::attach_terminal(&manifest.zellij_session).await
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
    use clap::Parser;

    #[test]
    fn bare_osanwe_parses_as_no_subcommand() {
        let cli = Cli::try_parse_from(["osanwe"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn onboard_defaults_flag_parses() {
        let cli = Cli::try_parse_from([
            "osanwe",
            "onboard",
            "--defaults",
            "--repo",
            "/tmp/proj",
            "--force",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Onboard {
                repo,
                defaults,
                force,
                launch,
                no_attach,
            }) => {
                assert_eq!(repo, PathBuf::from("/tmp/proj"));
                assert!(defaults);
                assert!(force);
                assert!(!launch);
                assert!(!no_attach);
            }
            _ => panic!("expected onboard"),
        }
    }

    #[test]
    fn doctor_and_attach_are_visible_commands() {
        let cli = Cli::try_parse_from(["osanwe", "doctor"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Doctor)));
        let cli = Cli::try_parse_from(["osanwe", "attach", "--repo", "."]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Attach { run_id: None, .. })
        ));
    }

    #[test]
    fn legacy_start_still_parses() {
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
            Some(Commands::Start {
                task,
                repo,
                no_attach,
            }) => {
                assert_eq!(task, "implement auth");
                assert_eq!(repo, PathBuf::from("/tmp/repo"));
                assert!(no_attach);
            }
            _ => panic!("expected start command"),
        }
    }
}
