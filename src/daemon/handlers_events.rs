use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::model::{AgentState, ControlOwner, RunStatus};
use crate::state::Transition;

use super::support::{identity_agent, require_param_agent};
use super::{Daemon, Identity};

impl Daemon {
    pub(super) async fn report_progress(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .context("progress.report requires message")?
            .to_owned();
        self.apply_and_persist(
            Transition::AgentStateChanged {
                agent_id: agent_id.clone(),
                state: AgentState::Thinking,
                message: Some(message.clone()),
            },
            &agent_id,
            "progress",
            json!({"message": message}),
        )
        .await?;
        Ok(json!({"accepted": true}))
    }

    pub(super) async fn request_attention(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .context("input.request requires message")?
            .to_owned();
        let details = params.get("details").cloned().unwrap_or_else(|| json!({}));
        self.apply_and_persist(
            Transition::AttentionRequested {
                agent_id: agent_id.clone(),
                message: message.clone(),
                details: details.clone(),
            },
            &agent_id,
            "attention_requested",
            json!({"message": message, "details": details}),
        )
        .await?;
        Ok(json!({"accepted": true, "attention": true}))
    }

    pub(super) async fn record_hook(&self, identity: &Identity, params: &Value) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        let event_name = params
            .get("event_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let event = params.get("event").cloned().unwrap_or(Value::Null);
        self.append_event(&agent_id, &format!("hook.{event_name}"), event.clone())
            .await?;

        let state = match event_name {
            "SessionStart" => Some(AgentState::SessionReady),
            "UserPromptSubmit" => Some(AgentState::Thinking),
            "PreToolUse" => Some(AgentState::ToolRunning),
            "PermissionRequest" | "PermissionDenied" => Some(AgentState::ApprovalRequired),
            "PostToolUse" | "PostToolUseFailure" => Some(AgentState::Thinking),
            "Stop" | "StopFailure" => Some(AgentState::Idle),
            "SessionEnd" => Some(AgentState::Exited),
            _ => None,
        };
        if let Some(state) = state {
            self.apply_and_persist(
                Transition::AgentStateChanged {
                    agent_id: agent_id.clone(),
                    state,
                    message: Some(event_name.into()),
                },
                &agent_id,
                "agent_state",
                json!({"state": state}),
            )
            .await?;
        }
        if event_name == "SessionStart" {
            if let Some(session_id) = event
                .get("session_id")
                .or_else(|| event.get("sessionId"))
                .and_then(Value::as_str)
            {
                self.observe_session(
                    identity,
                    &json!({"agent_id": agent_id, "session_id": session_id}),
                )
                .await?;
            } else {
                self.schedule_bootstrap(agent_id);
            }
        }
        Ok(json!({"recorded": true}))
    }

    pub(super) async fn observe_session(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        let session_id = params
            .get("session_id")
            .and_then(Value::as_str)
            .context("session.observe requires session_id")?
            .to_owned();
        self.apply_and_persist(
            Transition::ProviderSessionObserved {
                agent_id: agent_id.clone(),
                session_id,
            },
            &agent_id,
            "session_observed",
            json!({}),
        )
        .await?;
        self.schedule_bootstrap(agent_id);
        Ok(json!({"recorded": true}))
    }


    pub(super) async fn register_pane(&self, params: &Value) -> anyhow::Result<Value> {
        let agent_id = params
            .get("agent_id")
            .and_then(Value::as_str)
            .context("pane.register requires agent_id")?
            .to_owned();
        let pane_id = params
            .get("pane_id")
            .and_then(Value::as_str)
            .context("pane.register requires pane_id")?
            .to_owned();
        self.apply_and_persist(
            Transition::PaneRegistered {
                agent_id: agent_id.clone(),
                pane_id: pane_id.clone(),
            },
            "orchestrator",
            "pane_registered",
            json!({"agent_id": agent_id, "pane_id": pane_id}),
        )
        .await?;
        Ok(json!({"registered": true}))
    }

    pub(super) async fn pane_started(&self, identity: &Identity, params: &Value) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        self.apply_and_persist(
            Transition::AgentStateChanged {
                agent_id: agent_id.clone(),
                state: AgentState::ProcessStarting,
                message: Some("provider process started".into()),
            },
            &agent_id,
            "pane_started",
            params.clone(),
        )
        .await?;
        self.schedule_bootstrap_fallback(agent_id);
        Ok(json!({"recorded": true}))
    }

    pub(super) async fn pane_exited(&self, identity: &Identity, params: &Value) -> anyhow::Result<Value> {
        let agent_id = identity_agent(identity)?;
        require_param_agent(params, &agent_id)?;
        let success = params.get("success").and_then(Value::as_bool).unwrap_or(false);
        self.apply_and_persist(
            Transition::AgentStateChanged {
                agent_id: agent_id.clone(),
                state: if success {
                    AgentState::Exited
                } else {
                    AgentState::Failed
                },
                message: Some("provider process exited".into()),
            },
            &agent_id,
            "pane_exited",
            params.clone(),
        )
        .await?;
        Ok(json!({"recorded": true}))
    }

    pub(super) async fn focus_pane(&self, params: &Value) -> anyhow::Result<Value> {
        let agent_id = params
            .get("agent_id")
            .and_then(Value::as_str)
            .context("pane.focus requires agent_id")?;
        let pane_id = {
            let run = self.run.lock().await;
            run.agents
                .get(agent_id)
                .and_then(|agent| agent.pane_id.clone())
                .context("agent does not have a pane")?
        };
        self.pane_host.focus(&pane_id).await?;
        Ok(json!({"focused": pane_id}))
    }

    pub(super) async fn set_control_owner(&self, params: &Value) -> anyhow::Result<Value> {
        let agent_id = params
            .get("agent_id")
            .and_then(Value::as_str)
            .context("control.set requires agent_id")?;
        let owner = match params.get("owner").and_then(Value::as_str) {
            Some("orchestrator") => ControlOwner::Orchestrator,
            Some("user") => ControlOwner::User,
            Some("shared") => ControlOwner::Shared,
            _ => bail!("control owner must be orchestrator, user, or shared"),
        };
        let snapshot = {
            let mut run = self.run.lock().await;
            let agent = run.agents.get_mut(agent_id).context("unknown agent")?;
            agent.control_owner = owner;
            run.touch();
            run.clone()
        };
        self.store.save(&snapshot)?;
        Ok(json!({"agent_id": agent_id, "owner": owner}))
    }

    pub(super) async fn cancel_run(&self) -> anyhow::Result<Value> {
        self.set_run_status(RunStatus::Cancelled).await?;
        Ok(json!({"cancelled": true}))
    }
}
