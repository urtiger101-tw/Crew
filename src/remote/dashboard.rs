//! A federated selector owns no PTYs. Enter attaches using the selected server.
use super::{AttachTarget, attach, store, transport, view};
use anyhow::{Context, Result, bail};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

struct Screen {
    active: bool,
}
impl Screen {
    fn enter() -> Result<Self> {
        let mut screen = Self { active: false };
        screen.resume()?;
        Ok(screen)
    }
    fn resume(&mut self) -> Result<()> {
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide
        ) {
            let _ = terminal::disable_raw_mode();
            return Err(error.into());
        }
        self.active = true;
        Ok(())
    }
    fn suspend(&mut self) -> Result<()> {
        if self.active {
            self.active = false;
            let a = execute!(
                std::io::stdout(),
                cursor::Show,
                terminal::LeaveAlternateScreen
            );
            let b = terminal::disable_raw_mode();
            a?;
            b?;
        }
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    machine: String,
    epoch: String,
    workspace: String,
    name: String,
}

fn clean(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|&c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn rows(views: &[Value]) -> Vec<(String, Option<Target>)> {
    let mut rows = Vec::new();
    for machine in views {
        let id = text(&machine["machine_id"]);
        let label = text(&machine["label"]);
        let state = text(&machine["state"]);
        rows.push((format!("{label} [{id}] - {state}"), None));
        if let Some(error) = machine.get("error").and_then(Value::as_str) {
            rows.push((format!("  {error}"), None));
        }
        if state != "reachable" {
            continue;
        }
        let epoch = text(&machine["data"]["epoch"]);
        if epoch.is_empty() {
            rows.push((
                "  Attachment unavailable: daemon identity missing; update Crew.".into(),
                None,
            ));
        }
        if let Some(workspaces) = machine["data"]["workspaces"].as_array() {
            for workspace in workspaces {
                let wid = text(&workspace["workspace_id"]);
                let name = text(&workspace["name"]);
                rows.push((
                    format!("  {wid}  {name}"),
                    (!epoch.is_empty()).then(|| Target {
                        machine: id.into(),
                        epoch: epoch.into(),
                        workspace: wid.into(),
                        name: name.into(),
                    }),
                ));
                if let Some(agents) = machine["data"]["agents"].as_array() {
                    for agent in agents.iter().filter(|a| a["workspace_id"] == wid) {
                        let kind = text(&agent["agent"]["agent"]);
                        let label = agent["agent"]["name"].as_str().unwrap_or(kind);
                        rows.push((
                            format!(
                                "      {}  {}  {}",
                                text(&agent["pane_id"]),
                                label,
                                text(&agent["status"])
                            ),
                            None,
                        ));
                    }
                }
                if rows.len() >= 4000 {
                    rows.push((
                        "Additional rows omitted; use federation list --json.".into(),
                        None,
                    ));
                    return rows;
                }
            }
        }
    }
    rows
}

struct Checks {
    budget: transport::Budget,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for Checks {
    fn drop(&mut self) {
        self.budget.cancel();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn start_refresh(
    machines: &[store::Machine],
    duration: u64,
    tx: &mpsc::Sender<Value>,
) -> (usize, Checks) {
    let mut checks = Checks {
        budget: transport::Budget::new(duration),
        workers: Vec::new(),
    };
    let local_tx = tx.clone();
    let budget = checks.budget.clone();
    checks.workers.push(std::thread::spawn(move || {
        let _ = local_tx.send(view(
            "local",
            "Local",
            true,
            transport::local(&[], true, budget),
        ));
    }));
    for machine in machines.iter().cloned() {
        let tx = tx.clone();
        let budget = checks.budget.clone();
        checks.workers.push(std::thread::spawn(move || {
            let result = if machine.enabled {
                view(
                    &machine.id,
                    &machine.label,
                    false,
                    transport::remote(&machine, &[], true, budget),
                )
            } else {
                json!({"machine_id":machine.id,"label":machine.label,"state":"disabled"})
            };
            let _ = tx.send(result);
        }));
    }
    (machines.len() + 1, checks)
}

fn initial_views(machines: &[store::Machine]) -> Vec<Value> {
    std::iter::once(json!({"machine_id":"local","label":"Local","state":"checking"}))
        .chain(machines.iter().map(|m| json!({"machine_id":m.id,"label":m.label,"state":if m.enabled {"checking"} else {"disabled"}})))
        .collect()
}

pub fn run(duration: u64) -> Result<i32> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("federation watch requires a terminal; use crew federation list --json in scripts");
    }
    let path = store::path()?;
    let mut machines = store::load(&path)?.machines;
    let mut screen = Screen::enter()?;
    let (tx, rx) = mpsc::channel();
    let (mut pending, mut _checks) = start_refresh(&machines, duration, &tx);
    let mut views = initial_views(&machines);
    let mut selected: Option<Target> = None;
    let mut message = "Enter attaches to the selected workspace; no local fallback.".to_string();
    let mut updated = Instant::now();
    loop {
        for next in rx.try_iter() {
            if let Some(old) = views
                .iter_mut()
                .find(|old| old["machine_id"] == next["machine_id"])
            {
                *old = next;
            }
            pending = pending.saturating_sub(1);
            if pending == 0 {
                updated = Instant::now();
            }
        }
        let rows = rows(&views);
        let targets: Vec<Target> = rows
            .iter()
            .filter_map(|(_, target)| target.clone())
            .collect();
        let selected_index = selected
            .as_ref()
            .and_then(|s| targets.iter().position(|t| t == s));
        if selected.is_none() {
            selected = targets.first().cloned();
        }
        let selected_row = rows
            .iter()
            .position(|(_, t)| t.is_some() && *t == selected)
            .unwrap_or(0);
        let (width, height) = terminal::size().unwrap_or((100, 30));
        let available = height.saturating_sub(5) as usize;
        let offset = selected_row.saturating_sub(available.saturating_sub(1));
        let mut output = std::io::stdout();
        execute!(
            output,
            cursor::MoveTo(0, 0),
            terminal::Clear(terminal::ClearType::All)
        )?;
        writeln!(
            output,
            "{}\r",
            clean(
                "Crew federation | Up/Down select | Enter attach | r refresh | q quit",
                width.saturating_sub(1) as usize
            )
        )?;
        writeln!(
            output,
            "{}\r",
            clean(
                &format!("{pending} connection checks pending. {message}"),
                width.saturating_sub(1) as usize
            )
        )?;
        writeln!(output, "\r")?;
        for (line, target) in rows.iter().skip(offset).take(available) {
            let prefix = if target.is_some() && *target == selected {
                "> "
            } else {
                "  "
            };
            writeln!(
                output,
                "{}\r",
                clean(&format!("{prefix}{line}"), width.saturating_sub(1) as usize)
            )?;
        }
        output.flush()?;
        let mut refresh = pending == 0 && updated.elapsed() >= Duration::from_secs(15);
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(0),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(0);
                    }
                    KeyCode::Up if !targets.is_empty() => {
                        selected = Some(
                            targets[selected_index.map_or(targets.len() - 1, |i| {
                                (i + targets.len() - 1) % targets.len()
                            })]
                            .clone(),
                        )
                    }
                    KeyCode::Down if !targets.is_empty() => {
                        selected = Some(
                            targets[selected_index.map_or(0, |i| (i + 1) % targets.len())].clone(),
                        )
                    }
                    KeyCode::Char('r') => {
                        if pending == 0 {
                            refresh = true;
                            message = "Refreshing each machine independently.".into();
                        } else {
                            message = "Checks are still running; q remains available.".into();
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(target) =
                            selected.clone().filter(|target| targets.contains(target))
                        {
                            // Re-read the saved profile before acting, so disabling/removal wins.
                            let catalog = store::load(&path)?;
                            let machine = if target.machine == "local" {
                                None
                            } else {
                                Some(catalog.select(&target.machine, true)?.clone())
                            };
                            screen.suspend()?;
                            let outcome = attach(
                                machine.as_ref(),
                                AttachTarget::Snapshot {
                                    workspace: &target.workspace,
                                    epoch: &target.epoch,
                                },
                            );
                            screen.resume()?;
                            message = match outcome {
                                Ok(0) => {
                                    format!("Returned from {} / {}.", target.machine, target.name)
                                }
                                Ok(code) => format!(
                                    "Attach failed with exit {code}; no other machine was selected."
                                ),
                                Err(error) => format!("Attach failed: {error}"),
                            };
                            refresh = pending == 0;
                        } else {
                            message = "No reachable workspace is selected. Press r to check again."
                                .into();
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if refresh {
            machines = store::load(&path)
                .context("reload saved machines")?
                .machines;
            views = initial_views(&machines);
            (pending, _checks) = start_refresh(&machines, duration, &tx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_rows_cannot_be_selected_and_terminal_controls_are_removed() {
        let rows = rows(&[
            json!({"machine_id":"m1","label":"Live","state":"reachable","data":{"epoch":"live-epoch","workspaces":[{"workspace_id":"w1","name":"work"}],"agents":[{"workspace_id":"w1","pane_id":"p1","status":"working","agent":{"agent":"codex"}}]}}),
            json!({"machine_id":"m2","label":"Offline","state":"unavailable","data":{"workspaces":[{"workspace_id":"w1","name":"stale"}]}}),
        ]);
        let selected: Vec<_> = rows.iter().filter_map(|(_, t)| t.as_ref()).collect();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].machine, "m1");
        assert!(rows.iter().any(|(s, _)| s.contains("codex")));
        assert_eq!(clean("中文abc\x1b\r", 5), "中文a");
    }

    #[test]
    fn legacy_summary_without_daemon_identity_cannot_be_attached() {
        let rows = rows(&[json!({"machine_id":"old","state":"reachable","data":{
            "workspaces":[{"workspace_id":"w1","name":"work"}],"agents":[]
        }})]);
        assert!(rows.iter().all(|(_, target)| target.is_none()));
        assert!(
            rows.iter()
                .any(|(line, _)| line.contains("daemon identity missing"))
        );
    }
}
