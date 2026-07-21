use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::model::{
    AgentRole, AssignmentStatus, CheckResult, PlanSpec, RunStatus, VerificationReport,
    VerificationVerdict, WorkerResult,
};
use crate::state::Transition;

use super::support::{plan_hash, require_param_agent, require_role, validate_plan};
use super::{Daemon, Identity};

impl Daemon {
    pub(super) async fn submit_plan(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = require_role(identity, AgentRole::Planner)?;
        require_param_agent(params, &agent_id)?;
        let plan: PlanSpec = serde_json::from_value(
            params
                .get("plan")
                .cloned()
                .context("plan.submit requires plan")?,
        )?;
        validate_plan(&plan)?;
        self.apply_and_persist(
            Transition::PlanSubmitted {
                agent_id: agent_id.clone(),
                plan: plan.clone(),
            },
            &agent_id,
            "plan_submitted",
            serde_json::to_value(&plan)?,
        )
        .await?;
        let run_id = self.run.lock().await.run_id.clone();
        self.store.write_artifact(
            &run_id,
            Path::new("plan.json"),
            &serde_json::to_vec_pretty(&plan)?,
        )?;
        Ok(json!({"accepted": true, "status": "plan_review", "plan_hash": plan_hash(&plan)?}))
    }

    pub(super) async fn approve_plan(&self) -> anyhow::Result<Value> {
        self.apply_and_persist(
            Transition::PlanApproved,
            "orchestrator",
            "plan_approved",
            json!({}),
        )
        .await?;
        self.spawn_worker().await?;
        Ok(json!({"approved": true, "status": "building"}))
    }

    pub(super) async fn complete_assignment(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = require_role(identity, AgentRole::Worker)?;
        require_param_agent(params, &agent_id)?;
        let result: WorkerResult = serde_json::from_value(
            params
                .get("result")
                .cloned()
                .context("assignment.complete requires result")?,
        )?;
        self.apply_and_persist(
            Transition::AssignmentCompleted {
                agent_id: agent_id.clone(),
                result: result.clone(),
            },
            &agent_id,
            "assignment_completed",
            serde_json::to_value(&result)?,
        )
        .await?;

        let run_id = self.run.lock().await.run_id.clone();
        self.store.write_artifact(
            &run_id,
            &PathBuf::from("assignments").join(format!("{}.result.json", result.assignment_id)),
            &serde_json::to_vec_pretty(&result)?,
        )?;
        if result.status != AssignmentStatus::Completed {
            return Ok(json!({"accepted": true, "status": "blocked"}));
        }

        let (worktree, integration, integration_base, original_base) = {
            let run = self.run.lock().await;
            let agent = run.agents.get(&agent_id).context("worker disappeared")?;
            (
                agent.worktree.clone(),
                run.integration_worktree.clone(),
                agent
                    .last_integrated_sha
                    .clone()
                    .unwrap_or_else(|| run.base_sha.clone()),
                run.base_sha.clone(),
            )
        };
        let checkpoint = self
            .workspace
            .checkpoint(
                &worktree,
                &integration_base,
                &format!("osanwe: complete {}", result.assignment_id),
            )
            .await?;
        if let Some(checkpoint) = checkpoint {
            if let Err(error) = self
                .workspace
                .integrate(&integration, &integration_base, &checkpoint)
                .await
            {
                self.block_run(format!("integration conflict: {error}"))
                    .await?;
                return Err(error);
            }
            {
                let mut run = self.run.lock().await;
                if let Some(agent) = run.agents.get_mut(&agent_id) {
                    agent.last_integrated_sha = Some(checkpoint.clone());
                }
                let snapshot = run.clone();
                drop(run);
                self.store.save(&snapshot)?;
            }
            let patch = self
                .workspace
                .diff_patch(&integration, &original_base)
                .await?;
            self.store.write_artifact(
                &run_id,
                Path::new("checkpoints/integration.patch"),
                patch.as_bytes(),
            )?;
        }
        self.set_run_status(RunStatus::Checking).await?;
        self.spawn_checks().await?;
        Ok(json!({"accepted": true, "status": "checking"}))
    }

    pub(super) async fn complete_checks(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        match identity {
            Identity::Admin => {}
            Identity::Agent { role, .. } if *role == AgentRole::Checks => {}
            _ => bail!("only the checks pane may submit deterministic results"),
        }
        let results: Vec<CheckResult> = serde_json::from_value(
            params
                .get("results")
                .cloned()
                .context("checks.complete requires results")?,
        )?;
        self.apply_and_persist(
            Transition::ChecksCompleted {
                results: results.clone(),
            },
            "checks",
            "checks_completed",
            serde_json::to_value(&results)?,
        )
        .await?;
        let status = self.run.lock().await.status;
        if status == RunStatus::Verifying {
            self.spawn_verifier().await?;
        } else if status == RunStatus::Repairing {
            let failures: Vec<String> = results
                .iter()
                .filter(|result| result.required && !result.passed)
                .map(|result| format!("{}: {}", result.id, result.summary))
                .collect();
            self.create_repair_assignment(failures).await?;
        }
        Ok(json!({"accepted": true, "status": status}))
    }

    pub(super) async fn submit_verification(
        &self,
        identity: &Identity,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let agent_id = require_role(identity, AgentRole::Verifier)?;
        require_param_agent(params, &agent_id)?;
        let report: VerificationReport = serde_json::from_value(
            params
                .get("report")
                .cloned()
                .context("verification.submit requires report")?,
        )?;
        if report.verdict == VerificationVerdict::Pass
            && report.findings.iter().any(|finding| finding.blocks_release)
        {
            bail!("verification cannot pass while a release-blocking finding exists")
        }
        self.apply_and_persist(
            Transition::VerificationSubmitted {
                agent_id: agent_id.clone(),
                report: report.clone(),
            },
            &agent_id,
            "verification_submitted",
            serde_json::to_value(&report)?,
        )
        .await?;
        let run_id = self.run.lock().await.run_id.clone();
        self.store.write_artifact(
            &run_id,
            Path::new("verification.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        if report.verdict == VerificationVerdict::NeedsFix {
            self.create_repair_assignment(report.required_fixes.clone())
                .await?;
        }
        Ok(json!({"accepted": true, "status": self.run.lock().await.status}))
    }
}
