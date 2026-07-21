use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use osanwe::process::{CommandOutput, CommandRunner, CommandSpec};
use osanwe::zellij::{PaneHost, PaneSpec, ZellijPaneHost};

#[derive(Default)]
struct RecordingRunner {
    commands: Mutex<Vec<CommandSpec>>,
}

#[async_trait]
impl CommandRunner for RecordingRunner {
    async fn run(&self, spec: &CommandSpec) -> anyhow::Result<CommandOutput> {
        self.commands.lock().unwrap().push(spec.clone());
        Ok(CommandOutput::success("terminal_7\n"))
    }
}

#[tokio::test]
async fn creates_a_targetable_interactive_pane_without_shell_wrapping() {
    let runner = Arc::new(RecordingRunner::default());
    let host = ZellijPaneHost::new("osanwe-test", runner.clone());

    let pane_id = host
        .create_pane(PaneSpec {
            title: "[P] Codex Planner".into(),
            cwd: PathBuf::from("/repo"),
            command: CommandSpec::new("osanwe").args(["pane", "--agent-id", "planner"]),
        })
        .await
        .expect("pane creation");

    assert_eq!(pane_id.as_str(), "terminal_7");
    let commands = runner.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "zellij");
    assert!(commands[0].args.iter().any(|arg| arg == "new-pane"));
    assert!(commands[0].args.iter().any(|arg| arg == "--cwd"));
    assert!(commands[0].args.iter().any(|arg| arg == "osanwe"));
}

#[tokio::test]
async fn paste_targets_the_requested_pane() {
    let runner = Arc::new(RecordingRunner::default());
    let host = ZellijPaneHost::new("osanwe-test", runner.clone());

    host.paste("terminal_4", "hello\nworld")
        .await
        .expect("paste");

    let commands = runner.commands.lock().unwrap();
    assert!(commands[0]
        .args
        .windows(2)
        .any(|pair| pair == ["--pane-id", "terminal_4"]));
    assert!(commands[0].args.iter().any(|arg| arg == "paste"));
}
