use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

use crate::model::{RunManifest, WorkflowEvent};

#[derive(Clone, Debug)]
pub struct RunStore {
    root: PathBuf,
}

impl RunStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_environment() -> anyhow::Result<Self> {
        Ok(Self::new(state_home()?))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    #[must_use]
    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir().join(run_id)
    }

    #[must_use]
    pub fn manifest_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("manifest.json")
    }

    #[must_use]
    pub fn socket_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join("relayd.sock")
    }

    pub fn create_run_dir(&self, run_id: &str) -> anyhow::Result<PathBuf> {
        let run_dir = self.run_dir(run_id);
        for directory in [
            run_dir.clone(),
            run_dir.join("assignments"),
            run_dir.join("checks"),
            run_dir.join("checkpoints"),
            run_dir.join("provider-overlays"),
            run_dir.join("logs"),
        ] {
            fs::create_dir_all(&directory)
                .with_context(|| format!("create {}", directory.display()))?;
            private_directory(&directory)?;
        }
        Ok(run_dir)
    }

    pub fn save(&self, manifest: &RunManifest) -> anyhow::Result<()> {
        self.create_run_dir(&manifest.run_id)?;
        let target = self.manifest_path(&manifest.run_id);
        let temporary = target.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(manifest)?;
        fs::write(&temporary, bytes)
            .with_context(|| format!("write {}", temporary.display()))?;
        private_file(&temporary)?;
        fs::rename(&temporary, &target)
            .with_context(|| format!("replace {}", target.display()))?;
        Ok(())
    }

    pub fn load(&self, run_id: &str) -> anyhow::Result<RunManifest> {
        let path = self.manifest_path(run_id);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("decode {}", path.display()))
    }

    pub fn list(&self) -> anyhow::Result<Vec<RunManifest>> {
        let directory = self.runs_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let mut runs = Vec::new();
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("read {}", directory.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let run_id = entry.file_name().to_string_lossy().into_owned();
            if let Ok(manifest) = self.load(&run_id) {
                runs.push(manifest);
            }
        }
        runs.sort_by_key(|run| std::cmp::Reverse(run.updated_at));
        Ok(runs)
    }

    pub fn append_event(&self, event: &WorkflowEvent) -> anyhow::Result<()> {
        self.create_run_dir(&event.run_id)?;
        let path = self.run_dir(&event.run_id).join("events.ndjson");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        file.flush()?;
        private_file(&path)?;
        Ok(())
    }

    pub fn write_artifact(
        &self,
        run_id: &str,
        relative_path: &Path,
        bytes: &[u8],
    ) -> anyhow::Result<PathBuf> {
        validate_relative_path(relative_path)?;
        let path = self.run_dir(run_id).join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            private_directory(parent)?;
        }
        fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        private_file(&path)?;
        Ok(path)
    }
}

pub fn state_home() -> anyhow::Result<PathBuf> {
    if let Some(path) = env::var_os("OSANWE_STATE_HOME") {
        return Ok(PathBuf::from(path));
    }
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("osanwe"));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".local/state/osanwe"));
    }
    bail!("cannot determine state directory; set OSANWE_STATE_HOME")
}

fn validate_relative_path(path: &Path) -> anyhow::Result<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("artifact path must stay inside the run directory")
    }
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn private_directory(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn private_file(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn private_file(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::model::RunManifest;

    #[test]
    fn manifest_round_trip_is_atomic_and_lossless() {
        let temporary = TempDir::new().unwrap();
        let store = RunStore::new(temporary.path().join("state"));
        let manifest = RunManifest::new_with_id(
            "run-1".into(),
            "task",
            temporary.path().join("repo"),
            store.run_dir("run-1"),
            "abc".into(),
            temporary.path().join("integration"),
        );

        store.save(&manifest).unwrap();
        assert_eq!(store.load("run-1").unwrap(), manifest);
        assert!(!store.manifest_path("run-1").with_extension("json.tmp").exists());
    }

    #[test]
    fn artifact_paths_cannot_escape_the_run_directory() {
        let temporary = TempDir::new().unwrap();
        let store = RunStore::new(temporary.path().join("state"));
        let error = store
            .write_artifact("run-1", Path::new("../secret"), b"no")
            .unwrap_err();
        assert!(error.to_string().contains("inside"));
    }
}
