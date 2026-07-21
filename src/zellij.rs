use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::process::{CommandRunner, CommandSpec};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaneInfo {
    pub id: u64,
    #[serde(default)]
    pub is_plugin: bool,
    #[serde(default)]
    pub is_focused: bool,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub exited: bool,
    pub exit_status: Option<i32>,
    #[serde(default)]
    pub pane_command: String,
    #[serde(default)]
    pub pane_cwd: String,
    #[serde(default)]
    pub pane_rows: u16,
    #[serde(default)]
    pub pane_columns: u16,
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

#[async_trait]
pub trait PaneHost: Send + Sync {
    async fn create_session(&self) -> anyhow::Result<()>;
    async fn create_pane(&self, spec: PaneSpec) -> anyhow::Result<PaneId>;
    async fn paste(&self, pane_id: &str, text: &str) -> anyhow::Result<()>;
    async fn send_keys(&self, pane_id: &str, keys: &[&str]) -> anyhow::Result<()>;
    async fn focus(&self, pane_id: &str) -> anyhow::Result<()>;
    async fn close(&self, pane_id: &str) -> anyhow::Result<()>;
    async fn rename(&self, pane_id: &str, title: &str) -> anyhow::Result<()>;
    async fn list_panes(&self) -> anyhow::Result<Vec<PaneInfo>>;
    async fn snapshot(&self, pane_id: &str) -> anyhow::Result<String>;
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

    #[must_use]
    pub fn session(&self) -> &str {
        &self.session
    }

    fn action(&self, action: &str) -> CommandSpec {
        CommandSpec::new("zellij").args([
            "--session".into(),
            self.session.clone(),
            "action".into(),
            action.into(),
        ])
    }
}

#[async_trait]
impl PaneHost for ZellijPaneHost {
    async fn create_session(&self) -> anyhow::Result<()> {
        let output = self
            .runner
            .run(&CommandSpec::new("zellij").args([
                "attach".into(),
                "--create-background".into(),
                self.session.clone(),
                "options".into(),
                "--default-layout".into(),
                "compact".into(),
            ]))
            .await?;
        output.require_success("create Zellij session")?;
        Ok(())
    }

    async fn create_pane(&self, spec: PaneSpec) -> anyhow::Result<PaneId> {
        let cwd = spec
            .cwd
            .to_str()
            .context("pane cwd is not valid UTF-8")?
            .to_owned();
        let mut command = self.action("new-pane").args([
            "--name".into(),
            spec.title,
            "--cwd".into(),
            cwd,
            "--".into(),
            spec.command.program,
        ]);
        command.args.extend(spec.command.args);
        let output = self.runner.run(&command).await?;
        let output = output.require_success("create Zellij pane")?;
        let pane_id = output.stdout.trim();
        if pane_id.is_empty() {
            bail!("Zellij did not return a pane ID; version 0.44 or newer is required");
        }
        Ok(PaneId::new(pane_id))
    }

    async fn paste(&self, pane_id: &str, text: &str) -> anyhow::Result<()> {
        self.runner
            .run(&self.action("paste").args([
                "--pane-id".into(),
                pane_id.into(),
                text.into(),
            ]))
            .await?
            .require_success("paste into pane")?;
        Ok(())
    }

    async fn send_keys(&self, pane_id: &str, keys: &[&str]) -> anyhow::Result<()> {
        let mut command = self.action("send-keys").args([
            "--pane-id".into(),
            pane_id.into(),
        ]);
        command.args.extend(keys.iter().map(|key| (*key).to_owned()));
        self.runner
            .run(&command)
            .await?
            .require_success("send keys to pane")?;
        Ok(())
    }

    async fn focus(&self, pane_id: &str) -> anyhow::Result<()> {
        self.runner
            .run(&self.action("focus-pane-id").arg(pane_id))
            .await?
            .require_success("focus pane")?;
        Ok(())
    }

    async fn close(&self, pane_id: &str) -> anyhow::Result<()> {
        self.runner
            .run(
                &self
                    .action("close-pane")
                    .args(["--pane-id".into(), pane_id.into()]),
            )
            .await?
            .require_success("close pane")?;
        Ok(())
    }

    async fn rename(&self, pane_id: &str, title: &str) -> anyhow::Result<()> {
        self.runner
            .run(&self.action("rename-pane").args([
                "--pane-id".into(),
                pane_id.into(),
                title.into(),
            ]))
            .await?
            .require_success("rename pane")?;
        Ok(())
    }

    async fn list_panes(&self) -> anyhow::Result<Vec<PaneInfo>> {
        let output = self
            .runner
            .run(&self.action("list-panes").arg("--json"))
            .await?
            .require_success("list Zellij panes")?;
        serde_json::from_str(&output.stdout).context("decode Zellij pane list")
    }

    async fn snapshot(&self, pane_id: &str) -> anyhow::Result<String> {
        let output = self
            .runner
            .run(&self.action("dump-screen").args([
                "--pane-id".into(),
                pane_id.into(),
                "--full".into(),
            ]))
            .await?
            .require_success("dump pane screen")?;
        Ok(output.stdout)
    }
}
