use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use osanwe::process::{CommandOutput, CommandRunner, CommandSpec};
use osanwe::zellij::{pane_program_and_args, PaneSpec, ZellijPaneHost};

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

#[test]
fn pane_program_and_args_forwards_env_via_env_wrapper() {
    let mut spec = CommandSpec::new("grok").args(["--cwd", "/repo", "-m", "grok-4.5"]);
    spec = spec
        .env("OSANWE_DIR", "/repo/.osanwe")
        .env("OSANWE_ROLE", "worker")
        .env("OSANWE_CLIENT", "grok")
        .env("OSANWE_MODEL", "grok-4.5")
        .env("OSANWE_PROJECT", "/repo");
    let (program, args) = pane_program_and_args(&spec);
    assert_eq!(program, "env");
    assert!(args.iter().any(|a| a == "OSANWE_DIR=/repo/.osanwe"));
    assert!(args.iter().any(|a| a == "OSANWE_ROLE=worker"));
    assert!(args.iter().any(|a| a == "OSANWE_CLIENT=grok"));
    assert!(args.iter().any(|a| a == "OSANWE_MODEL=grok-4.5"));
    assert!(args.iter().any(|a| a == "OSANWE_PROJECT=/repo"));
    // program and original args follow KEY=VAL pairs
    let grok_at = args.iter().position(|a| a == "grok").expect("grok");
    assert!(grok_at > 0);
    assert_eq!(&args[grok_at + 1..], &["--cwd", "/repo", "-m", "grok-4.5"]);
}

#[test]
fn pane_program_and_args_without_env_is_passthrough() {
    let spec = CommandSpec::new("codex").args(["-C", "/repo"]);
    let (program, args) = pane_program_and_args(&spec);
    assert_eq!(program, "codex");
    assert_eq!(args, vec!["-C", "/repo"]);
}

#[tokio::test]
async fn create_pane_wraps_env_for_live_launch_path() {
    let runner = Arc::new(RecordingRunner::default());
    let host = ZellijPaneHost::new("osanwe-test", runner.clone());

    let command = CommandSpec::new("grok")
        .args(["--cwd", "/proj"])
        .env("OSANWE_DIR", "/proj/.osanwe")
        .env("OSANWE_ROLE", "orchestrator");

    host.create_pane(PaneSpec {
        title: "[O] grok".into(),
        cwd: PathBuf::from("/proj"),
        command,
    })
    .await
    .expect("pane");

    let commands = runner.commands.lock().unwrap();
    let args = &commands[0].args;
    // After `--` we should see `env KEY=… grok …`
    let dash = args.iter().position(|a| a == "--").expect("--");
    assert_eq!(args.get(dash + 1).map(String::as_str), Some("env"));
    let rest = &args[dash + 2..];
    assert!(rest.iter().any(|a| a == "OSANWE_DIR=/proj/.osanwe"));
    assert!(rest.iter().any(|a| a == "OSANWE_ROLE=orchestrator"));
    assert!(rest.iter().any(|a| a == "grok"));
}
