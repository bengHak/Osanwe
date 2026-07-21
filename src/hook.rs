use std::env;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::ipc::IpcClient;

pub async fn forward_stdin() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    tokio::io::stdin().read_to_end(&mut bytes).await?;
    if bytes.is_empty() {
        return Ok(());
    }
    let event: Value = serde_json::from_slice(&bytes).context("decode hook event")?;
    let client = match IpcClient::from_environment() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("osanwe hook: {error}");
            return Ok(());
        }
    };
    let agent_id = env::var("OSANWE_AGENT_ID").unwrap_or_else(|_| "unknown".into());
    let grok_event = env::var("GROK_HOOK_EVENT").ok();
    let event_name = event
        .get("hook_event_name")
        .or_else(|| event.get("hookEventName"))
        .and_then(Value::as_str)
        .or(grok_event.as_deref())
        .unwrap_or("unknown")
        .to_owned();

    let _ = client
        .call(
            "hook.record",
            json!({
                "agent_id": agent_id.clone(),
                "event_name": event_name,
                "event": event
            }),
        )
        .await;

    let grok_session = env::var("GROK_SESSION_ID").ok();
    let session_id = event
        .get("session_id")
        .or_else(|| event.get("sessionId"))
        .and_then(Value::as_str)
        .or(grok_session.as_deref())
        .map(ToOwned::to_owned);
    if let Some(session_id) = session_id {
        let _ = client
            .call(
                "session.observe",
                json!({"agent_id": agent_id, "session_id": session_id}),
            )
            .await;
    }
    // Deliberately produce no stdout. Codex keeps its native approval prompt when
    // PermissionRequest is undecided; Grok passive hooks continue without changing input.
    Ok(())
}
