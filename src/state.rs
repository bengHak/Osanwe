use serde_json::Value;
use thiserror::Error;

use crate::model::{
    AgentRole, AgentState, AssignmentStatus, CheckResult, PlanSpec, RunManifest, RunStatus,
    VerificationReport, VerificationVerdict, WorkerResult,
};

#[derive(Clone, Debug)]
pub enum Transition {
    PlanSubmitted {
        agent_id: String,
        plan: PlanSpec,
    },
    PlanApproved,
    AssignmentCompleted {
        agent_id: String,
        result: WorkerResult,
    },
    ChecksCompleted {
        results: Vec<CheckResult>,
    },
    VerificationSubmitted {
        agent_id: String,
        report: VerificationReport,
    },
    AgentStateChanged {
        agent_id: String,
        state: AgentState,
        message: Option<String>,
    },
    PaneRegistered {
        agent_id: String,
        pane_id: String,
    },
    ProviderSessionObserved {
        agent_id: String,
        session_id: String,
    },
    AttentionRequested {
        agent_id: String,
        message: String,
        details: Value,
    },
}

pub struct Workflow;

impl Workflow {
    pub fn apply(run: &mut RunManifest, transition: Transition) -> Result<(), WorkflowError> {
        match transition {
            Transition::PlanSubmitted { agent_id, plan } => {
                require_role(run, &agent_id, AgentRole::Planner)?;
                require_status(run, &[RunStatus::Planning, RunStatus::PlanReview])?;
                run.plan = Some(plan);
                run.status = RunStatus::PlanReview;
                set_agent_state(run, &agent_id, AgentState::Idle, Some("plan submitted".into()))?;
            }
            Transition::PlanApproved => {
                require_status(run, &[RunStatus::PlanReview])?;
                if run.plan.is_none() {
                    return Err(WorkflowError::MissingPlan);
                }
                run.status = RunStatus::Scheduling;
            }
            Transition::AssignmentCompleted { agent_id, result } => {
                require_role(run, &agent_id, AgentRole::Worker)?;
                require_status(run, &[RunStatus::Building, RunStatus::Repairing])?;
                let assignment = run
                    .assignments
                    .iter_mut()
                    .find(|assignment| assignment.id == result.assignment_id)
                    .ok_or_else(|| WorkflowError::UnknownAssignment(result.assignment_id.clone()))?;
                if assignment.agent_id != agent_id {
                    return Err(WorkflowError::AssignmentOwnerMismatch);
                }
                assignment.status = result.status;
                set_agent_state(
                    run,
                    &agent_id,
                    AgentState::AssignmentCompleted,
                    Some(result.summary),
                )?;
                run.status = match result.status {
                    AssignmentStatus::Completed => RunStatus::Integrating,
                    AssignmentStatus::Failed | AssignmentStatus::Blocked => RunStatus::Blocked,
                    AssignmentStatus::Queued | AssignmentStatus::Active => {
                        return Err(WorkflowError::IncompleteWorkerResult)
                    }
                };
            }
            Transition::ChecksCompleted { results } => {
                require_status(run, &[RunStatus::Checking])?;
                let required_failed = results
                    .iter()
                    .any(|result| result.required && !result.passed);
                run.check_results = results;
                run.status = if required_failed {
                    RunStatus::Repairing
                } else {
                    RunStatus::Verifying
                };
            }
            Transition::VerificationSubmitted { agent_id, report } => {
                require_role(run, &agent_id, AgentRole::Verifier)?;
                require_status(run, &[RunStatus::Verifying])?;
                run.status = match report.verdict {
                    VerificationVerdict::Pass => RunStatus::Passed,
                    VerificationVerdict::NeedsFix => RunStatus::Repairing,
                    VerificationVerdict::Blocked => RunStatus::Blocked,
                };
                run.verification = Some(report);
                set_agent_state(run, &agent_id, AgentState::Idle, Some("review submitted".into()))?;
            }
            Transition::AgentStateChanged {
                agent_id,
                state,
                message,
            } => {
                set_agent_state(run, &agent_id, state, message)?;
            }
            Transition::PaneRegistered { agent_id, pane_id } => {
                let agent = run
                    .agents
                    .get_mut(&agent_id)
                    .ok_or_else(|| WorkflowError::UnknownAgent(agent_id.clone()))?;
                agent.pane_id = Some(pane_id);
                agent.state = AgentState::ProcessStarting;
            }
            Transition::ProviderSessionObserved {
                agent_id,
                session_id,
            } => {
                let agent = run
                    .agents
                    .get_mut(&agent_id)
                    .ok_or_else(|| WorkflowError::UnknownAgent(agent_id.clone()))?;
                agent.provider_session_id = Some(session_id);
                agent.state = AgentState::SessionReady;
            }
            Transition::AttentionRequested {
                agent_id,
                message,
                details: _,
            } => {
                let agent = run
                    .agents
                    .get_mut(&agent_id)
                    .ok_or_else(|| WorkflowError::UnknownAgent(agent_id.clone()))?;
                agent.state = AgentState::AwaitingUser;
                agent.last_message = Some(message.clone());
                run.attention.push(format!("{agent_id}: {message}"));
            }
        }
        run.touch();
        Ok(())
    }
}

fn require_role(
    run: &RunManifest,
    agent_id: &str,
    expected: AgentRole,
) -> Result<(), WorkflowError> {
    let agent = run
        .agents
        .get(agent_id)
        .ok_or_else(|| WorkflowError::UnknownAgent(agent_id.into()))?;
    if agent.role != expected {
        return Err(WorkflowError::WrongRole {
            expected,
            actual: agent.role,
        });
    }
    Ok(())
}

fn require_status(run: &RunManifest, expected: &[RunStatus]) -> Result<(), WorkflowError> {
    if expected.contains(&run.status) {
        Ok(())
    } else {
        Err(WorkflowError::WrongStatus {
            actual: run.status,
            expected: expected.to_vec(),
        })
    }
}

fn set_agent_state(
    run: &mut RunManifest,
    agent_id: &str,
    state: AgentState,
    message: Option<String>,
) -> Result<(), WorkflowError> {
    let agent = run
        .agents
        .get_mut(agent_id)
        .ok_or_else(|| WorkflowError::UnknownAgent(agent_id.into()))?;
    agent.state = state;
    if message.is_some() {
        agent.last_message = message;
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("unknown agent: {0}")]
    UnknownAgent(String),
    #[error("only the {expected:?} role may perform this operation; actual role: {actual:?}")]
    WrongRole {
        expected: AgentRole,
        actual: AgentRole,
    },
    #[error("invalid run status {actual:?}; expected one of {expected:?}")]
    WrongStatus {
        actual: RunStatus,
        expected: Vec<RunStatus>,
    },
    #[error("plan approval requires a submitted plan")]
    MissingPlan,
    #[error("unknown assignment: {0}")]
    UnknownAssignment(String),
    #[error("assignment does not belong to this worker")]
    AssignmentOwnerMismatch,
    #[error("worker result must be completed, failed, or blocked")]
    IncompleteWorkerResult,
}
