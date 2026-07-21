mod handlers_core;
mod handlers_events;
mod handlers_results;
mod persistence;
mod prompting;
mod spawning;
mod support;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::ipc::{self, RpcHandler, RpcRequest, RpcResponse};
use crate::model::{AgentRole, RunManifest};
use crate::process::TokioCommandRunner;
use crate::store::RunStore;
use crate::workspace::WorkspaceManager;
use crate::zellij::{PaneHost, ZellijPaneHost};

pub use support::{role_label, role_provider};

#[derive(Clone)]
pub struct Daemon {
    pub(super) store: RunStore,
    pub(super) run: Arc<Mutex<RunManifest>>,
    pub(super) pane_host: Arc<dyn PaneHost>,
    pub(super) workspace: WorkspaceManager,
    pub(super) executable: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) enum Identity {
    Admin,
    Agent { id: String, role: AgentRole },
}

impl Daemon {
    pub async fn load(store: RunStore, run_id: &str, executable: PathBuf) -> anyhow::Result<Self> {
        let run = store.load(run_id)?;
        let runner = Arc::new(TokioCommandRunner);
        let pane_host = Arc::new(ZellijPaneHost::new(
            run.zellij_session.clone(),
            runner.clone(),
        ));
        Ok(Self {
            store,
            run: Arc::new(Mutex::new(run)),
            pane_host,
            workspace: WorkspaceManager::new(runner),
            executable,
        })
    }

    #[must_use]
    pub fn with_components(
        store: RunStore,
        run: RunManifest,
        pane_host: Arc<dyn PaneHost>,
        workspace: WorkspaceManager,
        executable: PathBuf,
    ) -> Self {
        Self {
            store,
            run: Arc::new(Mutex::new(run)),
            pane_host,
            workspace,
            executable,
        }
    }

    pub async fn serve(self) -> anyhow::Result<()> {
        let socket_path = {
            let run = self.run.lock().await;
            self.store.socket_path(&run.run_id)
        };
        ipc::serve(&socket_path, Arc::new(self)).await
    }
}

#[async_trait]
impl RpcHandler for Daemon {
    async fn handle(&self, request: RpcRequest) -> RpcResponse {
        let id = request.id.clone();
        match self.dispatch(&request).await {
            Ok(result) => RpcResponse::success(id, result),
            Err(error) => RpcResponse::failure(id, -32_000, error.to_string()),
        }
    }
}

pub async fn run_daemon(store: RunStore, run_id: &str) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("resolve Osanwe executable")?;
    Daemon::load(store, run_id, executable).await?.serve().await
}
