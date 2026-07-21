use std::path::{Component, Path};
use std::process::Stdio;
use std::time::Instant;

use anyhow::{bail, Context};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::model::{CheckResult, CheckSpec};

pub async fn run_checks(worktree: &Path, checks: &[CheckSpec]) -> Vec<CheckResult> {
    let checks = if checks.is_empty() {
        default_checks(worktree)
    } else {
        checks.to_vec()
    };
    let mut results = Vec::with_capacity(checks.len());
    for check in checks {
        let result = run_check(worktree, &check).await.unwrap_or_else(|error| CheckResult {
            id: check.id.clone(),
            passed: false,
            required: check.required,
            exit_code: None,
            duration_ms: 0,
            summary: error.to_string(),
        });
        results.push(result);
    }
    results
}

pub async fn run_check(worktree: &Path, check: &CheckSpec) -> anyhow::Result<CheckResult> {
    validate_check(check)?;
    let cwd = worktree.join(&check.cwd);
    let canonical_worktree = worktree
        .canonicalize()
        .with_context(|| format!("resolve {}", worktree.display()))?;
    let canonical_cwd = cwd
        .canonicalize()
        .with_context(|| format!("resolve check cwd {}", cwd.display()))?;
    if !canonical_cwd.starts_with(&canonical_worktree) {
        bail!("check cwd escapes the integration worktree")
    }

    println!("\n==> [{}] {} {}", check.id, check.program, check.args.join(" "));
    let started = Instant::now();
    let mut command = Command::new(&check.program);
    command
        .args(&check.args)
        .current_dir(&canonical_cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = timeout(
        Duration::from_secs(check.timeout_seconds),
        command.status(),
    )
    .await
    .with_context(|| format!("check {} timed out", check.id))?
    .with_context(|| format!("start check {}", check.id))?;
    let duration_ms = started.elapsed().as_millis();
    let exit_code = status.code();
    Ok(CheckResult {
        id: check.id.clone(),
        passed: status.success(),
        required: check.required,
        exit_code,
        duration_ms,
        summary: if status.success() {
            format!("passed in {duration_ms} ms")
        } else {
            format!("failed with status {exit_code:?} in {duration_ms} ms")
        },
    })
}

fn validate_check(check: &CheckSpec) -> anyhow::Result<()> {
    if check.program.trim().is_empty() {
        bail!("check program cannot be empty")
    }
    if check.cwd.is_absolute()
        || check
            .cwd
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("check cwd must be relative to the integration worktree")
    }
    if check.timeout_seconds == 0 {
        bail!("check timeout must be at least one second")
    }
    Ok(())
}

#[must_use]
pub fn default_checks(worktree: &Path) -> Vec<CheckSpec> {
    if worktree.join("Cargo.toml").exists() {
        return vec![
            CheckSpec {
                id: "cargo-fmt".into(),
                program: "cargo".into(),
                args: vec!["fmt".into(), "--all".into(), "--".into(), "--check".into()],
                cwd: ".".into(),
                required: true,
                timeout_seconds: 120,
            },
            CheckSpec {
                id: "cargo-test".into(),
                program: "cargo".into(),
                args: vec!["test".into(), "--all-features".into()],
                cwd: ".".into(),
                required: true,
                timeout_seconds: 1_200,
            },
        ];
    }
    if worktree.join("package.json").exists() {
        return vec![CheckSpec {
            id: "npm-test".into(),
            program: "npm".into(),
            args: vec!["test".into()],
            cwd: ".".into(),
            required: true,
            timeout_seconds: 1_200,
        }];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn check_cwd_must_stay_inside_worktree() {
        let temporary = TempDir::new().unwrap();
        let check = CheckSpec {
            id: "escape".into(),
            program: "true".into(),
            args: Vec::new(),
            cwd: "..".into(),
            required: true,
            timeout_seconds: 1,
        };
        let error = run_check(temporary.path(), &check).await.unwrap_err();
        assert!(error.to_string().contains("relative"));
    }
}
