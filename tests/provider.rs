use std::path::Path;

use osanwe::model::{AgentRole, Provider};
use osanwe::provider::{LaunchContext, ProviderDriver};

#[test]
fn codex_planner_command_is_native_interactive() {
    let driver = ProviderDriver::for_provider(Provider::Codex);
    let command = driver
        .interactive_command(&LaunchContext::test(
            AgentRole::Planner,
            Path::new("/tmp/integration"),
        ))
        .expect("command");

    assert_eq!(command.program, "codex");
    assert!(command.args.iter().any(|arg| arg == "--cd"));
    assert!(command.args.iter().any(|arg| arg == "--profile"));
    assert!(command.args.iter().any(|arg| arg == "read-only"));
    assert!(!command.args.iter().any(|arg| arg == "exec"));
    assert!(!command.args.iter().any(|arg| arg == "--json"));
}

#[test]
fn grok_worker_command_is_native_interactive() {
    let driver = ProviderDriver::for_provider(Provider::Grok);
    let command = driver
        .interactive_command(&LaunchContext::test(
            AgentRole::Worker,
            Path::new("/tmp/worker"),
        ))
        .expect("command");

    assert_eq!(command.program, "grok");
    assert!(command.args.iter().any(|arg| arg == "--cwd"));
    assert!(command.args.iter().any(|arg| arg == "--session-id"));
    assert!(command.args.iter().any(|arg| arg == "--plugin-dir"));
    assert!(!command.args.iter().any(|arg| arg == "-p"));
    assert!(!command.args.iter().any(|arg| arg == "--single"));
}
