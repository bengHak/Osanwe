use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde_json::json;
use uuid::Uuid;

use crate::model::{AgentRole, Provider};
use crate::process::CommandSpec;

#[derive(Clone, Debug)]
pub struct LaunchContext {
    pub run_id: String,
    pub agent_id: String,
    pub role: AgentRole,
    pub worktree: PathBuf,
    pub run_dir: PathBuf,
    pub socket_path: PathBuf,
    pub token: String,
    pub executable: PathBuf,
    pub codex_home: PathBuf,
    pub provider_session_id: Option<String>,
}

impl LaunchContext {
    #[must_use]
    pub fn test(role: AgentRole, worktree: &Path) -> Self {
        let run_dir = PathBuf::from("/tmp/osanwe-test-run");
        Self {
            run_id: "test-run".into(),
            agent_id: match role {
                AgentRole::Planner => "planner",
                AgentRole::Worker => "worker-1",
                AgentRole::Verifier => "verifier",
                AgentRole::Orchestrator => "orchestrator",
                AgentRole::Checks => "checks",
            }
            .into(),
            role,
            worktree: worktree.to_path_buf(),
            socket_path: run_dir.join("relayd.sock"),
            token: "test-token".into(),
            executable: PathBuf::from("/usr/local/bin/osanwe"),
            codex_home: PathBuf::from("/tmp/osanwe-codex-home"),
            run_dir,
            provider_session_id: None,
        }
    }

    #[must_use]
    pub fn from_environment(
        run_id: String,
        agent_id: String,
        role: AgentRole,
        worktree: PathBuf,
        run_dir: PathBuf,
        socket_path: PathBuf,
        token: String,
        executable: PathBuf,
    ) -> Self {
        Self {
            run_id,
            agent_id,
            role,
            worktree,
            run_dir,
            socket_path,
            token,
            executable,
            codex_home: codex_home(),
            provider_session_id: None,
        }
    }

    #[must_use]
    pub fn overlay_environment(&self) -> Vec<(String, String)> {
        vec![
            ("OSANWE_RUN_ID".into(), self.run_id.clone()),
            ("OSANWE_AGENT_ID".into(), self.agent_id.clone()),
            ("OSANWE_ROLE".into(), role_name(self.role).into()),
            (
                "OSANWE_SOCKET".into(),
                self.socket_path.to_string_lossy().into_owned(),
            ),
            ("OSANWE_AGENT_TOKEN".into(), self.token.clone()),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct ProviderOverlay {
    pub root: PathBuf,
    pub profile_name: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ProviderDriver {
    Codex,
    Grok,
}

impl ProviderDriver {
    #[must_use]
    pub const fn for_provider(provider: Provider) -> Self {
        match provider {
            Provider::Codex => Self::Codex,
            Provider::Grok => Self::Grok,
        }
    }

    pub fn prepare_overlay(&self, ctx: &LaunchContext) -> anyhow::Result<ProviderOverlay> {
        match self {
            Self::Codex => prepare_codex_overlay(ctx),
            Self::Grok => prepare_grok_overlay(ctx),
        }
    }

    pub fn interactive_command(&self, ctx: &LaunchContext) -> anyhow::Result<CommandSpec> {
        match self {
            Self::Codex => codex_command(ctx, false),
            Self::Grok => grok_command(ctx, false),
        }
    }

    pub fn resume_command(&self, ctx: &LaunchContext) -> anyhow::Result<CommandSpec> {
        if ctx
            .provider_session_id
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            bail!("provider session ID is required to resume an agent")
        }
        match self {
            Self::Codex => codex_command(ctx, true),
            Self::Grok => grok_command(ctx, true),
        }
    }

    #[must_use]
    pub fn bootstrap_prompt(&self, role: AgentRole) -> String {
        match role {
            AgentRole::Planner => concat!(
                "You are the planner for this Osanwe run. ",
                "Call osanwe.get_assignment to obtain the task and constraints. ",
                "Inspect the repository with your normal Codex tools, MCP servers, skills, and plugins. ",
                "Do not implement the task. When the plan is complete, call osanwe.submit_plan."
            )
            .into(),
            AgentRole::Worker => concat!(
                "You are a worker for this Osanwe run. ",
                "Call osanwe.get_assignment to obtain your current assignment. ",
                "Use your normal Grok Build tools, MCP servers, skills, plugins, subagents, and memory. ",
                "Work only in the assigned worktree and scope. ",
                "When finished, call osanwe.complete_assignment; when blocked, call osanwe.request_input."
            )
            .into(),
            AgentRole::Verifier => concat!(
                "You are the independent verifier for this Osanwe run. ",
                "Call osanwe.get_assignment to obtain the original task, approved plan, integrated diff, ",
                "deterministic checks, and worker report. Inspect the repository independently and do not modify files. ",
                "When complete, call osanwe.submit_verification."
            )
            .into(),
            AgentRole::Orchestrator | AgentRole::Checks => String::new(),
        }
    }
}

fn codex_command(ctx: &LaunchContext, resume: bool) -> anyhow::Result<CommandSpec> {
    let profile = codex_profile_name(ctx);
    let sandbox = match ctx.role {
        AgentRole::Planner | AgentRole::Verifier => "read-only",
        AgentRole::Worker => "workspace-write",
        AgentRole::Orchestrator | AgentRole::Checks => {
            bail!("Codex may only be launched for planner, worker, or verifier roles")
        }
    };
    let mut command = CommandSpec::new("codex").args(vec![
        "--cd".to_owned(),
        path_text(&ctx.worktree)?,
        "--profile".to_owned(),
        profile,
        "--sandbox".to_owned(),
        sandbox.to_owned(),
    ]);
    if resume {
        command = command.args(vec![
            "resume".to_owned(),
            ctx.provider_session_id
                .clone()
                .context("missing Codex session ID")?,
        ]);
    }
    for (key, value) in ctx.overlay_environment() {
        command = command.env(key, value);
    }
    Ok(command.cwd(ctx.worktree.clone()))
}

fn grok_command(ctx: &LaunchContext, resume: bool) -> anyhow::Result<CommandSpec> {
    if ctx.role != AgentRole::Worker {
        bail!("Grok may only be launched for worker roles")
    }
    let plugin_dir = ctx
        .run_dir
        .join("provider-overlays")
        .join(format!("grok-{}", ctx.agent_id));
    let mut command = CommandSpec::new("grok").args(vec![
        "--cwd".to_owned(),
        path_text(&ctx.worktree)?,
        "--plugin-dir".to_owned(),
        path_text(&plugin_dir)?,
    ]);
    if resume {
        command = command.args(vec![
            "--resume".to_owned(),
            ctx.provider_session_id
                .clone()
                .context("missing Grok session ID")?,
        ]);
    } else {
        let session_id = ctx
            .provider_session_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        command = command.args(vec!["--session-id".to_owned(), session_id]);
    }
    for (key, value) in ctx.overlay_environment() {
        command = command.env(key, value);
    }
    Ok(command.cwd(ctx.worktree.clone()))
}

fn prepare_codex_overlay(ctx: &LaunchContext) -> anyhow::Result<ProviderOverlay> {
    fs::create_dir_all(&ctx.codex_home)
        .with_context(|| format!("create {}", ctx.codex_home.display()))?;
    let profile_name = codex_profile_name(ctx);
    let profile_path = ctx.codex_home.join(format!("{profile_name}.config.toml"));
    let executable = path_text(&ctx.executable)?;
    let hook_command = format!("{} hook", shell_quote(&executable));

    let mut document = toml::map::Map::new();
    document.insert(
        "features".into(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "hooks".into(),
            toml::Value::Boolean(true),
        )])),
    );

    let mut server = toml::map::Map::new();
    server.insert("command".into(), toml::Value::String(executable));
    server.insert(
        "args".into(),
        toml::Value::Array(vec![toml::Value::String("bridge".into())]),
    );
    server.insert("startup_timeout_sec".into(), toml::Value::Integer(10));
    server.insert("tool_timeout_sec".into(), toml::Value::Integer(60));
    server.insert(
        "env".into(),
        toml::Value::Table(toml::map::Map::from_iter(
            ctx.overlay_environment()
                .into_iter()
                .map(|(key, value)| (key, toml::Value::String(value))),
        )),
    );
    document.insert(
        "mcp_servers".into(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "osanwe".into(),
            toml::Value::Table(server),
        )])),
    );

    let mut hooks = toml::map::Map::new();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "PreCompact",
        "PostCompact",
    ] {
        hooks.insert(
            event.into(),
            toml::Value::Array(vec![toml::Value::Table(toml::map::Map::from_iter([(
                "hooks".into(),
                toml::Value::Array(vec![toml::Value::Table(toml::map::Map::from_iter([
                    ("type".into(), toml::Value::String("command".into())),
                    ("command".into(), toml::Value::String(hook_command.clone())),
                    ("timeout".into(), toml::Value::Integer(5)),
                ]))]),
            )]))]),
        );
    }
    document.insert("hooks".into(), toml::Value::Table(hooks));

    fs::write(&profile_path, toml::to_string_pretty(&document)?)
        .with_context(|| format!("write {}", profile_path.display()))?;
    Ok(ProviderOverlay {
        root: profile_path,
        profile_name: Some(profile_name),
    })
}

fn prepare_grok_overlay(ctx: &LaunchContext) -> anyhow::Result<ProviderOverlay> {
    let root = ctx
        .run_dir
        .join("provider-overlays")
        .join(format!("grok-{}", ctx.agent_id));
    fs::create_dir_all(root.join(".claude-plugin"))?;
    fs::create_dir_all(root.join("hooks"))?;

    let manifest = json!({
        "name": format!("osanwe-{}", ctx.agent_id),
        "description": "Osanwe orchestration bridge",
        "version": "0.1.0"
    });
    write_json(root.join(".claude-plugin/plugin.json"), &manifest)?;

    let executable = path_text(&ctx.executable)?;
    let mut env_object = serde_json::Map::new();
    for (key, value) in ctx.overlay_environment() {
        env_object.insert(key, serde_json::Value::String(value));
    }
    let mcp = json!({
        "mcpServers": {
            "osanwe": {
                "command": executable,
                "args": ["bridge"],
                "env": env_object
            }
        }
    });
    write_json(root.join(".mcp.json"), &mcp)?;

    let command = format!("{} hook", shell_quote(&path_text(&ctx.executable)?));
    let mut event_map = serde_json::Map::new();
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionDenied",
        "Stop",
        "StopFailure",
        "SubagentStart",
        "SubagentStop",
        "PreCompact",
        "PostCompact",
    ] {
        event_map.insert(
            event.into(),
            json!([{"hooks": [{"type": "command", "command": command, "timeout": 5}]}]),
        );
    }
    write_json(root.join("hooks/hooks.json"), &json!({"hooks": event_map}))?;

    Ok(ProviderOverlay {
        root,
        profile_name: None,
    })
}

fn write_json(path: PathBuf, value: &serde_json::Value) -> anyhow::Result<()> {
    fs::write(&path, serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("write {}", path.display()))
}

fn codex_profile_name(ctx: &LaunchContext) -> String {
    format!("osanwe-{}-{}", short_id(&ctx.run_id), ctx.agent_id)
}

fn short_id(value: &str) -> &str {
    &value[..value.len().min(12)]
}

fn codex_home() -> PathBuf {
    env::var_os("CODEX_HOME").map_or_else(
        || {
            env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        },
        PathBuf::from,
    )
}

fn path_text(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[must_use]
pub const fn role_name(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "orchestrator",
        AgentRole::Planner => "planner",
        AgentRole::Worker => "worker",
        AgentRole::Verifier => "verifier",
        AgentRole::Checks => "checks",
    }
}

pub fn parse_role(value: &str) -> anyhow::Result<AgentRole> {
    match value {
        "orchestrator" => Ok(AgentRole::Orchestrator),
        "planner" => Ok(AgentRole::Planner),
        "worker" => Ok(AgentRole::Worker),
        "verifier" => Ok(AgentRole::Verifier),
        "checks" => Ok(AgentRole::Checks),
        other => bail!("unknown agent role: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_prompts_require_structured_completion_tools() {
        let planner = ProviderDriver::Codex.bootstrap_prompt(AgentRole::Planner);
        let worker = ProviderDriver::Grok.bootstrap_prompt(AgentRole::Worker);
        assert!(planner.contains("osanwe.submit_plan"));
        assert!(worker.contains("osanwe.complete_assignment"));
    }

    #[test]
    fn shell_quoting_handles_apostrophes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }
}
