use serde_json::{json, Value};

use crate::model::{RunStatus, WorkflowEvent};
use crate::state::{Transition, Workflow};

use super::Daemon;

impl Daemon {
    pub(super) async fn apply_and_persist(
        &self,
        transition: Transition,
        source: &str,
        kind: &str,
        payload: Value,
    ) -> anyhow::Result<()> {
        let snapshot = {
            let mut run = self.run.lock().await;
            Workflow::apply(&mut run, transition)?;
            run.clone()
        };
        self.store.save(&snapshot)?;
        self.store.append_event(&WorkflowEvent::new(
            snapshot.run_id,
            source,
            kind,
            payload,
        ))?;
        Ok(())
    }

    pub(super) async fn append_event(
        &self,
        source: &str,
        kind: &str,
        payload: Value,
    ) -> anyhow::Result<()> {
        let run_id = self.run.lock().await.run_id.clone();
        self.store
            .append_event(&WorkflowEvent::new(run_id, source, kind, payload))
    }

    pub(super) async fn set_run_status(&self, status: RunStatus) -> anyhow::Result<()> {
        let snapshot = {
            let mut run = self.run.lock().await;
            run.status = status;
            run.touch();
            run.clone()
        };
        self.store.save(&snapshot)?;
        self.store.append_event(&WorkflowEvent::new(
            snapshot.run_id,
            "orchestrator",
            "run_status",
            json!({"status": status}),
        ))?;
        Ok(())
    }

    pub(super) async fn block_run(&self, message: String) -> anyhow::Result<()> {
        let snapshot = {
            let mut run = self.run.lock().await;
            run.status = RunStatus::Blocked;
            run.attention.push(message.clone());
            run.touch();
            run.clone()
        };
        self.store.save(&snapshot)?;
        self.store.append_event(&WorkflowEvent::new(
            snapshot.run_id,
            "orchestrator",
            "run_blocked",
            json!({"message": message}),
        ))?;
        Ok(())
    }
}
