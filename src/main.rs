use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
    DefaultTerminal,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

#[derive(Clone, Debug)]
struct Package {
    name: String,
    cask: bool,
    installed: bool,
    outdated: bool,
    detail: String,
}
impl Package {
    fn flag(&self) -> &'static str {
        if self.cask {
            "--cask"
        } else {
            "--formula"
        }
    }
    fn status(&self) -> &'static str {
        if self.outdated {
            "↑"
        } else if self.installed {
            "✓"
        } else {
            "○"
        }
    }
}
fn brew() -> Command {
    let mut c = Command::new(std::env::var_os("DUNK_BREW").unwrap_or_else(|| "brew".into()));
    c.env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_COLOR", "1");
    c
}
fn output(args: &[&str]) -> Result<String> {
    let o = brew()
        .args(args)
        .output()
        .context("Cannot run Homebrew. Install brew and put it on PATH.")?;
    if !o.status.success() {
        bail!(
            "brew {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}
fn catalog() -> Result<Vec<Package>> {
    let mut packages = BTreeMap::new();
    for (cmd, cask) in [("formulae", false), ("casks", true)] {
        for name in output(&[cmd])?.split_whitespace() {
            packages.insert(
                (cask, name.to_string()),
                Package {
                    name: name.into(),
                    cask,
                    installed: false,
                    outdated: false,
                    detail: "Press Enter to load description, versions, dependencies and caveats."
                        .into(),
                },
            );
        }
    }
    let info: Value = serde_json::from_str(&output(&["info", "--json=v2", "--installed"])?)?;
    for (key, cask) in [("formulae", false), ("casks", true)] {
        for p in info[key].as_array().into_iter().flatten() {
            let name = p[if cask { "token" } else { "full_name" }]
                .as_str()
                .or_else(|| p["name"].as_str())
                .context("Missing package name")?;
            let entry = packages.entry((cask, name.into())).or_insert(Package {
                name: name.into(),
                cask,
                installed: true,
                outdated: false,
                detail: String::new(),
            });
            entry.installed = true;
            entry.outdated = p["outdated"].as_bool().unwrap_or(false);
            entry.detail = p["desc"].as_str().unwrap_or("").into();
        }
    }
    Ok(packages.into_values().collect())
}
enum Message {
    Catalog(Result<Vec<Package>>),
    Log(String),
    Done(Result<()>),
    Detail(Result<String>),
}
fn load(tx: Sender<Message>) {
    thread::spawn(move || {
        let _ = tx.send(Message::Catalog(catalog()));
    });
}
#[derive(Clone, Debug)]
enum Operation {
    Package(Package, &'static str),
    UpgradeSelected(Vec<Package>),
    UpgradeAll,
}
impl Operation {
    fn commands(&self) -> Vec<Vec<String>> {
        match self {
            Self::Package(p, action) => {
                vec![vec![(*action).into(), p.flag().into(), p.name.clone()]]
            }
            Self::UpgradeSelected(packages) => [false, true]
                .into_iter()
                .filter_map(|cask| {
                    let selected: Vec<_> = packages.iter().filter(|p| p.cask == cask).collect();
                    if selected.is_empty() {
                        return None;
                    }
                    let mut args = vec!["upgrade".into(), selected[0].flag().into()];
                    args.extend(selected.iter().map(|p| p.name.clone()));
                    Some(args)
                })
                .collect(),
            Self::UpgradeAll => vec![vec!["upgrade".into()]],
        }
    }
    fn confirmation(&self) -> String {
        let scope = match self {
            Self::UpgradeAll => "Upgrade ALL installed formulae and casks eligible under Homebrew's normal upgrade rules. Includes packages hidden by search; ignores marked selections.\n\n".to_string(),
            Self::UpgradeSelected(packages) => format!("Upgrade {} selected package(s), including any marked packages hidden by search.\n\n", packages.len()),
            Self::Package(..) => String::new(),
        };
        format!(
            "{scope}Confirm: {}\n\nPress y to proceed, any other key to cancel.",
            self.commands()
                .iter()
                .map(|args| format!("brew {}", args.join(" ")))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}
fn run_command(tx: &Sender<Message>, args: &[String]) -> Result<()> {
    let mut child = brew()
        .env("HOMEBREW_NO_ASK", "1")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().context("Missing stdout")?;
    let stderr = child.stderr.take().context("Missing stderr")?;
    let readers: Vec<_> = [
        Box::new(stdout) as Box<dyn std::io::Read + Send>,
        Box::new(stderr),
    ]
    .into_iter()
    .map(|stream| {
        let tx = tx.clone();
        thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                match line {
                    Ok(s) => {
                        let _ = tx.send(Message::Log(s));
                    }
                    Err(_) => break,
                }
            }
        })
    })
    .collect();
    let status = child.wait()?;
    for reader in readers {
        let _ = reader.join();
    }
    if !status.success() {
        bail!("brew {} exited with {status}. See the log; privileged casks may require running brew in a terminal.", args.join(" "));
    }
    Ok(())
}
fn operate(tx: Sender<Message>, operation: Operation) {
    thread::spawn(move || {
        let result = operation
            .commands()
            .iter()
            .try_for_each(|args| run_command(&tx, args));
        let _ = tx.send(Message::Done(result));
    });
}
struct App {
    packages: Vec<Package>,
    query: String,
    searching: bool,
    tab: usize,
    selected: usize,
    busy: bool,
    status: String,
    logs: Vec<String>,
    confirm: Option<Operation>,
    marked: BTreeSet<(bool, String)>,
    detail: String,
}
impl App {
    fn new() -> Self {
        Self {
            packages: vec![],
            query: String::new(),
            searching: false,
            tab: 0,
            selected: 0,
            busy: true,
            status: "Loading Homebrew catalog…".into(),
            logs: vec![],
            confirm: None,
            marked: BTreeSet::new(),
            detail: String::new(),
        }
    }
    fn visible(&self) -> Vec<&Package> {
        let q = self.query.to_lowercase();
        self.packages
            .iter()
            .filter(|p| {
                (self.tab == 0 || self.tab == 1 && p.installed || self.tab == 2 && p.outdated)
                    && p.name.to_lowercase().contains(&q)
            })
            .collect()
    }
    fn toggle_mark(&mut self) {
        if self.busy || self.tab != 1 {
            return;
        }
        if let Some(p) = self.visible().get(self.selected) {
            let key = (p.cask, p.name.clone());
            if !self.marked.remove(&key) {
                self.marked.insert(key);
            }
        }
    }
    fn request_upgrade(&mut self, all: bool) {
        if self.busy || self.tab != 1 {
            return;
        }
        if !self.packages.iter().any(|p| p.installed) {
            self.status = "No installed packages to upgrade.".into();
            return;
        }
        if all {
            self.confirm = Some(Operation::UpgradeAll);
        } else {
            let mut packages: Vec<_> = self
                .packages
                .iter()
                .filter(|p| p.installed && self.marked.contains(&(p.cask, p.name.clone())))
                .cloned()
                .collect();
            if packages.is_empty() {
                if let Some(p) = self.visible().get(self.selected) {
                    packages.push((*p).clone());
                }
            }
            if packages.is_empty() {
                self.status = "No selected package to upgrade.".into();
            } else {
                self.confirm = Some(Operation::UpgradeSelected(packages));
            }
        }
    }
    fn draw(&self, f: &mut ratatui::Frame) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(f.area());
        let tabs = ["1: All", "2: Installed", "3: Outdated"];
        let mut navigation = vec![Span::raw(" ")];
        for (index, label) in tabs.into_iter().enumerate() {
            if index > 0 {
                navigation.push(Span::raw(" | "));
            }
            navigation.push(Span::styled(
                label,
                if index == self.tab {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                },
            ));
        }
        f.render_widget(
            Paragraph::new(vec![
                Line::raw(format!(
                    " DUNK   Homebrew package manager   / {}",
                    self.query
                )),
                Line::from(navigation),
                Line::raw(if self.tab == 1 {
                    format!(
                        " Space mark · u upgrade selected · U upgrade all · {} marked",
                        self.marked.len()
                    )
                } else {
                    String::new()
                }),
            ])
            .style(Style::default().fg(Color::Cyan)),
            areas[0],
        );
        let cols = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(areas[1]);
        let visible = self.visible();
        let items: Vec<_> = visible
            .iter()
            .map(|p| {
                ListItem::new(format!(
                    "{} {}  {:<30} {}",
                    if self.tab == 1 {
                        if self.marked.contains(&(p.cask, p.name.clone())) {
                            "[x]"
                        } else {
                            "[ ]"
                        }
                    } else {
                        ""
                    },
                    p.status(),
                    p.name,
                    if p.cask { "cask" } else { "formula" }
                ))
                .style(Style::default().fg(if p.outdated {
                    Color::Yellow
                } else if p.installed {
                    Color::Green
                } else {
                    Color::White
                }))
            })
            .collect();
        let mut state = ListState::default().with_selected(if visible.is_empty() {
            None
        } else {
            Some(self.selected.min(visible.len() - 1))
        });
        f.render_stateful_widget(
            List::new(items)
                .block(Block::bordered().title(format!(" Packages ({}) ", visible.len())))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("›"),
            cols[0],
            &mut state,
        );
        let detail = if let Some(operation) = &self.confirm {
            operation.confirmation()
        } else if !self.detail.is_empty() {
            self.detail.clone()
        } else {
            visible
                .get(self.selected)
                .map(|p| {
                    format!(
                        "{}\n\n{}\n\n✓ installed   ↑ outdated   ○ available",
                        p.name, p.detail
                    )
                })
                .unwrap_or_else(|| "No matching packages.".into())
        };
        f.render_widget(
            Paragraph::new(detail)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().title(" Package details ")),
            cols[1],
        );
        let h = areas[2].height.saturating_sub(2) as usize;
        f.render_widget(
            Paragraph::new(
                self.logs
                    .iter()
                    .rev()
                    .take(h)
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .block(Block::bordered().title(" Brew output ")),
            areas[2],
        );
        f.render_widget(Paragraph::new(format!("{}\n j/k move · / search · 1/2/3 filter · Enter details · i install · u upgrade selected · X remove · r refresh · q quit",self.status)).wrap(Wrap { trim: true }), areas[3]);
    }
}
fn run(terminal: &mut DefaultTerminal) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut app = App::new();
    load(tx.clone());
    loop {
        while let Ok(message) = rx.try_recv() {
            match message {
                Message::Catalog(result) => {
                    app.busy = false;
                    match result {
                        Ok(p) => {
                            app.packages = p;
                            app.marked.retain(|(cask, name)| {
                                app.packages
                                    .iter()
                                    .any(|p| p.installed && p.cask == *cask && p.name == *name)
                            });
                            app.selected = app.selected.min(app.visible().len().saturating_sub(1));
                            app.status = "Ready".into();
                        }
                        Err(e) => app.status = format!("{e:#}"),
                    }
                }
                Message::Log(s) => {
                    app.logs.push(s);
                    if app.logs.len() > 1000 {
                        app.logs.remove(0);
                    }
                }
                Message::Done(result) => {
                    app.status = match result {
                        Ok(()) => "Done. Refreshing installed state…".into(),
                        Err(e) => {
                            app.logs.push(format!("{e:#}"));
                            format!("{e:#}")
                        }
                    };
                    load(tx.clone());
                }
                Message::Detail(result) => {
                    app.busy = false;
                    app.detail = result.unwrap_or_else(|e| format!("{e:#}"));
                    app.status = "Ready".into();
                }
            }
        }
        terminal.draw(|f| app.draw(f))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if !app.busy {
                break;
            }
            app.status = "Wait for the current Brew command to finish before quitting.".into();
            continue;
        }
        if let Some(operation) = app.confirm.take() {
            if key.code == KeyCode::Char('y') {
                app.busy = true;
                app.logs.clear();
                app.detail.clear();
                app.status = "Running confirmed Brew command(s)…".into();
                operate(tx.clone(), operation);
            }
            continue;
        }
        if app.searching {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => app.searching = false,
                KeyCode::Backspace => {
                    app.query.pop();
                }
                KeyCode::Char(c) => app.query.push(c),
                _ => {}
            }
            app.selected = 0;
            app.detail.clear();
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc if !app.busy => break,
            KeyCode::Char('/') => app.searching = true,
            KeyCode::Char(c @ '1'..='3') => {
                app.tab = c as usize - '1' as usize;
                app.selected = 0;
                app.detail.clear();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.selected = (app.selected + 1).min(app.visible().len().saturating_sub(1));
                app.detail.clear();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.selected = app.selected.saturating_sub(1);
                app.detail.clear();
            }
            KeyCode::Char('r') if !app.busy => {
                app.busy = true;
                app.status = "Refreshing…".into();
                load(tx.clone());
            }
            KeyCode::Enter if !app.busy => {
                if let Some(p) = app.visible().get(app.selected).cloned().cloned() {
                    app.busy = true;
                    app.status = "Loading package details…".into();
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let _ = tx.send(Message::Detail(output(&["info", p.flag(), &p.name])));
                    });
                }
            }
            KeyCode::Char(' ') => app.toggle_mark(),
            KeyCode::Char('U') => app.request_upgrade(true),
            KeyCode::Char('u') if app.tab == 1 => app.request_upgrade(false),
            KeyCode::Char(c @ ('i' | 'u' | 'X')) if !app.busy => {
                if let Some(p) = app.visible().get(app.selected).cloned().cloned() {
                    let action = match c {
                        'i' if !p.installed => Some("install"),
                        'u' if p.installed => Some("upgrade"),
                        'X' if p.installed => Some("uninstall"),
                        _ => None,
                    };
                    if let Some(a) = action {
                        app.confirm = Some(Operation::Package(p, a));
                    } else {
                        app.status = "Action does not apply to this package.".into();
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--help" | "-h") => {
            println!("dunk — Mason-inspired Homebrew TUI\n\nUsage: dunk [--check | --help]\n\n--check   Load real Brew catalog and report package counts without opening the UI.\nDUNK_BREW overrides the brew executable.");
            return Ok(());
        }
        Some("--check") => {
            let p = catalog()?;
            println!(
                "{} packages, {} installed, {} outdated",
                p.len(),
                p.iter().filter(|p| p.installed).count(),
                p.iter().filter(|p| p.outdated).count()
            );
            return Ok(());
        }
        Some(arg) => bail!("Unknown argument: {arg}"),
        None => {}
    }
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_distinguish_installed_and_outdated() {
        let mut app = App::new();
        app.packages = vec![
            ("rust", true, false),
            ("rust-analyzer", true, true),
            ("ruby", false, false),
        ]
        .into_iter()
        .map(|(name, installed, outdated)| Package {
            name: name.into(),
            installed,
            outdated,
            cask: false,
            detail: String::new(),
        })
        .collect();
        app.query = "RUST".into();
        assert_eq!(app.visible().len(), 2);
        app.tab = 2;
        assert_eq!(app.visible()[0].name, "rust-analyzer");
        app.tab = 1;
        app.query.clear();
        assert_eq!(app.visible().len(), 2);
    }
    fn installed_app() -> App {
        let mut app = App::new();
        app.busy = false;
        app.tab = 1;
        app.packages = vec![
            Package {
                name: "shared".into(),
                cask: false,
                installed: true,
                outdated: true,
                detail: String::new(),
            },
            Package {
                name: "shared".into(),
                cask: true,
                installed: true,
                outdated: false,
                detail: String::new(),
            },
            Package {
                name: "available".into(),
                cask: false,
                installed: false,
                outdated: false,
                detail: String::new(),
            },
        ];
        app
    }
    #[test]
    fn marked_upgrades_keep_hidden_selections_and_package_kinds() {
        let mut app = installed_app();
        app.toggle_mark();
        app.selected = 1;
        app.toggle_mark();
        app.query = "no matches".into();
        app.request_upgrade(false);
        assert_eq!(
            app.confirm.as_ref().unwrap().commands(),
            vec![
                vec!["upgrade", "--formula", "shared"],
                vec!["upgrade", "--cask", "shared"]
            ]
        );
        assert!(app
            .confirm
            .as_ref()
            .unwrap()
            .confirmation()
            .contains("2 selected"));
        app.request_upgrade(true);
        assert_eq!(
            app.confirm.as_ref().unwrap().commands(),
            vec![vec!["upgrade"]]
        );
        assert!(app
            .confirm
            .as_ref()
            .unwrap()
            .confirmation()
            .contains("ALL installed"));
    }
    #[test]
    fn upgrade_falls_back_to_focus_and_unmark_restores_focus_behavior() {
        let mut app = installed_app();
        app.toggle_mark();
        app.toggle_mark();
        assert!(app.marked.is_empty());
        app.selected = 1;
        app.request_upgrade(false);
        assert_eq!(
            app.confirm.as_ref().unwrap().commands(),
            vec![vec!["upgrade", "--cask", "shared"]]
        );
    }
    #[test]
    fn upgrade_guards_busy_wrong_tab_and_empty_selection() {
        let mut app = installed_app();
        for (busy, tab) in [(true, 1), (false, 0), (false, 2)] {
            app.busy = busy;
            app.tab = tab;
            app.toggle_mark();
            app.request_upgrade(false);
            app.request_upgrade(true);
            assert!(app.confirm.is_none());
            assert!(app.marked.is_empty());
        }
        app.tab = 1;
        app.query = "no matches".into();
        app.request_upgrade(false);
        assert!(app.confirm.is_none());
        app.packages.clear();
        app.request_upgrade(true);
        assert!(app.confirm.is_none());
    }
    #[test]
    fn renders_empty_and_small_terminals() {
        for (w, h) in [(100, 30), (20, 8), (1, 1)] {
            let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            t.draw(|f| App::new().draw(f)).unwrap();
        }
    }
}
