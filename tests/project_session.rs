//! Integration tests for project `.osanwe` scaffold, config, and launch specs.

use std::path::Path;
use std::process::Command;

use osanwe::onboard::{apply_choices, apply_defaults};
use osanwe::project::{
    config_exists, config_path, load_config, osanwe_dir, scaffold, scaffold_with_config,
    ClientKind, ProjectConfig, RoleChoice, SCAFFOLD_DIRS,
};
use osanwe::session_launch::build_role_launch_specs;
use tempfile::tempdir;

#[test]
fn defaults_scaffold_creates_full_file_bus() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    apply_defaults(root).unwrap();

    assert!(config_exists(root));
    for name in SCAFFOLD_DIRS {
        assert!(
            osanwe_dir(root).join(name).is_dir(),
            "missing scaffold dir {name}"
        );
    }
    let config = load_config(root).unwrap();
    assert_eq!(config.schema_version, "1");
    assert!(!config.zellij_session.is_empty());
    assert!(config.zellij_session.starts_with("osanwe-"));
}

#[test]
fn custom_role_clients_drive_launch_commands() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let mut config = ProjectConfig::defaults_for_repo(root);
    config.roles.orchestrator = RoleChoice::new(ClientKind::Grok, "grok-fast");
    config.roles.planner = RoleChoice::new(ClientKind::Codex, "o3");
    config.roles.worker = RoleChoice::new(ClientKind::Codex, "gpt-5.6-sol");
    config.enable_verifier = true;
    config.roles.verifier = Some(RoleChoice::new(ClientKind::Grok, "grok-4.5"));
    scaffold_with_config(root, &config).unwrap();

    let specs = build_role_launch_specs(root, &config).unwrap();
    assert_eq!(specs.len(), 4);

    let by_role = |role: &str| specs.iter().find(|s| s.role == role).unwrap();

    let orch = by_role("orchestrator");
    assert_eq!(orch.command.program, "grok");
    assert!(orch
        .command
        .args
        .windows(2)
        .any(|w| w == ["-m", "grok-fast"]));
    assert!(
        orch.command.args.windows(2).any(|w| w[0] == "--session-id"),
        "first Grok launch should use --session-id: {:?}",
        orch.command.args
    );
    assert_eq!(
        orch.command.env.get("OSANWE_DIR").map(String::as_str),
        Some(osanwe_dir(root).to_str().unwrap())
    );
    // Env is on CommandSpec; Zellij create_pane wraps it via `env KEY=VAL`.
    let (pane_prog, pane_args) = osanwe::zellij::pane_program_and_args(&orch.command);
    assert_eq!(pane_prog, "env");
    assert!(pane_args.iter().any(|a| a.starts_with("OSANWE_DIR=")));
    assert!(pane_args.iter().any(|a| a == "grok"));
    assert!(orch
        .command
        .args
        .iter()
        .any(|arg| arg.contains("Osanwe role: orchestrator")));

    let planner = by_role("planner");
    assert_eq!(planner.command.program, "codex");
    assert!(planner.command.args.iter().any(|a| a == "-C"));
    assert!(planner.command.args.windows(2).any(|w| w == ["-m", "o3"]));
    assert_eq!(
        planner.command.env.get("OSANWE_ROLE").map(String::as_str),
        Some("planner")
    );
    assert_eq!(
        planner.command.env.get("OSANWE_CLIENT").map(String::as_str),
        Some("codex")
    );

    let worker = by_role("worker");
    assert_eq!(worker.command.program, "codex");
    assert_eq!(worker.command.cwd.as_deref(), Some(root));
    assert!(worker.command.args.iter().any(|a| a == "workspace-write"));

    let verifier = by_role("verifier");
    assert_eq!(verifier.command.program, "grok");
    assert!(verifier
        .command
        .args
        .windows(2)
        .any(|w| w == ["-m", "grok-4.5"]));
}

#[test]
fn apply_choices_roundtrip_via_disk() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let mut config = ProjectConfig::defaults_for_repo(root);
    config.roles.worker.model = "custom-worker-model".into();
    apply_choices(root, &config).unwrap();

    // Re-scaffold must not wipe config.
    scaffold(root).unwrap();
    let loaded = load_config(root).unwrap();
    assert_eq!(loaded.roles.worker.model, "custom-worker-model");
    assert!(config_path(root).is_file());
}

#[test]
fn binary_help_surfaces_new_primary_commands() {
    let exe = env!("CARGO_BIN_EXE_osanwe");
    let output = Command::new(exe)
        .arg("--help")
        .output()
        .expect("run osanwe --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("onboard"),
        "help should list onboard: {stdout}"
    );
    assert!(
        stdout.contains("attach"),
        "help should list attach: {stdout}"
    );
    assert!(
        stdout.contains("doctor"),
        "help should list doctor: {stdout}"
    );
    assert!(stdout.contains("stop"), "help should list stop: {stdout}");
    // Primary product path is bare invocation / onboard — not only start "<task>".
    assert!(
        stdout.to_lowercase().contains("file bus")
            || stdout.to_lowercase().contains(".osanwe")
            || stdout.to_lowercase().contains("onboard"),
        "help should describe file-bus / onboard product: {stdout}"
    );
}

#[test]
fn binary_onboard_defaults_writes_project_config() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let exe = env!("CARGO_BIN_EXE_osanwe");
    let output = Command::new(exe)
        .args([
            "onboard",
            "--defaults",
            "--repo",
            root.to_str().unwrap(),
            "--force",
        ])
        .output()
        .expect("run osanwe onboard --defaults");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(config_exists(root));
    let config = load_config(root).unwrap();
    assert_eq!(config.roles.orchestrator.client, ClientKind::Codex);
    assert_eq!(config.roles.worker.client, ClientKind::Grok);
    assert!(Path::new(&osanwe_dir(root))
        .join("prompts/worker.md")
        .is_file());
}

#[test]
fn onboard_defaults_requires_force_to_replace_existing_config() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let mut config = ProjectConfig::defaults_for_repo(root);
    config.roles.worker.model = "keep-me".into();
    scaffold_with_config(root, &config).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_osanwe"))
        .args(["onboard", "--defaults", "--repo", root.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(load_config(root).unwrap().roles.worker.model, "keep-me");
}
