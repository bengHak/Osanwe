use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};

use crate::process::{CommandOutput, CommandRunner, CommandSpec};

#[derive(Clone, Debug)]
pub struct RepositorySnapshot {
    pub root: PathBuf,
    pub base_sha: String,
}

#[derive(Clone)]
pub struct WorkspaceManager {
    runner: Arc<dyn CommandRunner>,
}

impl WorkspaceManager {
    #[must_use]
    pub fn new(runner: Arc<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    pub async fn preflight(&self, path: &Path) -> anyhow::Result<RepositorySnapshot> {
        let root = self
            .git(path, ["rev-parse", "--show-toplevel"])
            .await?
            .require_success("find Git repository")?
            .stdout
            .trim()
            .to_owned();
        let root = PathBuf::from(root);
        let status = self
            .git(&root, ["status", "--porcelain=v2", "--untracked-files=no"])
            .await?
            .require_success("inspect repository status")?;
        if !status.stdout.trim().is_empty() {
            bail!("repository has tracked changes; commit or stash them before starting Osanwe")
        }
        let base_sha = self
            .git(&root, ["rev-parse", "HEAD"])
            .await?
            .require_success("resolve HEAD")?
            .stdout
            .trim()
            .to_owned();
        Ok(RepositorySnapshot { root, base_sha })
    }

    pub async fn create_worktree(
        &self,
        repository: &Path,
        destination: &Path,
        branch: &str,
        base_sha: &str,
    ) -> anyhow::Result<()> {
        validate_branch(branch)?;
        if destination.exists() {
            bail!("worktree destination already exists: {}", destination.display())
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let destination = path_text(destination)?;
        self.git(
            repository,
            ["worktree", "add", "-b", branch, &destination, base_sha],
        )
        .await?
        .require_success("create Git worktree")?;
        Ok(())
    }

    pub async fn checkpoint(
        &self,
        worktree: &Path,
        base_sha: &str,
        message: &str,
    ) -> anyhow::Result<Option<String>> {
        self.git(worktree, ["add", "-A"])
            .await?
            .require_success("stage worker changes")?;
        let diff = self.git(worktree, ["diff", "--cached", "--quiet"]).await?;
        match diff.status {
            0 => {}
            1 => {
                self.git(
                    worktree,
                    [
                        "-c",
                        "user.name=Osanwe",
                        "-c",
                        "user.email=osanwe@localhost",
                        "commit",
                        "-m",
                        message,
                        "--no-gpg-sign",
                    ],
                )
                .await?
                .require_success("create worker checkpoint")?;
            }
            status => bail!("inspect staged worker changes failed with status {status}"),
        }
        let sha = self
            .git(worktree, ["rev-parse", "HEAD"])
            .await?
            .require_success("resolve worker checkpoint")?
            .stdout
            .trim()
            .to_owned();
        if sha == base_sha {
            Ok(None)
        } else {
            Ok(Some(sha))
        }
    }

    pub async fn integrate(
        &self,
        integration: &Path,
        base_sha: &str,
        head_sha: &str,
    ) -> anyhow::Result<()> {
        let range = format!("{base_sha}..{head_sha}");
        self.git(integration, ["cherry-pick", &range])
            .await?
            .require_success("integrate worker checkpoint")?;
        Ok(())
    }

    pub async fn abort_integration(&self, integration: &Path) -> anyhow::Result<()> {
        let output = self.git(integration, ["cherry-pick", "--abort"]).await?;
        if output.status != 0 && !output.stderr.contains("no cherry-pick") {
            return output
                .require_success("abort cherry-pick")
                .map(|_| ());
        }
        Ok(())
    }

    pub async fn changed_paths(
        &self,
        worktree: &Path,
        base_sha: &str,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let output = self
            .git(worktree, ["diff", "--name-only", base_sha, "HEAD"])
            .await?
            .require_success("list changed paths")?;
        Ok(output
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(PathBuf::from)
            .collect())
    }

    pub async fn diff_patch(&self, worktree: &Path, base_sha: &str) -> anyhow::Result<String> {
        Ok(self
            .git(worktree, ["diff", "--binary", base_sha, "HEAD"])
            .await?
            .require_success("create integrated diff")?
            .stdout)
    }

    pub async fn diff_stat(&self, worktree: &Path, base_sha: &str) -> anyhow::Result<String> {
        Ok(self
            .git(worktree, ["diff", "--stat", base_sha, "HEAD"])
            .await?
            .require_success("create diff summary")?
            .stdout)
    }

    pub async fn remove_worktree(
        &self,
        repository: &Path,
        worktree: &Path,
    ) -> anyhow::Result<()> {
        let worktree = path_text(worktree)?;
        self.git(repository, ["worktree", "remove", "--force", &worktree])
            .await?
            .require_success("remove Git worktree")?;
        Ok(())
    }

    async fn git<I, S>(&self, cwd: &Path, args: I) -> anyhow::Result<CommandOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.runner
            .run(&CommandSpec::new("git").args(args).cwd(cwd))
            .await
            .with_context(|| format!("run git in {}", cwd.display()))
    }
}

fn validate_branch(branch: &str) -> anyhow::Result<()> {
    if branch.is_empty()
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.chars().any(char::is_whitespace)
    {
        bail!("invalid generated branch name: {branch}")
    }
    Ok(())
}

pub fn validate_relative_scope(path: &Path) -> anyhow::Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path must remain inside the worktree: {}", path.display())
    }
    Ok(())
}

fn path_text(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .with_context(|| format!("path is not valid UTF-8: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;
    use crate::process::TokioCommandRunner;

    fn initialize_repository(path: &Path) {
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .status()
            .unwrap();
        std::fs::write(path.join("README.md"), "base\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .status()
            .unwrap();
        Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "base",
            ])
            .current_dir(path)
            .status()
            .unwrap();
    }

    #[tokio::test]
    async fn worker_checkpoint_integrates_into_isolated_worktree() {
        let temporary = TempDir::new().unwrap();
        let repo = temporary.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        initialize_repository(&repo);
        let manager = WorkspaceManager::new(Arc::new(TokioCommandRunner));
        let snapshot = manager.preflight(&repo).await.unwrap();
        let integration = temporary.path().join("integration");
        let worker = temporary.path().join("worker");
        manager
            .create_worktree(
                &repo,
                &integration,
                "osanwe/test/integration",
                &snapshot.base_sha,
            )
            .await
            .unwrap();
        manager
            .create_worktree(
                &repo,
                &worker,
                "osanwe/test/worker-1",
                &snapshot.base_sha,
            )
            .await
            .unwrap();

        std::fs::write(worker.join("feature.txt"), "implemented\n").unwrap();
        let checkpoint = manager
            .checkpoint(&worker, &snapshot.base_sha, "worker result")
            .await
            .unwrap()
            .unwrap();
        manager
            .integrate(&integration, &snapshot.base_sha, &checkpoint)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(integration.join("feature.txt")).unwrap(),
            "implemented\n"
        );
        assert!(!repo.join("feature.txt").exists());
    }
}
