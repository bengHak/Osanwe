use anyhow::Context;
use serde_json::json;
use uuid::Uuid;

use crate::model::{AgentRecord, AgentRole, AgentState, RunStatus};
use crate::process::CommandSpec;
use crate::provider::{LaunchContext, ProviderDriver};
use crate::state::Transition;
use crate::zellij::PaneSpec;

use super::support::{assignment_from_plan, pane_title, path_text, plan_hash, short_id};
use super::Daemon;

impl Daemon {
    pub(super) async fn spawn_worker(&self) -> anyhow::Result<()> {
        let (run_id, repo_root, run_dir, base_sha, plan, worker_exists) = {
            let run = self.run.lock().await;
            (
                run.run_id.clone(),
                run.repo_root.clone(),
                run.run_dir.clone(),
                run.base_sha.clone(),
                run.plan.clone().context("approved plan is missing")?,
                run.agents.contains_key("worker-1"),
            )
        };
        if worker_exists {
            return self
                .inject_custom_prompt(
                    "worker-1",
                    "A new assignment is ready. Call osanwe.get_assignment and continue.",
                )
                .await;
        }
        let worker_id = "worker-1".to_owned();
        let worktree = run_dir.join("worktrees/worker-1");
        self.workspace
            .create_worktree(
                &repo_root,
                &worktree,
                &format!("osanwe/{}/worker-1", short_id(&run_id)),
                &base_sha,
            )
            .await?;
        let plan_hash = plan_hash(&plan)?;
        let assignment = assignment_from_plan(&plan, &worker_id, plan_hash);
        let mut worker = AgentRecord::new(&worker_id, AgentRole::Worker, worktree);
        worker.provider_session_id = Some(Uuid::new_v4().to_string());
        worker.current_assignment_id = Some(assignment.id.clone());
        worker.state = AgentState::PaneCreating;
        let snapshot = {
            let mut run = self.run.lock().await;
            run.status = RunStatus::Building;
            run.assignments.push(assignment);
            run.agents.insert(worker_id.clone(), worker);
            run.touch();
            run.clone()
        };
        self.store.save(&snapshot)?;
        self.spawn_provider_agent(&worker_id).await
    }

    pub(super) async fn spawn_verifier(&self) -> anyhow::Result<()> {
        let (worktree, exists) = {
            let run = self.run.lock().await;
            (
                run.integration_worktree.clone(),
                run.agents.contains_key("verifier"),
            )
        };
        if exists {
            return self
                .inject_custom_prompt(
                    "verifier",
                    "Verification evidence changed. Call osanwe.get_assignment and verify again.",
                )
                .await;
        }
        let mut verifier = AgentRecord::new("verifier", AgentRole::Verifier, worktree);
        verifier.state = AgentState::PaneCreating;
        let snapshot = {
            let mut run = self.run.lock().await;
            run.agents.insert("verifier".into(), verifier);
            run.touch();
            run.clone()
        };
        self.store.save(&snapshot)?;
        self.spawn_provider_agent("verifier").await
    }

    pub(super) async fn spawn_checks(&self) -> anyhow::Result<()> {
        let (run_id, worktree, session, existing_pane) = {
            let mut run = self.run.lock().await;
            let integration_worktree = run.integration_worktree.clone();
            let checks = run.agents.entry("checks".into()).or_insert_with(|| {
                AgentRecord::new("checks", AgentRole::Checks, integration_worktree)
            });
            checks.state = AgentState::PaneCreating;
            (
                run.run_id.clone(),
                run.integration_worktree.clone(),
                run.zellij_session.clone(),
                checks.pane_id.clone(),
            )
        };
        if let Some(pane) = existing_pane {
            self.pane_host.close(&pane).await.ok();
        }
        let executable = path_text(&self.executable)?;
        let pane_id = self
            .pane_host
            .create_pane(PaneSpec {
                title: "[C] Deterministic Checks".into(),
                cwd: worktree,
                command: CommandSpec::new(executable)
                    .args(["checks", "--run-id", &run_id]),
            })
            .await?;
        self.apply_and_persist(
            Transition::PaneRegistered {
                agent_id: "checks".into(),
                pane_id: pane_id.as_str().into(),
            },
            "checks",
            "pane_registered",
            json!({"pane_id": pane_id.as_str(), "zellij_session": session}),
        )
        .await?;
        Ok(())
    }

    pub(super) async fn spawn_provider_agent(&self, agent_id: &str) -> anyhow::Result<()> {
        let (run, agent) = {
            let run = self.run.lock().await;
            let agent = run
                .agents
                .get(agent_id)
                .cloned()
                .context("unknown provider agent")?;
            (run.clone(), agent)
        };
        let provider = agent.provider.context("agent does not have a provider")?;
        let context = LaunchContext::from_environment(
            run.run_id.clone(),
            agent.id.clone(),
            agent.role,
            agent.worktree.clone(),
            run.run_dir.clone(),
            self.store.socket_path(&run.run_id),
            agent.token.clone(),
            self.executable.clone(),
        );
        let context = LaunchContext {
            provider_session_id: agent.provider_session_id.clone(),
            ..context
        };
        ProviderDriver::for_provider(provider).prepare_overlay(&context)?;

        let executable = path_text(&self.executable)?;
        let pane_id = self
            .pane_host
            .create_pane(PaneSpec {
                title: pane_title(agent.role, &agent.id),
                cwd: agent.worktree.clone(),
                command: CommandSpec::new(executable).args([
                    "pane",
                    "--run-id",
                    &run.run_id,
                    "--agent-id",
                    &agent.id,
                ]),
            })
            .await?;
        self.apply_and_persist(
            Transition::PaneRegistered {
                agent_id: agent.id.clone(),
                pane_id: pane_id.as_str().into(),
            },
            "orchestrator",
            "pane_registered",
            json!({"agent_id": agent.id, "pane_id": pane_id.as_str()}),
        )
        .await?;
        Ok(())
    }

}
