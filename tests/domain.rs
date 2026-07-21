use osanwe::model::{
    AgentRecord, AgentRole, AgentState, PlanSpec, RunManifest, RunStatus, VerificationReport,
    VerificationVerdict,
};
use osanwe::state::{Transition, Workflow};

#[test]
fn planner_submission_moves_run_to_plan_review() {
    let mut run = RunManifest::new_for_test("add a feature");
    run.status = RunStatus::Planning;
    run.agents.insert(
        "planner".into(),
        AgentRecord::test_agent("planner", AgentRole::Planner),
    );
    let plan = PlanSpec::minimal("Implement the feature");

    Workflow::apply(
        &mut run,
        Transition::PlanSubmitted {
            agent_id: "planner".into(),
            plan: plan.clone(),
        },
    )
    .expect("planner may submit a plan");

    assert_eq!(run.status, RunStatus::PlanReview);
    assert_eq!(run.plan, Some(plan));
    assert_eq!(run.agents["planner"].state, AgentState::Idle);
}

#[test]
fn worker_cannot_submit_plan() {
    let mut run = RunManifest::new_for_test("add a feature");
    run.status = RunStatus::Planning;
    run.agents.insert(
        "worker-1".into(),
        AgentRecord::test_agent("worker-1", AgentRole::Worker),
    );

    let error = Workflow::apply(
        &mut run,
        Transition::PlanSubmitted {
            agent_id: "worker-1".into(),
            plan: PlanSpec::minimal("invalid"),
        },
    )
    .expect_err("worker plan submission must be rejected");

    assert!(error.to_string().contains("Planner"));
}

#[test]
fn passing_verification_completes_run() {
    let mut run = RunManifest::new_for_test("add a feature");
    run.status = RunStatus::Verifying;
    run.agents.insert(
        "verifier".into(),
        AgentRecord::test_agent("verifier", AgentRole::Verifier),
    );

    Workflow::apply(
        &mut run,
        Transition::VerificationSubmitted {
            agent_id: "verifier".into(),
            report: VerificationReport {
                verdict: VerificationVerdict::Pass,
                summary: "all criteria satisfied".into(),
                acceptance_results: Vec::new(),
                findings: Vec::new(),
                required_fixes: Vec::new(),
            },
        },
    )
    .expect("verifier may submit a report");

    assert_eq!(run.status, RunStatus::Passed);
}
