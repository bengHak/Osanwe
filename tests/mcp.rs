use osanwe::mcp::{tool_definitions_for, McpToolCall};
use osanwe::model::AgentRole;

#[test]
fn planner_and_worker_receive_different_mutating_tools() {
    let planner = tool_definitions_for(AgentRole::Planner);
    let worker = tool_definitions_for(AgentRole::Worker);

    assert!(planner.iter().any(|tool| tool.name == "submit_plan"));
    assert!(!planner
        .iter()
        .any(|tool| tool.name == "complete_assignment"));
    assert!(worker
        .iter()
        .any(|tool| tool.name == "complete_assignment"));
    assert!(!worker.iter().any(|tool| tool.name == "submit_plan"));
}

#[test]
fn tool_call_maps_to_role_scoped_daemon_method() {
    let call = McpToolCall {
        name: "submit_plan".into(),
        arguments: serde_json::json!({"plan": {"schema_version": "1.0"}}),
    };

    let request = call
        .into_daemon_request(AgentRole::Planner, "planner")
        .expect("valid planner call");

    assert_eq!(request.method, "plan.submit");
    assert_eq!(request.params["agent_id"], "planner");
}
