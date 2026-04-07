mod app;
mod git;
mod github;
mod types;
mod ui;

use std::io::stderr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use types::{ActiveAction, ActivePanel};

fn main() -> Result<()> {
    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("no cwd"));

    let repo_root = git::find_repo_root(&cwd).unwrap_or(cwd.clone());

    // ── terminal setup — write TUI to stderr so stdout can be captured ──────
    enable_raw_mode()?;
    let mut stderr = stderr();
    execute!(stderr, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    // ── app init + initial worktree load ────────────────────────────────────
    let mut app = App::new(repo_root.clone());

    match git::list_worktrees(&repo_root) {
        Ok(wts) => {
            app.worktrees = wts;
            app.worktrees_loading = false;
        }
        Err(e) => {
            app.worktrees_loading = false;
            app.worktrees_error = Some(e.to_string());
        }
    }

    // ── main event loop ─────────────────────────────────────────────────────
    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key(&mut app, key.code, key.modifiers);
            }
        }

        if app.should_quit {
            break;
        }
    }

    // ── cleanup ─────────────────────────────────────────────────────────────
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    // Write cd target to stdout — captured by the wt() shell function
    if let Some(ref path) = app.exit_path {
        println!("{path}");
    }

    Ok(())
}

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    // Global: Ctrl-C always quits
    if modifiers == KeyModifiers::CONTROL && code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match app.active_action {
        ActiveAction::NewBranch => handle_new_branch_input(app, code),
        ActiveAction::SyncPr => handle_sync_pr_input(app, code),
        ActiveAction::Delete => handle_delete_input(app, code),
        ActiveAction::None => handle_normal_input(app, code),
    }
}

// ─────────────────────────────── Normal mode ────────────────────────────────

fn handle_normal_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Tab => {
            app.active_panel = match app.active_panel {
                ActivePanel::Sidebar => ActivePanel::Console,
                ActivePanel::Console => ActivePanel::Sidebar,
            };
        }
        KeyCode::Up if app.active_panel == ActivePanel::Sidebar => {
            app.sidebar_index = app.sidebar_index.saturating_sub(1);
        }
        KeyCode::Down if app.active_panel == ActivePanel::Sidebar => {
            let max = app.total_sidebar_items().saturating_sub(1);
            app.sidebar_index = (app.sidebar_index + 1).min(max);
        }
        // Shortcut keys (only active when sidebar is focused)
        KeyCode::Char('n') if app.active_panel == ActivePanel::Sidebar => {
            open_action(app, ActiveAction::NewBranch);
        }
        KeyCode::Char('p') if app.active_panel == ActivePanel::Sidebar => {
            open_action(app, ActiveAction::SyncPr);
        }
        KeyCode::Char('d') if app.active_panel == ActivePanel::Sidebar => {
            open_action(app, ActiveAction::Delete);
        }
        KeyCode::Char('r') if app.active_panel == ActivePanel::Sidebar => {
            refresh_worktrees(app);
        }
        KeyCode::Enter if app.active_panel == ActivePanel::Sidebar => {
            activate_sidebar_selection(app);
        }
        _ => {}
    }
}

fn open_action(app: &mut App, action: ActiveAction) {
    app.overlay_index = 0;
    app.clear_input();
    app.delete_confirming = false;

    if action == ActiveAction::SyncPr {
        app.prs_loading = true;
        app.prs_error = None;
        app.prs.clear();
        match github::list_open_prs(&app.repo_root) {
            Ok(prs) => {
                app.prs = prs;
                app.prs_loading = false;
            }
            Err(e) => {
                app.prs_loading = false;
                app.prs_error = Some(e.to_string());
            }
        }
    }

    app.active_action = action;
}

fn activate_sidebar_selection(app: &mut App) {
    let idx = app.sidebar_index;
    if idx < app::COMMANDS.len() {
        match app::COMMANDS[idx].0 {
            "New Branch" => open_action(app, ActiveAction::NewBranch),
            "Sync GH PR" => open_action(app, ActiveAction::SyncPr),
            "Delete Worktree" => open_action(app, ActiveAction::Delete),
            "Refresh" => refresh_worktrees(app),
            _ => {}
        }
    } else {
        let wt_idx = idx - app::COMMANDS.len();
        if let Some(wt) = app.worktrees.get(wt_idx) {
            let path = wt.path.clone();
            let branch = wt.branch.clone();
            app.log(types::ConsoleMessage::command(format!(
                "Switching to worktree: {branch} → {path}"
            )));
            app.exit_path = Some(path);
            app.should_quit = true;
        }
    }
}

fn refresh_worktrees(app: &mut App) {
    app.log(types::ConsoleMessage::command("Refreshing worktrees..."));
    match git::list_worktrees(&app.repo_root) {
        Ok(wts) => {
            app.worktrees = wts;
            app.worktrees_error = None;
            app.log(types::ConsoleMessage::success("Worktrees refreshed."));
        }
        Err(e) => {
            app.worktrees_error = Some(e.to_string());
            app.log(types::ConsoleMessage::error(e.to_string()));
        }
    }
}

// ──────────────────────────── New Branch overlay ────────────────────────────

fn handle_new_branch_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.clear_input();
        }
        KeyCode::Backspace => {
            app.input_backspace();
        }
        KeyCode::Enter => {
            let branch = app.input_buffer.trim().to_string();
            if branch.is_empty() {
                return;
            }
            app.active_action = ActiveAction::None;
            app.log(types::ConsoleMessage::command(format!(
                "Creating worktree for branch: {branch}"
            )));
            let root = app.repo_root.clone();
            match git::add_worktree(&root, &branch) {
                Ok(lines) => {
                    app.log_lines(lines);
                    refresh_worktrees_silent(app);
                }
                Err(e) => app.log(types::ConsoleMessage::error(e.to_string())),
            }
            app.clear_input();
        }
        KeyCode::Char(c) => {
            app.input_char(c);
        }
        _ => {}
    }
}

// ───────────────────────────── Sync PR overlay ──────────────────────────────

fn handle_sync_pr_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
        }
        KeyCode::Up => {
            app.overlay_index = app.overlay_index.saturating_sub(1);
        }
        KeyCode::Down => {
            let max = app.prs.len().saturating_sub(1);
            app.overlay_index = (app.overlay_index + 1).min(max);
        }
        KeyCode::Enter => {
            if let Some(pr) = app.prs.get(app.overlay_index).cloned() {
                app.active_action = ActiveAction::None;
                app.log(types::ConsoleMessage::command(format!(
                    "Checking out PR #{}: {}",
                    pr.number, pr.title
                )));
                let root = app.repo_root.clone();
                match git::checkout_pr_as_worktree(&root, pr.number, &pr.head_ref_name) {
                    Ok(lines) => {
                        app.log_lines(lines);
                        refresh_worktrees_silent(app);
                    }
                    Err(e) => app.log(types::ConsoleMessage::error(e.to_string())),
                }
            }
        }
        _ => {}
    }
}

// ──────────────────────────── Delete overlay ────────────────────────────────

fn handle_delete_input(app: &mut App, code: KeyCode) {
    let deletable_len = app.deletable_worktrees().len();

    if app.delete_confirming {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let path = app
                    .deletable_worktrees()
                    .get(app.overlay_index)
                    .map(|wt| wt.path.clone());
                app.active_action = ActiveAction::None;
                app.delete_confirming = false;
                if let Some(path) = path {
                    app.log(types::ConsoleMessage::command(format!("Removing worktree: {path}")));
                    let root = app.repo_root.clone();
                    match git::remove_worktree(&root, &path) {
                        Ok(lines) => {
                            app.log_lines(lines);
                            refresh_worktrees_silent(app);
                        }
                        Err(e) => app.log(types::ConsoleMessage::error(e.to_string())),
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.delete_confirming = false;
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
        }
        KeyCode::Up => {
            app.overlay_index = app.overlay_index.saturating_sub(1);
        }
        KeyCode::Down => {
            app.overlay_index = (app.overlay_index + 1).min(deletable_len.saturating_sub(1));
        }
        KeyCode::Enter if deletable_len > 0 => {
            app.delete_confirming = true;
        }
        _ => {}
    }
}

// ────────────────────────────── Helpers ─────────────────────────────────────

fn refresh_worktrees_silent(app: &mut App) {
    if let Ok(wts) = git::list_worktrees(&app.repo_root) {
        app.worktrees = wts;
        app.worktrees_error = None;
    }
}
