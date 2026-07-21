use std::process::Stdio;

use anyhow::Context;
use serde_json::json;
use tokio::process::Command;

use crate::ipc::IpcClient;
use crate::provider::{LaunchContext, ProviderDriver};
use crate::store::RunStore;

pub async fn run_agent_pane(
    store: RunStore,
    run_id: &str,
    agent_id: &str,
    resume: bool,
) -> anyhow::Result<()> {
    let run = store.load(run_id)?;
    let agent = run
        .agents
        .get(agent_id)
        .cloned()
        .with_context(|| format!("unknown agent: {agent_id}"))?;
    let provider = agent.provider.context("agent does not have an LLM provider")?;
    let executable = std::env::current_exe().context("resolve Osanwe executable")?;
    let context = LaunchContext::from_environment(
        run.run_id.clone(),
        agent.id.clone(),
        agent.role,
        agent.worktree.clone(),
        run.run_dir.clone(),
        store.socket_path(run_id),
        agent.token.clone(),
        executable,
    );
    let context = LaunchContext {
        provider_session_id: agent.provider_session_id.clone(),
        ..context
    };
    let driver = ProviderDriver::for_provider(provider);
    driver.prepare_overlay(&context)?;
    let spec = if resume {
        driver.resume_command(&context)?
    } else {
        driver.interactive_command(&context)?
    };
    let client = IpcClient::new(store.socket_path(run_id), agent.token);
    let _ = client
        .call(
            "pane.started",
            json!({"agent_id": agent_id, "provider": provider}),
        )
        .await;

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .envs(&spec.env)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    let status = command
        .status()
        .await
        .with_context(|| format!("start interactive {} session", spec.program))?;
    let _ = client
        .call(
            "pane.exited",
            json!({
                "agent_id": agent_id,
                "success": status.success(),
                "exit_code": status.code()
            }),
        )
        .await;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("{} exited with status {:?}", spec.program, status.code())
    }
}

pub async fn run_check_pane(store: RunStore, run_id: &str) -> anyhow::Result<()> {
    let run = store.load(run_id)?;
    let checks_agent = run.agents.get("checks").context("checks agent is missing")?;
    let checks = run
        .plan
        .as_ref()
        .map(|plan| plan.checks.clone())
        .unwrap_or_default();
    let results = crate::checks::run_checks(&run.integration_worktree, &checks).await;
    let required_failed = results
        .iter()
        .any(|result| result.required && !result.passed);
    let client = IpcClient::new(store.socket_path(run_id), checks_agent.token.clone());
    client
        .call(
            "checks.complete",
            json!({"agent_id": "checks", "results": results}),
        )
        .await?;
    if required_failed {
        anyhow::bail!("one or more required checks failed")
    }
    Ok(())
}
