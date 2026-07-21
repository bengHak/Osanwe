//! First-run onboarding: choose client and model from catalogs in separate steps.

use std::io::{self, Write};
use std::path::Path;

use anyhow::{bail, Context};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{execute, terminal};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Terminal;

use crate::project::{
    default_session_name, scaffold_with_config, ClientKind, ModelOption, ProjectConfig, RoleChoice,
    RolesConfig,
};

#[derive(Clone, Debug)]
struct RoleDraft {
    name: &'static str,
    client_idx: usize,
    /// Index into `ClientKind::models()` for the selected client.
    model_idx: usize,
    enabled: bool,
    optional: bool,
}

impl RoleDraft {
    fn client(&self, clients: &[ClientKind; 2]) -> ClientKind {
        clients[self.client_idx]
    }

    fn model_id(&self, clients: &[ClientKind; 2]) -> &'static str {
        let models = self.client(clients).models();
        models
            .get(self.model_idx)
            .map(|m| m.id)
            .unwrap_or_else(|| self.client(clients).default_model_id())
    }
}

/// Separate onboarding phases: pick clients for all roles, then models for all roles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardPhase {
    /// Choosing the interactive client (codex / grok) for one role.
    PickClient { role_index: usize },
    /// Choosing the model from the client catalog for one role.
    PickModel { role_index: usize },
}

/// Pure wizard state used by the TUI (and unit-tested without a terminal).
#[derive(Clone, Debug)]
pub struct OnboardWizard {
    roles: Vec<RoleDraft>,
    clients: [ClientKind; 2],
    phase: OnboardPhase,
}

impl OnboardWizard {
    #[must_use]
    pub fn new() -> Self {
        Self {
            roles: default_role_drafts(),
            clients: ClientKind::all(),
            phase: OnboardPhase::PickClient { role_index: 0 },
        }
    }

    #[must_use]
    pub fn phase(&self) -> OnboardPhase {
        self.phase
    }

    #[must_use]
    pub fn roles_summary(&self) -> Vec<(String, String, String, bool)> {
        self.roles
            .iter()
            .map(|r| {
                (
                    r.name.to_owned(),
                    r.client(&self.clients).to_string(),
                    r.model_id(&self.clients).to_owned(),
                    r.enabled,
                )
            })
            .collect()
    }

    #[must_use]
    fn current_role(&self) -> Option<&RoleDraft> {
        let idx = match self.phase {
            OnboardPhase::PickClient { role_index } | OnboardPhase::PickModel { role_index } => {
                role_index
            }
        };
        self.roles.get(idx)
    }

    #[must_use]
    pub fn current_models(&self) -> &'static [ModelOption] {
        self.current_role()
            .map(|r| r.client(&self.clients).models())
            .unwrap_or(ClientKind::Codex.models())
    }

    #[must_use]
    pub fn current_model_idx(&self) -> usize {
        self.current_role().map(|r| r.model_idx).unwrap_or(0)
    }

    /// Cycle client for the role currently in PickClient phase.
    pub fn cycle_client(&mut self, delta: isize) {
        let OnboardPhase::PickClient { role_index } = self.phase else {
            return;
        };
        let Some(role) = self.roles.get_mut(role_index) else {
            return;
        };
        if !role.enabled {
            return;
        }
        let n = self.clients.len() as isize;
        let next = (role.client_idx as isize + delta).rem_euclid(n) as usize;
        role.client_idx = next;
        role.model_idx = 0;
    }

    /// Cycle model for the role currently in PickModel phase (catalog only).
    pub fn cycle_model(&mut self, delta: isize) {
        let OnboardPhase::PickModel { role_index } = self.phase else {
            return;
        };
        let Some(role) = self.roles.get_mut(role_index) else {
            return;
        };
        if !role.enabled {
            return;
        }
        let models = self.clients[role.client_idx].models();
        if models.is_empty() {
            return;
        }
        let n = models.len() as isize;
        role.model_idx = (role.model_idx as isize + delta).rem_euclid(n) as usize;
    }

    /// Toggle optional roles (verifier) during client phase only.
    pub fn toggle_optional_enabled(&mut self) {
        let OnboardPhase::PickClient { role_index } = self.phase else {
            return;
        };
        if let Some(role) = self.roles.get_mut(role_index) {
            if role.optional {
                role.enabled = !role.enabled;
            }
        }
    }

    /// Advance to the next step. Returns `true` when the wizard is complete.
    pub fn confirm_current(&mut self) -> bool {
        match self.phase {
            OnboardPhase::PickClient { role_index } => {
                let next = next_enabled_or_any_index(&self.roles, role_index + 1, true);
                if let Some(idx) = next {
                    self.phase = OnboardPhase::PickClient { role_index: idx };
                    false
                } else {
                    let first = first_enabled_index(&self.roles).unwrap_or(0);
                    self.phase = OnboardPhase::PickModel { role_index: first };
                    false
                }
            }
            OnboardPhase::PickModel { role_index } => {
                let next = next_enabled_index(&self.roles, role_index + 1);
                if let Some(idx) = next {
                    self.phase = OnboardPhase::PickModel { role_index: idx };
                    false
                } else {
                    true
                }
            }
        }
    }

    /// Go back one step. Returns `false` if already at the first step.
    pub fn go_back(&mut self) -> bool {
        match self.phase {
            OnboardPhase::PickClient { role_index } => {
                if role_index == 0 {
                    return false;
                }
                self.phase = OnboardPhase::PickClient {
                    role_index: role_index - 1,
                };
                true
            }
            OnboardPhase::PickModel { role_index } => {
                if let Some(prev) = prev_enabled_index(&self.roles, role_index) {
                    self.phase = OnboardPhase::PickModel { role_index: prev };
                    true
                } else {
                    let last = self.roles.len().saturating_sub(1);
                    self.phase = OnboardPhase::PickClient { role_index: last };
                    true
                }
            }
        }
    }

    pub fn build_config(&self, project_root: &Path) -> anyhow::Result<ProjectConfig> {
        build_config(project_root, &self.roles, &self.clients)
    }
}

impl Default for OnboardWizard {
    fn default() -> Self {
        Self::new()
    }
}

fn default_role_drafts() -> Vec<RoleDraft> {
    vec![
        RoleDraft {
            name: "orchestrator",
            client_idx: 0,
            model_idx: 0,
            enabled: true,
            optional: false,
        },
        RoleDraft {
            name: "planner",
            client_idx: 0,
            model_idx: 0,
            enabled: true,
            optional: false,
        },
        RoleDraft {
            name: "worker",
            client_idx: 1,
            model_idx: 0,
            enabled: true,
            optional: false,
        },
        RoleDraft {
            name: "verifier",
            client_idx: 0,
            model_idx: 0,
            enabled: true,
            optional: true,
        },
    ]
}

fn first_enabled_index(roles: &[RoleDraft]) -> Option<usize> {
    roles.iter().position(|r| r.enabled)
}

fn next_enabled_index(roles: &[RoleDraft], from: usize) -> Option<usize> {
    roles
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, r)| r.enabled)
        .map(|(i, _)| i)
}

fn next_enabled_or_any_index(
    roles: &[RoleDraft],
    from: usize,
    include_disabled_optional: bool,
) -> Option<usize> {
    if from >= roles.len() {
        return None;
    }
    if include_disabled_optional {
        Some(from)
    } else {
        next_enabled_index(roles, from)
    }
}

fn prev_enabled_index(roles: &[RoleDraft], before: usize) -> Option<usize> {
    roles
        .iter()
        .enumerate()
        .take(before)
        .rev()
        .find(|(_, r)| r.enabled)
        .map(|(i, _)| i)
}

/// Apply onboarding choices without a TUI (for tests and `--defaults`).
pub fn apply_choices(project_root: &Path, config: &ProjectConfig) -> anyhow::Result<()> {
    scaffold_with_config(project_root, config)?;
    Ok(())
}

/// Write default role client/model config and scaffold `.osanwe/`.
pub fn apply_defaults(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    let config = ProjectConfig::defaults_for_repo(project_root);
    apply_choices(project_root, &config)?;
    Ok(config)
}

/// Interactive onboarding TUI.
pub fn run_onboarding(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    if !stdin_is_tty() {
        bail!(
            "onboarding requires an interactive terminal; re-run in a TTY or use `osanwe onboard --defaults`"
        );
    }
    run_tui_onboarding(project_root)
}

/// True only when stdin is an interactive terminal.
#[must_use]
pub fn stdin_is_tty() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal()
}

fn run_tui_onboarding(project_root: &Path) -> anyhow::Result<ProjectConfig> {
    let mut wizard = OnboardWizard::new();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut status = phase_help(wizard.phase());

    let result = loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(5),
                    Constraint::Min(8),
                    Constraint::Length(3),
                ])
                .split(frame.area());

            let (phase_title, phase_hint) = match wizard.phase() {
                OnboardPhase::PickClient { .. } => (
                    "Phase 1/2 — Choose CLIENT",
                    "Pick which CLI runs each role. Model comes next.",
                ),
                OnboardPhase::PickModel { .. } => (
                    "Phase 2/2 — Choose MODEL",
                    "Pick a model from the list for this role's client.",
                ),
            };

            let header = Paragraph::new(Line::from(vec![
                Span::styled(phase_title, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(" — {}", project_root.display())),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Osanwe onboarding"),
            );
            frame.render_widget(header, chunks[0]);

            let role = wizard.current_role();
            let role_name = role.map(|r| r.name).unwrap_or("?");
            let role_enabled = role.map(|r| r.enabled).unwrap_or(false);
            let role_optional = role.map(|r| r.optional).unwrap_or(false);
            let client = role
                .map(|r| r.client(&wizard.clients).to_string())
                .unwrap_or_default();
            let model = role.map(|r| r.model_id(&wizard.clients)).unwrap_or("");

            let step_lines = match wizard.phase() {
                OnboardPhase::PickClient { role_index } => {
                    vec![
                        Line::from(format!(
                            "Role {}/{}: {role_name}{}",
                            role_index + 1,
                            wizard.roles.len(),
                            if role_optional {
                                if role_enabled {
                                    " (optional — enabled)"
                                } else {
                                    " (optional — disabled, Space to enable)"
                                }
                            } else {
                                ""
                            }
                        )),
                        Line::from(phase_hint),
                        Line::from(format!("Current client: {client}")),
                    ]
                }
                OnboardPhase::PickModel { role_index } => {
                    let enabled_count = wizard.roles.iter().filter(|r| r.enabled).count();
                    let position = wizard
                        .roles
                        .iter()
                        .take(role_index + 1)
                        .filter(|r| r.enabled)
                        .count();
                    vec![
                        Line::from(format!(
                            "Role model {position}/{enabled_count}: {role_name} (client: {client})"
                        )),
                        Line::from(phase_hint),
                        Line::from(format!("Selected: {model}")),
                    ]
                }
            };
            let step = Paragraph::new(step_lines)
                .block(Block::default().borders(Borders::ALL).title("current step"));
            frame.render_widget(step, chunks[1]);

            match wizard.phase() {
                OnboardPhase::PickClient { .. } => {
                    let items: Vec<ListItem> = wizard
                        .clients
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let selected = role
                                .map(|r| r.client_idx == i && r.enabled)
                                .unwrap_or(false);
                            let mark = if selected { "▶ " } else { "  " };
                            ListItem::new(format!("{mark}{c}"))
                        })
                        .collect();
                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("clients (↑/↓ or ←/→)"),
                    );
                    frame.render_widget(list, chunks[2]);
                }
                OnboardPhase::PickModel { .. } => {
                    let models = wizard.current_models();
                    let selected_idx = wizard.current_model_idx();
                    let items: Vec<ListItem> = models
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let mark = if i == selected_idx { "▶ " } else { "  " };
                            ListItem::new(format!("{mark}{:<28}  {}", m.id, m.label))
                        })
                        .collect();
                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!("models for {client} (↑/↓ or j/k)")),
                    );
                    // Split: list + summary
                    let body_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(5), Constraint::Length(6)])
                        .split(chunks[2]);
                    frame.render_widget(list, body_chunks[0]);

                    let mut summary = vec![Line::from(Span::styled(
                        "Summary:",
                        Style::default().add_modifier(Modifier::BOLD),
                    ))];
                    for (name, c, m, enabled) in wizard.roles_summary() {
                        let flag = if enabled { "on " } else { "off" };
                        summary.push(Line::from(format!("  [{flag}] {name:<13} {c:<6} {m}")));
                    }
                    frame.render_widget(
                        Paragraph::new(summary)
                            .block(Block::default().borders(Borders::ALL).title("roles"))
                            .wrap(Wrap { trim: false }),
                        body_chunks[1],
                    );
                }
            }

            let footer = Paragraph::new(status.as_str())
                .block(Block::default().borders(Borders::ALL).title("keys"));
            frame.render_widget(footer, chunks[3]);
        })?;

        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match (wizard.phase(), key.code) {
            (_, KeyCode::Char('q')) => {
                break Err(anyhow::anyhow!("onboarding cancelled"));
            }
            (
                OnboardPhase::PickClient { .. },
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Up | KeyCode::Char('k'),
            ) => {
                wizard.cycle_client(-1);
                status = phase_help(wizard.phase());
            }
            (
                OnboardPhase::PickClient { .. },
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Down | KeyCode::Char('j'),
            ) => {
                wizard.cycle_client(1);
                status = phase_help(wizard.phase());
            }
            (OnboardPhase::PickClient { .. }, KeyCode::Char(' ')) => {
                wizard.toggle_optional_enabled();
                status = phase_help(wizard.phase());
            }
            (OnboardPhase::PickClient { .. }, KeyCode::Enter) => {
                let _ = wizard.confirm_current();
                status = phase_help(wizard.phase());
            }
            (OnboardPhase::PickClient { .. }, KeyCode::BackTab | KeyCode::Esc) => {
                if !wizard.go_back() {
                    status = "already at first step".into();
                } else {
                    status = phase_help(wizard.phase());
                }
            }
            (
                OnboardPhase::PickModel { .. },
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Left | KeyCode::Char('h'),
            ) => {
                wizard.cycle_model(-1);
                status = phase_help(wizard.phase());
            }
            (
                OnboardPhase::PickModel { .. },
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Right | KeyCode::Char('l'),
            ) => {
                wizard.cycle_model(1);
                status = phase_help(wizard.phase());
            }
            (OnboardPhase::PickModel { .. }, KeyCode::Enter) => {
                if wizard.confirm_current() {
                    match wizard.build_config(project_root) {
                        Ok(config) => {
                            apply_choices(project_root, &config)?;
                            break Ok(config);
                        }
                        Err(error) => status = format!("invalid: {error}"),
                    }
                } else {
                    status = phase_help(wizard.phase());
                }
            }
            (OnboardPhase::PickModel { .. }, KeyCode::BackTab | KeyCode::Esc) => {
                let _ = wizard.go_back();
                status = phase_help(wizard.phase());
            }
            _ => {}
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    let config = result?;
    println!(
        "Saved {}. Launch with: osanwe",
        crate::project::config_path(project_root).display()
    );
    let _ = io::stdout().flush();
    Ok(config)
}

fn phase_help(phase: OnboardPhase) -> String {
    match phase {
        OnboardPhase::PickClient { .. } => {
            "↑/↓ client · Space toggle optional · Enter next · Esc back · q quit".into()
        }
        OnboardPhase::PickModel { .. } => {
            "↑/↓ model · Enter next/finish · Esc back · q quit".into()
        }
    }
}

fn build_config(
    project_root: &Path,
    roles: &[RoleDraft],
    clients: &[ClientKind; 2],
) -> anyhow::Result<ProjectConfig> {
    let pick = |name: &str| -> anyhow::Result<RoleChoice> {
        let role = roles
            .iter()
            .find(|r| r.name == name)
            .with_context(|| format!("missing role draft {name}"))?;
        Ok(RoleChoice::new(
            role.client(clients),
            role.model_id(clients),
        ))
    };
    let verifier_draft = roles
        .iter()
        .find(|r| r.name == "verifier")
        .context("missing verifier draft")?;
    let enable_verifier = verifier_draft.enabled;
    Ok(ProjectConfig {
        schema_version: "1".into(),
        zellij_session: default_session_name(project_root),
        enable_verifier,
        roles: RolesConfig {
            orchestrator: pick("orchestrator")?,
            planner: pick("planner")?,
            worker: pick("worker")?,
            verifier: if enable_verifier {
                Some(pick("verifier")?)
            } else {
                None
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{load_config, CODEX_MODELS, GROK_MODELS};
    use tempfile::tempdir;

    #[test]
    fn stdin_is_tty_uses_is_terminal_not_raw_mode_ok() {
        use std::io::IsTerminal;
        assert_eq!(stdin_is_tty(), std::io::stdin().is_terminal());
    }

    #[test]
    fn catalogs_match_cli_not_api() {
        // Codex CLI list models (not API-only, not hidden auto-review).
        let codex_ids: Vec<_> = CODEX_MODELS.iter().map(|m| m.id).collect();
        assert_eq!(
            codex_ids,
            vec![
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex-spark",
            ]
        );
        assert!(!CODEX_MODELS.iter().any(|m| m.id == "codex-auto-review"));
        // Grok Build stock model from `grok models` (not xAI API catalog).
        assert_eq!(GROK_MODELS.len(), 1);
        assert_eq!(GROK_MODELS[0].id, "grok-4.5");
        assert!(!GROK_MODELS.iter().any(|m| m.id.starts_with("grok-4.20")));
        assert!(!GROK_MODELS.iter().any(|m| m.id == "grok-build-0.1"));
    }

    #[test]
    fn wizard_separates_client_phase_from_model_phase() {
        let mut w = OnboardWizard::new();
        for expected in 0..4 {
            assert!(
                matches!(w.phase(), OnboardPhase::PickClient { role_index } if role_index == expected),
                "expected client step {expected}, got {:?}",
                w.phase()
            );
            assert!(!w.confirm_current() || expected == 3);
        }
        assert!(matches!(
            w.phase(),
            OnboardPhase::PickModel { role_index: 0 }
        ));
        assert!(!w.confirm_current());
        assert!(!w.confirm_current());
        assert!(!w.confirm_current());
        assert!(w.confirm_current());
    }

    #[test]
    fn wizard_client_cycle_resets_model_to_catalog_default() {
        let mut w = OnboardWizard::new();
        w.cycle_client(1);
        let orch = w.current_role().unwrap();
        assert_eq!(orch.client(&w.clients), ClientKind::Grok);
        assert_eq!(orch.model_id(&w.clients), "grok-4.5");
        assert_eq!(orch.model_idx, 0);
        assert!(matches!(w.phase(), OnboardPhase::PickClient { .. }));
    }

    #[test]
    fn wizard_model_cycles_only_in_model_phase_from_catalog() {
        let mut w = OnboardWizard::new();
        w.cycle_model(1); // ignored in client phase
        assert_eq!(w.current_role().unwrap().model_idx, 0);

        for _ in 0..4 {
            let _ = w.confirm_current();
        }
        assert!(matches!(w.phase(), OnboardPhase::PickModel { .. }));
        assert_eq!(
            w.current_role().unwrap().model_id(&w.clients),
            "gpt-5.6-sol"
        );

        w.cycle_model(1);
        assert_eq!(
            w.current_role().unwrap().model_id(&w.clients),
            "gpt-5.6-terra"
        );
        w.cycle_model(-1);
        assert_eq!(
            w.current_role().unwrap().model_id(&w.clients),
            "gpt-5.6-sol"
        );

        // Wrap around
        w.cycle_model(-1);
        assert_eq!(
            w.current_role().unwrap().model_id(&w.clients),
            CODEX_MODELS.last().unwrap().id
        );
    }

    #[test]
    fn wizard_model_catalog_switches_with_client() {
        let mut w = OnboardWizard::new();
        w.cycle_client(1); // grok for orchestrator
        for _ in 0..4 {
            let _ = w.confirm_current();
        }
        assert!(matches!(
            w.phase(),
            OnboardPhase::PickModel { role_index: 0 }
        ));
        let models = w.current_models();
        assert_eq!(models[0].id, "grok-4.5");
        assert!(models
            .iter()
            .all(|m| GROK_MODELS.iter().any(|g| g.id == m.id)));
        // Single stock model — cycling wraps to the same id.
        w.cycle_model(1);
        assert_eq!(w.current_role().unwrap().model_id(&w.clients), "grok-4.5");
        assert_eq!(w.current_models().len(), 1);
    }

    #[test]
    fn wizard_disabled_verifier_skips_model_step() {
        let mut w = OnboardWizard::new();
        for _ in 0..3 {
            assert!(!w.confirm_current());
        }
        w.toggle_optional_enabled();
        assert!(!w.current_role().unwrap().enabled);
        assert!(!w.confirm_current());
        assert!(!w.confirm_current());
        assert!(!w.confirm_current());
        assert!(w.confirm_current());
        let dir = tempdir().unwrap();
        let config = w.build_config(dir.path()).unwrap();
        assert!(!config.enable_verifier);
        assert!(config.roles.verifier.is_none());
        assert_eq!(config.roles.orchestrator.model, "gpt-5.6-sol");
        assert_eq!(config.roles.worker.model, "grok-4.5");
    }

    #[test]
    fn apply_defaults_writes_loadable_config() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let config = apply_defaults(root).unwrap();
        let loaded = load_config(root).unwrap();
        assert_eq!(loaded.roles.orchestrator.client, ClientKind::Codex);
        assert_eq!(loaded.roles.orchestrator.model, "gpt-5.6-sol");
        assert_eq!(loaded.roles.worker.client, ClientKind::Grok);
        assert_eq!(loaded.roles.worker.model, "grok-4.5");
        assert_eq!(config.zellij_session, loaded.zellij_session);
        assert!(root.join(".osanwe/todos").is_dir());
    }

    #[test]
    fn apply_choices_custom_clients() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let mut config = ProjectConfig::defaults_for_repo(root);
        config.roles.planner = RoleChoice::new(ClientKind::Grok, "grok-4.5");
        config.enable_verifier = false;
        config.roles.verifier = None;
        apply_choices(root, &config).unwrap();
        let loaded = load_config(root).unwrap();
        assert_eq!(loaded.roles.planner.client, ClientKind::Grok);
        assert_eq!(loaded.roles.planner.model, "grok-4.5");
        assert!(!loaded.enable_verifier);
    }
}
