use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::ipc::RpcRequest;
use crate::model::AgentRole;

use super::support::{authenticate, requested_agent, require_admin};
use super::{Daemon, Identity};

impl Daemon {
    pub(super) async fn dispatch(&self, request: &RpcRequest) -> anyhow::Result<Value> {
        let identity = {
            let run = self.run.lock().await;
            authenticate(&run, &request.token)?
        };

        match request.method.as_str() {
            "ping" => Ok(json!({"ok": true})),
            "state.get" => self.state_view(&identity).await,
            "assignment.get" => self.assignment_view(&identity, &request.params).await,
            "plan.submit" => self.submit_plan(&identity, &request.params).await,
            "plan.approve" => {
                require_admin(&identity)?;
                self.approve_plan().await
            }
            "assignment.complete" => {
                self.complete_assignment(&identity, &request.params).await
            }
            "checks.complete" => self.complete_checks(&identity, &request.params).await,
            "verification.submit" => {
                self.submit_verification(&identity, &request.params).await
            }
            "progress.report" => self.report_progress(&identity, &request.params).await,
            "input.request" | "deviation.report" => {
                self.request_attention(&identity, &request.params).await
            }
            "hook.record" => self.record_hook(&identity, &request.params).await,
            "session.observe" => self.observe_session(&identity, &request.params).await,
            "pane.register" => {
                require_admin(&identity)?;
                self.register_pane(&request.params).await
            }
            "pane.started" => self.pane_started(&identity, &request.params).await,
            "pane.exited" => self.pane_exited(&identity, &request.params).await,
            "pane.focus" => {
                require_admin(&identity)?;
                self.focus_pane(&request.params).await
            }
            "control.set" => {
                require_admin(&identity)?;
                self.set_control_owner(&request.params).await
            }
            "run.cancel" => {
                require_admin(&identity)?;
                self.cancel_run().await
            }
            other => bail!("unknown daemon method: {other}"),
        }
    }

    async fn state_view(&self, identity: &Identity) -> anyhow::Result<Value> {
        let run = self.run.lock().await.clone();
        match identity {
            Identity::Admin => Ok(serde_json::to_value(run)?),
            Identity::Agent { id, .. } => self.assignment_view(identity, &json!({"agent_id": id})).await,
        }
    }

    async fn assignment_view(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = requested_agent(identity, params)?;
        let run = self.run.lock().await.clone();
        let agent = run
            .agents
            .get(&agent_id)
            .context("unknown agent for assignment")?;
        let value = match agent.role {
            AgentRole::Planner => json!({
                "run_id": run.run_id,
                "role": "planner",
                "task": run.task,
                "repository": run.integration_worktree,
                "base_sha": run.base_sha,
                "existing_plan": run.plan,
                "constraints": {
                    "implementation_forbidden": true,
                    "completion_tool": "osanwe.submit_plan"
                }
            }),
            AgentRole::Worker => json!({
                "run_id": run.run_id.clone(),
                "role": "worker",
                "task": run.task.clone(),
                "plan": run.plan.clone(),
                "assignment": run.current_assignment_for(&agent_id),
                "worktree": agent.worktree.clone(),
                "constraints": {
                    "no_push": true,
                    "no_branch_switch": true,
                    "completion_tool": "osanwe.complete_assignment"
                }
            }),
            AgentRole::Verifier => {
                let patch = self
                    .workspace
                    .diff_patch(&run.integration_worktree, &run.base_sha)
                    .await
                    .unwrap_or_else(|error| format!("unable to read diff: {error}"));
                let stat = self
                    .workspace
                    .diff_stat(&run.integration_worktree, &run.base_sha)
                    .await
                    .unwrap_or_else(|error| format!("unable to read diff stat: {error}"));
                json!({
                    "run_id": run.run_id,
                    "role": "verifier",
                    "task": run.task,
                    "plan": run.plan,
                    "repository": run.integration_worktree,
                    "base_sha": run.base_sha,
                    "diff": patch,
                    "diff_stat": stat,
                    "checks": run.check_results,
                    "constraints": {
                        "read_only": true,
                        "completion_tool": "osanwe.submit_verification"
                    }
                })
            }
            AgentRole::Checks => json!({
                "run_id": run.run_id,
                "role": "checks",
                "checks": run.plan.as_ref().map(|plan| &plan.checks),
                "repository": run.integration_worktree
            }),
            AgentRole::Orchestrator => serde_json::to_value(run)?,
        };
        Ok(value)
    }

}
