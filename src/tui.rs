use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use serde_json::json;

use crate::ipc::IpcClient;
use crate::model::{AgentRecord, ControlOwner, RunManifest, RunStatus};
use crate::store::RunStore;

pub struct App {
    pub manifest: RunManifest,
    pub selected_agent: usize,
    pub status_message: String,
}

impl App {
    #[must_use]
    pub fn new(manifest: RunManifest) -> Self {
        Self {
            manifest,
            selected_agent: 0,
            status_message: "q quit · a approve plan · Enter focus pane · m toggle control".into(),
        }
    }

    #[must_use]
    pub fn selected_agent(&self) -> Option<&AgentRecord> {
        self.manifest.agents.values().nth(self.selected_agent)
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.manifest.agents.len();
        if count == 0 {
            self.selected_agent = 0;
            return;
        }
        let current = isize::try_from(self.selected_agent).unwrap_or_default();
        let maximum = isize::try_from(count.saturating_sub(1)).unwrap_or_default();
        self.selected_agent =
            usize::try_from((current + delta).clamp(0, maximum)).unwrap_or_default();
    }
}

pub async fn run(store: RunStore, run_id: &str) -> anyhow::Result<()> {
    let manifest = store.load(run_id)?;
    let client = IpcClient::new(store.socket_path(run_id), manifest.admin_token.clone());
    let mut app = App::new(manifest);
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut last_refresh = Instant::now() - Duration::from_secs(1);
    loop {
        if last_refresh.elapsed() >= Duration::from_millis(500) {
            match client.call("state.get", json!({})).await {
                Ok(value) => match serde_json::from_value(value) {
                    Ok(manifest) => app.manifest = manifest,
                    Err(error) => app.status_message = format!("state decode error: {error}"),
                },
                Err(error) => app.status_message = format!("daemon unavailable: {error}"),
            }
            last_refresh = Instant::now();
        }
        terminal.draw(|frame| render(frame, &app))?;

        if event::poll(Duration::from_millis(100))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                KeyCode::Char('r') => last_refresh = Instant::now() - Duration::from_secs(1),
                KeyCode::Char('a') => {
                    if app.manifest.status == RunStatus::PlanReview {
                        match client.call("plan.approve", json!({})).await {
                            Ok(_) => {
                                app.status_message = "plan approved; worker is starting".into()
                            }
                            Err(error) => app.status_message = format!("approval failed: {error}"),
                        }
                    } else {
                        app.status_message = "a plan can only be approved in PLAN REVIEW".into();
                    }
                }
                KeyCode::Enter => {
                    if let Some(agent) = app.selected_agent() {
                        let agent_id = agent.id.clone();
                        let _ = client
                            .call(
                                "control.set",
                                json!({"agent_id": agent_id, "owner": "user"}),
                            )
                            .await;
                        match client
                            .call("pane.focus", json!({"agent_id": agent_id}))
                            .await
                        {
                            Ok(_) => {
                                app.status_message = "pane focused; control belongs to user".into()
                            }
                            Err(error) => app.status_message = format!("focus failed: {error}"),
                        }
                    }
                }
                KeyCode::Char('m') => {
                    if let Some(agent) = app.selected_agent() {
                        let agent_id = agent.id.clone();
                        let next = if agent.control_owner == ControlOwner::User {
                            "orchestrator"
                        } else {
                            "user"
                        };
                        match client
                            .call("control.set", json!({"agent_id": agent_id, "owner": next}))
                            .await
                        {
                            Ok(_) => app.status_message = format!("{agent_id} control: {next}"),
                            Err(error) => {
                                app.status_message = format!("control change failed: {error}")
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    drop(terminal);
    drop(guard);
    Ok(())
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());
    render_header(frame, app, root[0]);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(root[1]);
    render_workflow(frame, app, body[0]);
    render_agents(frame, app, body[1]);
    frame.render_widget(
        Paragraph::new(app.status_message.as_str())
            .block(Block::default().borders(Borders::ALL).title("Actions"))
            .wrap(Wrap { trim: true }),
        root[2],
    );
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = Line::from(vec![
        Span::styled(" Osanwe ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "run {} · {:?}",
            short_id(&app.manifest.run_id),
            app.manifest.status
        )),
    ]);
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn render_workflow(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let plan = app
        .manifest
        .plan
        .as_ref()
        .map_or("No plan submitted", |plan| plan.task_summary.as_str());
    let checks = if app.manifest.check_results.is_empty() {
        "No deterministic check results".into()
    } else {
        app.manifest
            .check_results
            .iter()
            .map(|result| format!("{} {}", if result.passed { "✓" } else { "✗" }, result.id))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let attention = if app.manifest.attention.is_empty() {
        "None".into()
    } else {
        app.manifest.attention.join("\n")
    };
    let text = format!(
        "Task\n{}\n\nPlan\n{}\n\nChecks\n{}\n\nAttention\n{}",
        app.manifest.task, plan, checks, attention
    );
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Workflow"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_agents(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = app
        .manifest
        .agents
        .values()
        .enumerate()
        .map(|(index, agent)| {
            let marker = if index == app.selected_agent {
                ">"
            } else {
                " "
            };
            ListItem::new(Line::from(format!(
                "{marker} {} {:?} {:?} control={:?}",
                agent.id, agent.role, agent.state, agent.control_owner
            )))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title("Agents")),
        area,
    );
}

fn short_id(value: &str) -> &str {
    &value[..value.len().min(12)]
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn overview_renders_run_and_agent() {
        let mut manifest = RunManifest::new_for_test("implement feature");
        manifest.agents.insert(
            "planner".into(),
            AgentRecord::test_agent("planner", crate::model::AgentRole::Planner),
        );
        let app = App::new(manifest);
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Osanwe"));
        assert!(rendered.contains("planner"));
    }
}
