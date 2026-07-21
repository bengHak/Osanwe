use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub type AgentId = String;
pub type AssignmentId = String;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Grok,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Orchestrator,
    Planner,
    Worker,
    Verifier,
    Checks,
}

impl AgentRole {
    #[must_use]
    pub const fn default_provider(self) -> Option<Provider> {
        match self {
            Self::Planner | Self::Verifier => Some(Provider::Codex),
            Self::Worker => Some(Provider::Grok),
            Self::Orchestrator | Self::Checks => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    PreparingWorkspace,
    StartingTerminal,
    Planning,
    PlanReview,
    Scheduling,
    Building,
    Integrating,
    Checking,
    Verifying,
    Repairing,
    Passed,
    Blocked,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Absent,
    PaneCreating,
    ProcessStarting,
    SessionReady,
    PromptQueued,
    PromptSubmitted,
    Thinking,
    ToolRunning,
    ApprovalRequired,
    AwaitingUser,
    Idle,
    AssignmentCompleted,
    Interrupted,
    Exited,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlOwner {
    Orchestrator,
    User,
    Shared,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub role: AgentRole,
    pub provider: Option<Provider>,
    pub state: AgentState,
    pub control_owner: ControlOwner,
    pub pane_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub last_integrated_sha: Option<String>,
    pub worktree: PathBuf,
    pub token: String,
    pub current_assignment_id: Option<AssignmentId>,
    pub last_message: Option<String>,
}

impl AgentRecord {
    #[must_use]
    pub fn new(id: impl Into<String>, role: AgentRole, worktree: PathBuf) -> Self {
        Self {
            id: id.into(),
            role,
            provider: role.default_provider(),
            state: AgentState::Absent,
            control_owner: ControlOwner::Orchestrator,
            pane_id: None,
            provider_session_id: None,
            last_integrated_sha: None,
            worktree,
            token: Uuid::new_v4().simple().to_string(),
            current_assignment_id: None,
            last_message: None,
        }
    }

    #[must_use]
    pub fn test_agent(id: &str, role: AgentRole) -> Self {
        Self::new(id, role, PathBuf::from("/tmp/osanwe-test"))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanSpec {
    pub schema_version: String,
    pub task_summary: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub questions: Vec<String>,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub checks: Vec<CheckSpec>,
    #[serde(default)]
    pub completion_criteria: Vec<String>,
}

impl PlanSpec {
    #[must_use]
    pub fn minimal(summary: &str) -> Self {
        Self {
            schema_version: "1.0".into(),
            task_summary: summary.into(),
            assumptions: Vec::new(),
            questions: Vec::new(),
            steps: vec![PlanStep {
                id: "S1".into(),
                title: summary.into(),
                objective: summary.into(),
                dependencies: Vec::new(),
                write_scope: vec!["**".into()],
                acceptance_criteria: Vec::new(),
            }],
            checks: Vec::new(),
            completion_criteria: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckSpec {
    pub id: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_check_cwd")]
    pub cwd: PathBuf,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

fn default_check_cwd() -> PathBuf {
    PathBuf::from(".")
}

const fn default_true() -> bool {
    true
}

const fn default_timeout_seconds() -> u64 {
    1_200
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Assignment {
    pub id: AssignmentId,
    pub agent_id: AgentId,
    pub goal: String,
    pub plan_hash: String,
    #[serde(default)]
    pub step_ids: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub status: AssignmentStatus,
    #[serde(default)]
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentStatus {
    Queued,
    Active,
    Completed,
    Failed,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkerResult {
    pub assignment_id: AssignmentId,
    pub status: AssignmentStatus,
    pub summary: String,
    #[serde(default)]
    pub changed_paths: Vec<PathBuf>,
    #[serde(default)]
    pub deviations: Vec<String>,
    #[serde(default)]
    pub unresolved_questions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckResult {
    pub id: String,
    pub passed: bool,
    pub required: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: u128,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    Pass,
    NeedsFix,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationReport {
    pub verdict: VerificationVerdict,
    pub summary: String,
    #[serde(default)]
    pub acceptance_results: Vec<AcceptanceResult>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub required_fixes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AcceptanceResult {
    pub criterion: String,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub severity: FindingSeverity,
    pub description: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub blocks_release: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Blocker,
    Critical,
    Major,
    Minor,
    Info,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunManifest {
    pub schema_version: String,
    pub run_id: String,
    pub task: String,
    pub repo_root: PathBuf,
    pub run_dir: PathBuf,
    pub base_sha: String,
    pub integration_worktree: PathBuf,
    pub zellij_session: String,
    pub status: RunStatus,
    pub admin_token: String,
    pub plan: Option<PlanSpec>,
    pub verification: Option<VerificationReport>,
    #[serde(default)]
    pub agents: BTreeMap<AgentId, AgentRecord>,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
    #[serde(default)]
    pub check_results: Vec<CheckResult>,
    #[serde(default)]
    pub attention: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl RunManifest {
    #[must_use]
    pub fn new(
        task: impl Into<String>,
        repo_root: PathBuf,
        run_dir: PathBuf,
        base_sha: String,
        integration_worktree: PathBuf,
    ) -> Self {
        let run_id = Uuid::new_v4().simple().to_string();
        Self::new_with_id(
            run_id,
            task,
            repo_root,
            run_dir,
            base_sha,
            integration_worktree,
        )
    }

    #[must_use]
    pub fn new_with_id(
        run_id: String,
        task: impl Into<String>,
        repo_root: PathBuf,
        run_dir: PathBuf,
        base_sha: String,
        integration_worktree: PathBuf,
    ) -> Self {
        let now = unix_timestamp();
        Self {
            schema_version: "1.0".into(),
            zellij_session: format!("osanwe-{}", &run_id[..run_id.len().min(12)]),
            run_id,
            task: task.into(),
            repo_root,
            run_dir,
            base_sha,
            integration_worktree,
            status: RunStatus::Created,
            admin_token: Uuid::new_v4().simple().to_string(),
            plan: None,
            verification: None,
            agents: BTreeMap::new(),
            assignments: Vec::new(),
            check_results: Vec::new(),
            attention: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[must_use]
    pub fn new_for_test(task: &str) -> Self {
        Self::new_with_id(
            "test-run".into(),
            task,
            PathBuf::from("/tmp/repo"),
            PathBuf::from("/tmp/state/test-run"),
            "abc123".into(),
            PathBuf::from("/tmp/integration"),
        )
    }

    pub fn touch(&mut self) {
        self.updated_at = unix_timestamp();
    }

    #[must_use]
    pub fn agent_by_token(&self, token: &str) -> Option<&AgentRecord> {
        self.agents.values().find(|agent| agent.token == token)
    }

    #[must_use]
    pub fn current_assignment_for(&self, agent_id: &str) -> Option<&Assignment> {
        self.assignments.iter().rev().find(|assignment| {
            assignment.agent_id == agent_id
                && matches!(
                    assignment.status,
                    AssignmentStatus::Queued | AssignmentStatus::Active
                )
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowEvent {
    pub timestamp: u64,
    pub run_id: String,
    pub source: String,
    pub kind: String,
    pub payload: Value,
}

impl WorkflowEvent {
    #[must_use]
    pub fn new(
        run_id: impl Into<String>,
        source: impl Into<String>,
        kind: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            timestamp: unix_timestamp(),
            run_id: run_id.into(),
            source: source.into(),
            kind: kind.into(),
            payload,
        }
    }
}

#[must_use]
pub fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
