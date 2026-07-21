use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::model::{
    AgentRole, Assignment, AssignmentStatus, PlanSpec, Provider, RunManifest,
};
use crate::workspace::validate_relative_scope;

use super::Identity;

pub fn role_label(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "Orchestrator",
        AgentRole::Planner => "Planner",
        AgentRole::Worker => "Worker",
        AgentRole::Verifier => "Verifier",
        AgentRole::Checks => "Checks",
    }
}

pub const fn role_provider(role: AgentRole) -> Option<Provider> {
    role.default_provider()
}

pub(super) fn authenticate(run: &RunManifest, token: &str) -> anyhow::Result<Identity> {
    if token == run.admin_token {
        return Ok(Identity::Admin);
    }
    let agent = run
        .agent_by_token(token)
        .context("invalid Osanwe authentication token")?;
    Ok(Identity::Agent {
        id: agent.id.clone(),
        role: agent.role,
    })
}

pub(super) fn require_admin(identity: &Identity) -> anyhow::Result<()> {
    if matches!(identity, Identity::Admin) {
        Ok(())
    } else {
        bail!("this operation requires the orchestrator")
    }
}

pub(super) fn identity_agent(identity: &Identity) -> anyhow::Result<String> {
    match identity {
        Identity::Agent { id, .. } => Ok(id.clone()),
        Identity::Admin => bail!("this operation requires an agent identity"),
    }
}

pub(super) fn require_role(
    identity: &Identity,
    expected: AgentRole,
) -> anyhow::Result<String> {
    match identity {
        Identity::Agent { id, role } if *role == expected => Ok(id.clone()),
        Identity::Agent { role, .. } => {
            bail!("operation requires {expected:?}; authenticated role is {role:?}")
        }
        Identity::Admin => bail!("operation requires a {expected:?} agent identity"),
    }
}

pub(super) fn require_param_agent(params: &Value, expected: &str) -> anyhow::Result<()> {
    let requested = params
        .get("agent_id")
        .and_then(Value::as_str)
        .context("agent_id is required")?;
    if requested == expected {
        Ok(())
    } else {
        bail!("agent may only operate on its own assignment")
    }
}

pub(super) fn requested_agent(identity: &Identity, params: &Value) -> anyhow::Result<String> {
    match identity {
        Identity::Agent { id, .. } => {
            if params.get("agent_id").is_some() {
                require_param_agent(params, id)?;
            }
            Ok(id.clone())
        }
        Identity::Admin => params
            .get("agent_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("agent_id is required for an orchestrator request"),
    }
}

pub(super) fn validate_plan(plan: &PlanSpec) -> anyhow::Result<()> {
    if plan.schema_version != "1.0" {
        bail!("unsupported plan schema version: {}", plan.schema_version)
    }
    if plan.task_summary.trim().is_empty() {
        bail!("plan task summary cannot be empty")
    }
    if plan.steps.is_empty() {
        bail!("plan must contain at least one step")
    }

    let mut step_ids = BTreeSet::new();
    for step in &plan.steps {
        if step.id.trim().is_empty() {
            bail!("plan step ID cannot be empty")
        }
        if !step_ids.insert(step.id.clone()) {
            bail!("duplicate plan step ID: {}", step.id)
        }
        if step.title.trim().is_empty() || step.objective.trim().is_empty() {
            bail!("plan step {} requires a title and objective", step.id)
        }
        if step.write_scope.is_empty() {
            bail!("plan step {} requires a write scope", step.id)
        }
        for scope in &step.write_scope {
            validate_relative_scope(Path::new(scope))?;
        }
    }
    for step in &plan.steps {
        for dependency in &step.dependencies {
            if dependency == &step.id {
                bail!("plan step {} cannot depend on itself", step.id)
            }
            if !step_ids.contains(dependency) {
                bail!("plan step {} depends on unknown step {dependency}", step.id)
            }
        }
    }

    let mut check_ids = BTreeSet::new();
    for check in &plan.checks {
        if check.id.trim().is_empty() || check.program.trim().is_empty() {
            bail!("every check requires a non-empty ID and program")
        }
        if !check_ids.insert(check.id.clone()) {
            bail!("duplicate check ID: {}", check.id)
        }
        validate_relative_scope(&check.cwd)?;
        if check.timeout_seconds == 0 {
            bail!("check {} timeout must be at least one second", check.id)
        }
    }
    Ok(())
}

pub(super) fn plan_hash(plan: &PlanSpec) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(plan)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(super) fn assignment_from_plan(
    plan: &PlanSpec,
    agent_id: &str,
    plan_hash: String,
) -> Assignment {
    let mut write_scope = BTreeSet::new();
    let mut acceptance_criteria = BTreeSet::new();
    for step in &plan.steps {
        write_scope.extend(step.write_scope.iter().cloned());
        acceptance_criteria.extend(step.acceptance_criteria.iter().cloned());
    }
    Assignment {
        id: "T1".into(),
        agent_id: agent_id.into(),
        goal: plan.task_summary.clone(),
        plan_hash,
        step_ids: plan.steps.iter().map(|step| step.id.clone()).collect(),
        write_scope: write_scope.into_iter().collect(),
        acceptance_criteria: acceptance_criteria.into_iter().collect(),
        status: AssignmentStatus::Active,
        attempt: 1,
    }
}

pub(super) fn pane_title(role: AgentRole, agent_id: &str) -> String {
    match role {
        AgentRole::Orchestrator => "[O] Osanwe Orchestrator".into(),
        AgentRole::Planner => "[P] Codex Planner".into(),
        AgentRole::Worker => format!("[W] Grok {agent_id}"),
        AgentRole::Verifier => "[V] Codex Verifier".into(),
        AgentRole::Checks => "[C] Deterministic Checks".into(),
    }
}

pub(super) fn path_text(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

pub(super) fn short_id(value: &str) -> &str {
    &value[..value.len().min(12)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PlanSpec, PlanStep};

    #[test]
    fn plan_validation_rejects_unknown_dependencies() {
        let mut plan = PlanSpec::minimal("task");
        plan.steps[0].dependencies.push("missing".into());
        assert!(validate_plan(&plan)
            .unwrap_err()
            .to_string()
            .contains("unknown step"));
    }

    #[test]
    fn assignment_combines_step_scope_and_criteria() {
        let plan = PlanSpec {
            schema_version: "1.0".into(),
            task_summary: "task".into(),
            assumptions: Vec::new(),
            questions: Vec::new(),
            steps: vec![
                PlanStep {
                    id: "S1".into(),
                    title: "one".into(),
                    objective: "one".into(),
                    dependencies: Vec::new(),
                    write_scope: vec!["src/**".into()],
                    acceptance_criteria: vec!["works".into()],
                },
                PlanStep {
                    id: "S2".into(),
                    title: "two".into(),
                    objective: "two".into(),
                    dependencies: vec!["S1".into()],
                    write_scope: vec!["tests/**".into()],
                    acceptance_criteria: vec!["tested".into()],
                },
            ],
            checks: Vec::new(),
            completion_criteria: Vec::new(),
        };
        let assignment = assignment_from_plan(&plan, "worker-1", "hash".into());
        assert_eq!(assignment.step_ids, ["S1", "S2"]);
        assert_eq!(assignment.write_scope, ["src/**", "tests/**"]);
    }
}
