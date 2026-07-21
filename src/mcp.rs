use std::env;

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::ipc::{IpcClient, RpcRequest};
use crate::model::AgentRole;
use crate::provider::parse_role;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[must_use]
pub fn tool_definitions_for(role: AgentRole) -> Vec<ToolDefinition> {
    let mut tools = vec![
        tool(
            "get_assignment",
            "Return the current role-scoped assignment and run evidence.",
            json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ),
        tool(
            "report_progress",
            "Report concise progress without marking the assignment complete.",
            json!({
                "type": "object",
                "properties": {"message": {"type": "string"}},
                "required": ["message"],
                "additionalProperties": false
            }),
        ),
        tool(
            "request_input",
            "Ask the user for input or approval and pause autonomous orchestration.",
            json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"},
                    "details": {"type": "object"}
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        ),
    ];
    match role {
        AgentRole::Planner => tools.push(tool(
            "submit_plan",
            "Submit a complete structured implementation plan for user review.",
            json!({
                "type": "object",
                "properties": {"plan": plan_schema()},
                "required": ["plan"],
                "additionalProperties": false
            }),
        )),
        AgentRole::Worker => {
            tools.push(tool(
                "complete_assignment",
                "Submit the result of the current implementation or repair assignment.",
                json!({
                    "type": "object",
                    "properties": {"result": worker_result_schema()},
                    "required": ["result"],
                    "additionalProperties": false
                }),
            ));
            tools.push(tool(
                "report_deviation",
                "Report a required deviation from the approved plan before proceeding.",
                json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string"},
                        "details": {"type": "object"}
                    },
                    "required": ["message"],
                    "additionalProperties": false
                }),
            ));
        }
        AgentRole::Verifier => tools.push(tool(
            "submit_verification",
            "Submit the independent verification verdict and findings.",
            json!({
                "type": "object",
                "properties": {"report": verification_schema()},
                "required": ["report"],
                "additionalProperties": false
            }),
        )),
        AgentRole::Orchestrator | AgentRole::Checks => {}
    }
    tools
}

#[derive(Clone, Debug, Deserialize)]
pub struct McpToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

impl McpToolCall {
    pub fn into_daemon_request(
        self,
        role: AgentRole,
        agent_id: &str,
    ) -> anyhow::Result<RpcRequest> {
        let mut params = match self.arguments {
            Value::Object(map) => map,
            Value::Null => serde_json::Map::new(),
            _ => bail!("tool arguments must be an object"),
        };
        params.insert("agent_id".into(), Value::String(agent_id.into()));
        let method = match (role, self.name.as_str()) {
            (_, "get_assignment") => "assignment.get",
            (_, "report_progress") => "progress.report",
            (_, "request_input") => "input.request",
            (AgentRole::Planner, "submit_plan") => "plan.submit",
            (AgentRole::Worker, "complete_assignment") => "assignment.complete",
            (AgentRole::Worker, "report_deviation") => "deviation.report",
            (AgentRole::Verifier, "submit_verification") => "verification.submit",
            _ => bail!("tool {} is not allowed for role {role:?}", self.name),
        };
        Ok(RpcRequest::new(method, Value::Object(params), ""))
    }
}

pub async fn run_stdio() -> anyhow::Result<()> {
    let role = parse_role(&env::var("OSANWE_ROLE").context("OSANWE_ROLE is not set")?)?;
    let agent_id = env::var("OSANWE_AGENT_ID").context("OSANWE_AGENT_ID is not set")?;
    let token = env::var("OSANWE_AGENT_TOKEN").context("OSANWE_AGENT_TOKEN is not set")?;
    let client = IpcClient::from_environment()?;

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_message(
                    &mut stdout,
                    &json_rpc_error(Value::Null, -32700, error.to_string()),
                )
                .await?;
                continue;
            }
        };
        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if id.is_none() {
            continue;
        }
        let id = id.unwrap_or(Value::Null);
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "osanwe", "version": env!("CARGO_PKG_VERSION")}
                }
            }),
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {"tools": tool_definitions_for(role)}
            }),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                let mapped = match serde_json::from_value::<McpToolCall>(params) {
                    Ok(call) => call.into_daemon_request(role, &agent_id),
                    Err(error) => Err(anyhow::Error::new(error)),
                };
                match mapped {
                    Ok(mut daemon_request) => {
                        daemon_request.token = token.clone();
                        match client.call(&daemon_request.method, daemon_request.params).await {
                            Ok(result) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{"type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())}],
                                    "structuredContent": result,
                                    "isError": false
                                }
                            }),
                            Err(error) => json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{"type": "text", "text": error.to_string()}],
                                    "isError": true
                                }
                            }),
                        }
                    }
                    Err(error) => json_rpc_error(id, -32602, error.to_string()),
                }
            }
            _ => json_rpc_error(id, -32601, format!("unknown method: {method}")),
        };
        write_message(&mut stdout, &response).await?;
    }
    Ok(())
}

async fn write_message<W>(writer: &mut W, value: &Value) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn json_rpc_error(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

fn tool(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema,
    }
}

fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "task_summary": {"type": "string"},
            "assumptions": {"type": "array", "items": {"type": "string"}},
            "questions": {"type": "array", "items": {"type": "string"}},
            "steps": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "title": {"type": "string"},
                        "objective": {"type": "string"},
                        "dependencies": {"type": "array", "items": {"type": "string"}},
                        "write_scope": {"type": "array", "items": {"type": "string"}},
                        "acceptance_criteria": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id", "title", "objective", "write_scope"],
                    "additionalProperties": false
                }
            },
            "checks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "program": {"type": "string"},
                        "args": {"type": "array", "items": {"type": "string"}},
                        "cwd": {"type": "string"},
                        "required": {"type": "boolean"},
                        "timeout_seconds": {"type": "integer", "minimum": 1}
                    },
                    "required": ["id", "program"],
                    "additionalProperties": false
                }
            },
            "completion_criteria": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["schema_version", "task_summary", "steps"],
        "additionalProperties": false
    })
}

fn worker_result_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "assignment_id": {"type": "string"},
            "status": {"type": "string", "enum": ["completed", "failed", "blocked"]},
            "summary": {"type": "string"},
            "changed_paths": {"type": "array", "items": {"type": "string"}},
            "deviations": {"type": "array", "items": {"type": "string"}},
            "unresolved_questions": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["assignment_id", "status", "summary"],
        "additionalProperties": false
    })
}

fn verification_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verdict": {"type": "string", "enum": ["pass", "needs_fix", "blocked"]},
            "summary": {"type": "string"},
            "acceptance_results": {"type": "array"},
            "findings": {"type": "array"},
            "required_fixes": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["verdict", "summary"],
        "additionalProperties": false
    })
}
