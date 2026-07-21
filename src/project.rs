//! Project-local `.osanwe/` file-bus workspace: config, scaffold, paths.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};

/// Directory name under a Git project root.
pub const OSANWE_DIR: &str = ".osanwe";

/// Required subdirectories under `.osanwe/`.
pub const SCAFFOLD_DIRS: &[&str] = &[
    "todos",
    "plans",
    "sessions",
    "workers",
    "prompts",
    "assignments",
    "logs",
    "board",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Codex,
    Grok,
}

impl ClientKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
        }
    }

    #[must_use]
    pub const fn program(self) -> &'static str {
        self.as_str()
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "grok" => Ok(Self::Grok),
            other => bail!("unknown client: {other} (expected codex or grok)"),
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 2] {
        [Self::Codex, Self::Grok]
    }
}

impl std::fmt::Display for ClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A selectable model id for onboarding (CLI `-m` value).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelOption {
    pub id: &'static str,
    pub label: &'static str,
}

/// Codex CLI models selectable via `codex -m` / `/model`
/// (from local `~/.codex/models_cache.json` list visibility; excludes hidden).
pub const CODEX_MODELS: &[ModelOption] = &[
    ModelOption {
        id: "gpt-5.6-sol",
        label: "GPT-5.6 Sol (flagship)",
    },
    ModelOption {
        id: "gpt-5.6-terra",
        label: "GPT-5.6 Terra (balanced)",
    },
    ModelOption {
        id: "gpt-5.6-luna",
        label: "GPT-5.6 Luna (fast/cheap)",
    },
    ModelOption {
        id: "gpt-5.5",
        label: "GPT-5.5",
    },
    ModelOption {
        id: "gpt-5.4",
        label: "GPT-5.4",
    },
    ModelOption {
        id: "gpt-5.4-mini",
        label: "GPT-5.4 Mini",
    },
    ModelOption {
        id: "gpt-5.3-codex-spark",
        label: "GPT-5.3 Codex Spark",
    },
];

/// Grok Build CLI models from `grok models` (stock catalog only — not xAI API ids,
/// not user-defined `[model.*]` proxies in `~/.grok/config.toml`).
pub const GROK_MODELS: &[ModelOption] = &[ModelOption {
    id: "grok-4.5",
    label: "Grok 4.5 (default)",
}];

impl ClientKind {
    /// Models offered in onboarding for this client.
    #[must_use]
    pub const fn models(self) -> &'static [ModelOption] {
        match self {
            Self::Codex => CODEX_MODELS,
            Self::Grok => GROK_MODELS,
        }
    }

    /// Default model id for this client (first catalog entry).
    #[must_use]
    pub const fn default_model_id(self) -> &'static str {
        self.models()[0].id
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleChoice {
    pub client: ClientKind,
    /// Empty string means provider default model.
    #[serde(default)]
    pub model: String,
}

impl RoleChoice {
    #[must_use]
    pub fn new(client: ClientKind, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RolesConfig {
    pub orchestrator: RoleChoice,
    pub planner: RoleChoice,
    pub worker: RoleChoice,
    #[serde(default)]
    pub verifier: Option<RoleChoice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectConfig {
    pub schema_version: String,
    pub zellij_session: String,
    pub enable_verifier: bool,
    pub roles: RolesConfig,
}

impl ProjectConfig {
    #[must_use]
    pub fn defaults_for_repo(project_root: &Path) -> Self {
        let session = default_session_name(project_root);
        Self {
            schema_version: "1".into(),
            zellij_session: session,
            enable_verifier: true,
            roles: RolesConfig {
                orchestrator: RoleChoice::new(
                    ClientKind::Codex,
                    ClientKind::Codex.default_model_id(),
                ),
                planner: RoleChoice::new(ClientKind::Codex, ClientKind::Codex.default_model_id()),
                worker: RoleChoice::new(ClientKind::Grok, ClientKind::Grok.default_model_id()),
                verifier: Some(RoleChoice::new(
                    ClientKind::Codex,
                    ClientKind::Codex.default_model_id(),
                )),
            },
        }
    }

    #[must_use]
    pub fn active_roles(&self) -> Vec<(&'static str, &RoleChoice)> {
        let mut roles = vec![
            ("orchestrator", &self.roles.orchestrator),
            ("planner", &self.roles.planner),
            ("worker", &self.roles.worker),
        ];
        if self.enable_verifier {
            if let Some(verifier) = &self.roles.verifier {
                roles.push(("verifier", verifier));
            }
        }
        roles
    }
}

#[must_use]
pub fn osanwe_dir(project_root: &Path) -> PathBuf {
    project_root.join(OSANWE_DIR)
}

#[must_use]
pub fn config_path(project_root: &Path) -> PathBuf {
    osanwe_dir(project_root).join("config.toml")
}

#[must_use]
pub fn config_exists(project_root: &Path) -> bool {
    config_path(project_root).is_file()
}

/// Create the `.osanwe` tree (idempotent). Does not overwrite existing config.
pub fn scaffold(project_root: &Path) -> anyhow::Result<PathBuf> {
    let root = osanwe_dir(project_root);
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    for name in SCAFFOLD_DIRS {
        let dir = root.join(name);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    }

    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        fs::write(
            &gitignore,
            "logs/\nsessions/current.json\nboard/status.md\n",
        )
        .with_context(|| format!("write {}", gitignore.display()))?;
    }

    write_prompt_templates(&root)?;
    write_file_bus_readme(&root)?;

    let board = root.join("board/status.md");
    if !board.exists() {
        fs::write(
            &board,
            "# Osanwe board\n\nNo session activity yet.\n",
        )
        .with_context(|| format!("write {}", board.display()))?;
    }

    Ok(root)
}

/// Scaffold directories and persist `config.toml`.
pub fn scaffold_with_config(project_root: &Path, config: &ProjectConfig) -> anyhow::Result<PathBuf> {
    let root = scaffold(project_root)?;
    save_config(project_root, config)?;
    Ok(root)
}

pub fn save_config(project_root: &Path, config: &ProjectConfig) -> anyhow::Result<()> {
    let root = osanwe_dir(project_root);
    fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
    let path = config_path(project_root);
    let text = toml::to_string_pretty(config).context("serialize project config")?;
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, text).with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, &path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn load_config(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    let path = config_path(project_root);
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let config: ProjectConfig = toml::from_str(&text).context("parse .osanwe/config.toml")?;
    if config.schema_version != "1" {
        bail!(
            "unsupported .osanwe schema_version {} (expected 1)",
            config.schema_version
        );
    }
    Ok(config)
}

#[must_use]
pub fn default_session_name(project_root: &Path) -> String {
    let abs = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let digest = short_digest(abs.to_string_lossy().as_bytes());
    format!("osanwe-{digest}")
}

fn short_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    hex::encode(&hash[..4])
}

fn write_prompt_templates(root: &Path) -> anyhow::Result<()> {
    let prompts = [
        (
            "orchestrator.md",
            concat!(
                "# Orchestrator\n\n",
                "You are the Osanwe orchestrator for this repository.\n",
                "Shared file bus: `.osanwe/` in the project root.\n\n",
                "- Put goals and todos under `.osanwe/todos/` (markdown files).\n",
                "- Ask the planner to write plans under `.osanwe/plans/`.\n",
                "- Track worker assignments under `.osanwe/assignments/`.\n",
                "- Session metadata lives under `.osanwe/sessions/`.\n",
                "Coordinate by writing clear files other roles can read.\n"
            ),
        ),
        (
            "planner.md",
            concat!(
                "# Planner\n\n",
                "You are the Osanwe planner.\n",
                "Read todos from `.osanwe/todos/` and write structured plans to `.osanwe/plans/`.\n",
                "Do not implement code unless the orchestrator explicitly asks.\n"
            ),
        ),
        (
            "worker.md",
            concat!(
                "# Worker\n\n",
                "You are the Osanwe worker.\n",
                "Read the current assignment under `.osanwe/assignments/` and the approved plan under `.osanwe/plans/`.\n",
                "Implement in the repository working tree. Report status under `.osanwe/workers/`.\n"
            ),
        ),
        (
            "verifier.md",
            concat!(
                "# Verifier\n\n",
                "You are the independent Osanwe verifier.\n",
                "Review the plan, worker report, and repository diff. Do not implement features.\n",
                "Write your verification notes under `.osanwe/board/` or `.osanwe/assignments/`.\n"
            ),
        ),
    ];
    for (name, body) in prompts {
        let path = root.join("prompts").join(name);
        if !path.exists() {
            fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        }
    }
    Ok(())
}

fn write_file_bus_readme(root: &Path) -> anyhow::Result<()> {
    let path = root.join("README.md");
    if path.exists() {
        return Ok(());
    }
    let body = concat!(
        "# Osanwe file bus\n\n",
        "This directory is the shared coordination space for orchestrator, planner, worker, and verifier sessions.\n\n",
        "| Path | Purpose |\n",
        "| --- | --- |\n",
        "| `config.toml` | Role → client + model choices |\n",
        "| `todos/` | Goals and todo items |\n",
        "| `plans/` | Planner output |\n",
        "| `assignments/` | Work items for workers |\n",
        "| `workers/` | Worker status / notes |\n",
        "| `sessions/` | Session metadata |\n",
        "| `prompts/` | Role bootstrap prompts |\n",
        "| `board/` | Human-readable status |\n",
        "| `logs/` | Local logs |\n\n",
        "Re-run onboarding with `osanwe onboard`.\n",
    );
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Resolve a Git project root from a starting path (best-effort: walk up for `.git`).
pub fn find_project_root(start: &Path) -> anyhow::Result<PathBuf> {
    let start = if start.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        start.to_path_buf()
    };
    let mut current = start
        .canonicalize()
        .with_context(|| format!("resolve path {}", start.display()))?;
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        if current.join(OSANWE_DIR).join("config.toml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    // Fall back to the starting path (allows tests without a real git repo).
    start
        .canonicalize()
        .with_context(|| format!("resolve path {}", start.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scaffold_creates_required_tree_and_persists_config() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let config = ProjectConfig::defaults_for_repo(root);
        scaffold_with_config(root, &config).unwrap();

        assert!(config_path(root).is_file());
        for name in SCAFFOLD_DIRS {
            assert!(
                root.join(OSANWE_DIR).join(name).is_dir(),
                "missing dir {name}"
            );
        }
        assert!(root.join(OSANWE_DIR).join("prompts/orchestrator.md").is_file());
        assert!(root.join(OSANWE_DIR).join("README.md").is_file());

        let loaded = load_config(root).unwrap();
        assert_eq!(loaded.roles.worker.client, ClientKind::Grok);
        assert_eq!(loaded.roles.planner.client, ClientKind::Codex);
        assert!(loaded.enable_verifier);
        assert!(!loaded.zellij_session.is_empty());
    }

    #[test]
    fn config_roundtrip_preserves_role_client_and_model() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let mut config = ProjectConfig::defaults_for_repo(root);
        config.roles.orchestrator = RoleChoice::new(ClientKind::Grok, "grok-fast");
        config.roles.worker = RoleChoice::new(ClientKind::Codex, "o3");
        config.enable_verifier = false;
        config.roles.verifier = None;
        scaffold_with_config(root, &config).unwrap();

        let loaded = load_config(root).unwrap();
        assert_eq!(loaded.roles.orchestrator.client, ClientKind::Grok);
        assert_eq!(loaded.roles.orchestrator.model, "grok-fast");
        assert_eq!(loaded.roles.worker.client, ClientKind::Codex);
        assert_eq!(loaded.roles.worker.model, "o3");
        assert!(!loaded.enable_verifier);
        assert_eq!(loaded.active_roles().len(), 3);
    }

    #[test]
    fn scaffold_is_idempotent() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let config = ProjectConfig::defaults_for_repo(root);
        scaffold_with_config(root, &config).unwrap();
        // Second scaffold must not fail or wipe config.
        scaffold(root).unwrap();
        let loaded = load_config(root).unwrap();
        assert_eq!(loaded.schema_version, "1");
    }
}
