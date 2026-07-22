//! Opt-in smoke test for the installed Zellij binary.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use osanwe::process::{CommandSpec, TokioCommandRunner};
use osanwe::zellij::{PaneSpec, ZellijPaneHost};

struct SessionGuard(String);

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("zellij")
            .args(["delete-session", "--force", &self.0])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[tokio::test]
#[ignore = "requires Zellij 0.44+ on PATH"]
async fn creates_and_lists_a_live_pane() {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let session = format!("osanwe-{}", &id[..8]);
    let _guard = SessionGuard(session.clone());
    let host = ZellijPaneHost::new(session, Arc::new(TokioCommandRunner));
    host.create_session().await.unwrap();

    let pane = host
        .create_pane(PaneSpec {
            title: "Osanwe smoke".into(),
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            command: CommandSpec::new("sh").args(["-c", "sleep 5"]),
        })
        .await
        .unwrap();

    assert!(host
        .list_panes()
        .await
        .unwrap()
        .iter()
        .any(|candidate| candidate.pane_id() == pane));
}
