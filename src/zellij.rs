use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::process::{CommandRunner, CommandSpec};

/// Expand a [`CommandSpec`] into the program argv Zellij should exec after `--`.
/// When `env` is non-empty, wrap as `env KEY=VAL… program args…` so variables reach the pane.
#[must_use]
pub fn pane_program_and_args(spec: &CommandSpec) -> (String, Vec<String>) {
    if spec.env.is_empty() {
        return (spec.program.clone(), spec.args.clone());
    }
    let mut args = Vec::with_capacity(spec.env.len() + 1 + spec.args.len());
    for (key, value) in &spec.env {
        args.push(format!("{key}={value}"));
    }
    args.push(spec.program.clone());
    args.extend(spec.args.iter().cloned());
    ("env".into(), args)
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PaneId(String);

impl PaneId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct PaneSpec {
    pub title: String,
    pub cwd: PathBuf,
    pub command: CommandSpec,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaneInfo {
    pub id: u64,
    #[serde(default)]
    pub is_plugin: bool,
}

impl PaneInfo {
    #[must_use]
    pub fn pane_id(&self) -> PaneId {
        if self.is_plugin {
            PaneId::new(format!("plugin_{}", self.id))
        } else {
            PaneId::new(format!("terminal_{}", self.id))
        }
    }
}

pub struct ZellijPaneHost {
    session: String,
    runner: Arc<dyn CommandRunner>,
}

impl ZellijPaneHost {
    #[must_use]
    pub fn new(session: impl Into<String>, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            session: session.into(),
            runner,
        }
    }

    fn action(&self, action: &str) -> CommandSpec {
        CommandSpec::new("zellij").args([
            "--session".into(),
            self.session.clone(),
            "action".into(),
            action.into(),
        ])
    }

    pub async fn create_session(&self) -> anyhow::Result<()> {
        self.runner
            .run(&CommandSpec::new("zellij").args([
                "attach".into(),
                "--create-background".into(),
                self.session.clone(),
                "options".into(),
                "--default-layout".into(),
                "compact".into(),
            ]))
            .await?
            .require_success("create Zellij session")?;
        Ok(())
    }

    pub async fn create_pane(&self, spec: PaneSpec) -> anyhow::Result<PaneId> {
        let cwd = spec
            .cwd
            .to_str()
            .context("pane cwd is not valid UTF-8")?
            .to_owned();
        let (program, mut program_args) = pane_program_and_args(&spec.command);
        let mut command = self.action("new-pane").args([
            "--name".into(),
            spec.title,
            "--cwd".into(),
            cwd,
            "--".into(),
            program,
        ]);
        command.args.append(&mut program_args);
        let output = self
            .runner
            .run(&command)
            .await?
            .require_success("create Zellij pane")?;
        let pane_id = output.stdout.trim();
        if pane_id.is_empty() {
            bail!("Zellij did not return a pane ID; version 0.44 or newer is required");
        }
        Ok(PaneId::new(pane_id))
    }

    pub async fn close(&self, pane_id: &str) -> anyhow::Result<()> {
        self.runner
            .run(&self.action("close-pane").args(["--pane-id", pane_id]))
            .await?
            .require_success("close pane")?;
        Ok(())
    }

    pub async fn list_panes(&self) -> anyhow::Result<Vec<PaneInfo>> {
        let output = self
            .runner
            .run(&self.action("list-panes").arg("--json"))
            .await?
            .require_success("list Zellij panes")?;
        serde_json::from_str(&output.stdout).context("decode Zellij pane list")
    }
}
