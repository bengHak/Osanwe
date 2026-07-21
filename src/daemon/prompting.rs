use std::collections::BTreeSet;

use anyhow::{bail, Context};
use serde_json::json;
use tokio::time::{sleep, Duration};

use crate::model::{
    AgentState, Assignment, AssignmentStatus, ControlOwner, RunStatus, WorkflowEvent,
};
use crate::provider::ProviderDriver;

use super::support::plan_hash;
use super::Daemon;

impl Daemon {
    pub(super) fn schedule_bootstrap(&self, agent_id: String) {
        let daemon = self.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(250)).await;
            if let Err(error) = daemon.inject_bootstrap(&agent_id).await {
                let _ = daemon
                    .append_event(
                        &agent_id,
                        "bootstrap_deferred",
                        json!({"error": error.to_string()}),
                    )
                    .await;
            }
        });
    }

    pub(super) fn schedule_bootstrap_fallback(&self, agent_id: String) {
        let daemon = self.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(2)).await;
            if let Err(error) = daemon.inject_bootstrap(&agent_id).await {
                let _ = daemon
                    .append_event(
                        &agent_id,
                        "bootstrap_fallback_deferred",
                        json!({"error": error.to_string()}),
                    )
                    .await;
            }
        });
    }

    async fn inject_bootstrap(&self, agent_id: &str) -> anyhow::Result<()> {
        let (provider, role) = {
            let run = self.run.lock().await;
            let agent = run.agents.get(agent_id).context("unknown agent")?;
            let provider = agent.provider.context("agent has no LLM provider")?;
            (provider, agent.role)
        };
        let prompt = ProviderDriver::for_provider(provider).bootstrap_prompt(role);
        if prompt.is_empty() {
            return Ok(());
        }
        self.inject_custom_prompt(agent_id, &prompt).await
    }

    pub(super) async fn inject_custom_prompt(
        &self,
        agent_id: &str,
        prompt: &str,
    ) -> anyhow::Result<()> {
        let pane_id = {
            let run = self.run.lock().await;
            let agent = run.agents.get(agent_id).context("unknown agent")?;
            if agent.control_owner != ControlOwner::Orchestrator {
                bail!("agent is under user control")
            }
            if !matches!(
                agent.state,
                AgentState::ProcessStarting
                    | AgentState::SessionReady
                    | AgentState::Idle
                    | AgentState::AssignmentCompleted
            ) {
                bail!("agent is not ready for prompt injection: {:?}", agent.state)
            }
            agent.pane_id.clone().context("agent pane is not registered")?
        };

        self.apply_and_persist(
            crate::state::Transition::AgentStateChanged {
                agent_id: agent_id.into(),
                state: AgentState::PromptQueued,
                message: Some("prompt queued".into()),
            },
            "orchestrator",
            "prompt_queued",
            json!({"agent_id": agent_id}),
        )
        .await?;

        self.pane_host.paste(&pane_id, prompt).await?;
        self.pane_host.send_keys(&pane_id, &["Enter"]).await?;

        self.apply_and_persist(
            crate::state::Transition::AgentStateChanged {
                agent_id: agent_id.into(),
                state: AgentState::PromptSubmitted,
                message: Some("prompt submitted".into()),
            },
            "orchestrator",
            "prompt_submitted",
            json!({"agent_id": agent_id}),
        )
        .await
    }

    pub(super) async fn create_repair_assignment(
        &self,
        fixes: Vec<String>,
    ) -> anyhow::Result<()> {
        let (assignment, snapshot) = {
            let mut run = self.run.lock().await;
            let plan = run.plan.clone().context("repair requires an approved plan")?;
            let repair_number = run
                .assignments
                .iter()
                .filter(|assignment| assignment.id.starts_with('R'))
                .count()
                + 1;
            let attempt = run
                .assignments
                .iter()
                .map(|assignment| assignment.attempt)
                .max()
                .unwrap_or(1)
                + 1;
            let worker = run
                .agents
                .get_mut("worker-1")
                .context("repair requires worker-1")?;
            let mut write_scope = BTreeSet::new();
            let mut acceptance_criteria = BTreeSet::new();
            for step in &plan.steps {
                write_scope.extend(step.write_scope.iter().cloned());
                acceptance_criteria.extend(step.acceptance_criteria.iter().cloned());
            }
            let goal = if fixes.is_empty() {
                "Repair the implementation using the latest check and verification evidence."
                    .into()
            } else {
                format!("Repair the following required findings:\n- {}", fixes.join("\n- "))
            };
            let assignment = Assignment {
                id: format!("R{repair_number}"),
                agent_id: worker.id.clone(),
                goal,
                plan_hash: plan_hash(&plan)?,
                step_ids: plan.steps.iter().map(|step| step.id.clone()).collect(),
                write_scope: write_scope.into_iter().collect(),
                acceptance_criteria: acceptance_criteria.into_iter().collect(),
                status: AssignmentStatus::Active,
                attempt,
            };
            worker.current_assignment_id = Some(assignment.id.clone());
            worker.state = AgentState::Idle;
            run.status = RunStatus::Repairing;
            run.assignments.push(assignment.clone());
            run.touch();
            (assignment, run.clone())
        };

        self.store.save(&snapshot)?;
        self.store.append_event(&WorkflowEvent::new(
            snapshot.run_id,
            "orchestrator",
            "repair_assignment_created",
            serde_json::to_value(&assignment)?,
        ))?;
        self.inject_custom_prompt(
            "worker-1",
            "A repair assignment is ready. Call osanwe.get_assignment and address only the required fixes.",
        )
        .await
    }
}
