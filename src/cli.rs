use std::path::PathBuf;

use anyhow::bail;
use clap::{Parser, Subcommand};
use tokio::process::Command;

use crate::onboard;
use crate::project::{self, config_exists};
use crate::session_launch;

#[derive(Debug, Parser)]
#[command(
    name = "osanwe",
    version,
    about = "Launch Codex/Grok sessions with a project-local .osanwe file bus"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run first-time (or forced) onboarding: pick client + model per role.
    Onboard {
        /// Git repository / project root.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Write default role choices without opening the TUI.
        #[arg(long)]
        defaults: bool,
        /// Overwrite existing `.osanwe/config.toml`.
        #[arg(long)]
        force: bool,
        /// After onboarding, start the Zellij session.
        #[arg(long)]
        launch: bool,
        /// Create the session without attaching this terminal.
        #[arg(long)]
        no_attach: bool,
    },
    /// Attach to the project's Zellij session.
    Attach {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Stop the project's Zellij session.
    Stop {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Check required executables and print discovered versions.
    Doctor,
    /// Print the project board (used as a Zellij pane).
    #[command(hide = true)]
    Board {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => default_entry(PathBuf::from(".")).await,
        Some(Commands::Onboard {
            repo,
            defaults,
            force,
            launch,
            no_attach,
        }) => onboard_command(repo, defaults, force, launch, no_attach).await,
        Some(Commands::Attach { repo }) => {
            let root = project::find_project_root(&repo)?;
            session_launch::attach_project_session(&root).await
        }
        Some(Commands::Stop { repo }) => {
            let root = project::find_project_root(&repo)?;
            session_launch::stop_project_session(&root).await
        }
        Some(Commands::Doctor) => doctor().await,
        Some(Commands::Board { repo }) => {
            let root = project::find_project_root(&repo)?;
            session_launch::run_board(&root)
        }
    }
}

async fn default_entry(repo: PathBuf) -> anyhow::Result<()> {
    require_unix()?;
    let root = project::find_project_root(&repo)?;
    if !config_exists(&root) {
        println!("No `.osanwe/config.toml` found — starting onboarding…");
        let config = onboard::run_onboarding(&root)?;
        println!(
            "Onboarding complete (session {}). Launching…",
            config.zellij_session
        );
    } else {
        project::scaffold(&root)?;
    }
    session_launch::launch_project_session(&root, false).await
}

async fn onboard_command(
    repo: PathBuf,
    defaults: bool,
    force: bool,
    launch: bool,
    no_attach: bool,
) -> anyhow::Result<()> {
    let root = project::find_project_root(&repo)?;
    if config_exists(&root) && !force {
        bail!(
            "`.osanwe/config.toml` already exists; pass --force to re-onboard or run bare `osanwe` to launch"
        );
    }
    let config = if defaults {
        onboard::apply_defaults(&root)?
    } else {
        onboard::run_onboarding(&root)?
    };
    println!(
        "Wrote {} (session {})",
        project::config_path(&root).display(),
        config.zellij_session
    );
    if launch {
        require_unix()?;
        session_launch::launch_project_session(&root, no_attach).await?;
    }
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {
    println!("Osanwe checks required runtime tools (this binary does not bundle them).\n");

    let checks = [
        (
            "git",
            vec!["--version"],
            "project root detection",
            "Install Git for your OS (e.g. `brew install git` or distro package `git`).",
        ),
        (
            "zellij",
            vec!["--version"],
            "multi-pane session host (0.44+ required)",
            "Install Zellij 0.44+: macOS `brew install zellij`; Linux `cargo install --locked zellij` or https://github.com/zellij-org/zellij/releases",
        ),
        (
            "codex",
            vec!["--version"],
            "interactive Codex CLI (authenticate after install)",
            "Install Codex CLI and sign in: https://github.com/openai/codex (or your usual Codex install path).",
        ),
        (
            "grok",
            vec!["version"],
            "interactive Grok Build CLI (authenticate after install)",
            "Install Grok Build and sign in: https://x.ai/cli",
        ),
    ];

    let mut missing: Vec<(&str, &str)> = Vec::new();
    for (program, args, purpose, hint) in checks {
        match Command::new(program).args(args).output().await {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let version = if stdout.trim().is_empty() {
                    stderr.trim()
                } else {
                    stdout.trim()
                };
                println!("✓ {program:<8} {version}");
                println!("         {purpose}");
            }
            Ok(output) => {
                missing.push((program, hint));
                eprintln!(
                    "✗ {program:<8} exited with {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                eprintln!("         {purpose}");
            }
            Err(error) => {
                missing.push((program, hint));
                eprintln!("✗ {program:<8} {error}");
                eprintln!("         {purpose}");
            }
        }
    }

    println!();
    if missing.is_empty() {
        println!(
            "All required tools are available. Run `osanwe` in a project to onboard and launch."
        );
        Ok(())
    } else {
        println!("Missing or broken tools — install guidance:\n");
        for (program, hint) in &missing {
            println!("  • {program}");
            println!("      {hint}");
        }
        println!();
        println!("After installing, re-run: osanwe doctor");
        println!("Then from a git project:   osanwe");
        bail!("one or more required runtime tools are unavailable")
    }
}

fn require_unix() -> anyhow::Result<()> {
    if cfg!(unix) {
        Ok(())
    } else {
        bail!("Osanwe currently requires Linux, macOS, or WSL")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_osanwe_parses_as_no_subcommand() {
        let cli = Cli::try_parse_from(["osanwe"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn onboard_defaults_flag_parses() {
        let cli = Cli::try_parse_from([
            "osanwe",
            "onboard",
            "--defaults",
            "--repo",
            "/tmp/proj",
            "--force",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Onboard {
                repo,
                defaults,
                force,
                launch,
                no_attach,
            }) => {
                assert_eq!(repo, PathBuf::from("/tmp/proj"));
                assert!(defaults);
                assert!(force);
                assert!(!launch);
                assert!(!no_attach);
            }
            _ => panic!("expected onboard"),
        }
    }

    #[test]
    fn doctor_and_attach_are_visible_commands() {
        let cli = Cli::try_parse_from(["osanwe", "doctor"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Doctor)));
        let cli = Cli::try_parse_from(["osanwe", "attach", "--repo", "."]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Attach { .. })));
    }
}
