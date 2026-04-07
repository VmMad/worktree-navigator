mod app;
mod git;
mod github;
mod pty;
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

    enable_raw_mode()?;
    let mut stderr = stderr();
    execute!(stderr, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(repo_root.clone());

    // Set initial console size (will be updated on first Resize event)
    let size = terminal.size()?;
    let console_cols = size.width * 70 / 100 - 2;
    let console_rows = size.height.saturating_sub(2);
    app.console_size = (console_cols, console_rows);

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

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => handle_key(&mut app, key.code, key.modifiers),
                Event::Resize(cols, rows) => {
                    let console_cols = cols * 70 / 100 - 2;
                    let console_rows = rows.saturating_sub(2);
                    app.resize_ptys(console_cols, console_rows);
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Some(ref path) = app.exit_path {
        println!("{path}");
    }

    Ok(())
}

// ─────────────────────────────── Key dispatch ───────────────────────────────

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+C always quits
    if ctrl && code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    // Ctrl+Space toggles sidebar ↔ console focus from anywhere
    if ctrl && code == KeyCode::Char(' ') {
        if app.active_action == ActiveAction::None {
            app.active_panel = match app.active_panel {
                ActivePanel::Sidebar => ActivePanel::Console,
                ActivePanel::Console => ActivePanel::Sidebar,
            };
        }
        return;
    }

    match app.active_action {
        ActiveAction::NewBranch => handle_new_branch_input(app, code),
        ActiveAction::SyncPr => handle_sync_pr_input(app, code),
        ActiveAction::Delete => handle_delete_input(app, code),
        ActiveAction::None => match app.active_panel {
            ActivePanel::Console => handle_console_key(app, code, modifiers),
            ActivePanel::Sidebar => handle_sidebar_key(app, code),
        },
    }
}

// ─────────────────────────── Console: forward to PTY ────────────────────────

fn handle_console_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if let Some(bytes) = key_to_bytes(code, modifiers) {
        app.send_to_pty(&bytes);
    }
}

fn key_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);

    match code {
        KeyCode::Char(c) if ctrl => {
            // Ctrl+letter → byte 1–26
            let byte = (c.to_ascii_uppercase() as u8).wrapping_sub(b'@');
            Some(vec![byte])
        }
        KeyCode::Char(c) if alt => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            let mut v = vec![0x1b];
            v.extend_from_slice(s.as_bytes());
            Some(v)
        }
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::F(1) => Some(b"\x1bOP".to_vec()),
        KeyCode::F(2) => Some(b"\x1bOQ".to_vec()),
        KeyCode::F(3) => Some(b"\x1bOR".to_vec()),
        KeyCode::F(4) => Some(b"\x1bOS".to_vec()),
        KeyCode::F(n) => Some(format!("\x1b[{}~", n + 10).into_bytes()),
        _ => None,
    }
}

// ──────────────────────────────── Sidebar nav ────────────────────────────────

fn handle_sidebar_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') => {
            app.should_quit = true;
        }
        KeyCode::Up => {
            app.sidebar_index = app.sidebar_index.saturating_sub(1);
        }
        KeyCode::Down => {
            let max = app.total_sidebar_items().saturating_sub(1);
            app.sidebar_index = (app.sidebar_index + 1).min(max);
        }
        KeyCode::Right => {
            // Move focus to console without switching worktree
            app.active_panel = ActivePanel::Console;
        }
        KeyCode::Char('n') => open_action(app, ActiveAction::NewBranch),
        KeyCode::Char('p') => open_action(app, ActiveAction::SyncPr),
        KeyCode::Char('d') => open_action(app, ActiveAction::Delete),
        KeyCode::Char('r') => refresh_worktrees(app),
        KeyCode::Enter => activate_sidebar_selection(app),
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
            app.open_shell(&path);
        }
    }
}

fn refresh_worktrees(app: &mut App) {
    match git::list_worktrees(&app.repo_root) {
        Ok(wts) => {
            app.worktrees = wts;
            app.worktrees_error = None;
        }
        Err(e) => {
            app.worktrees_error = Some(e.to_string());
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
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Enter => {
            let branch = app.input_buffer.trim().to_string();
            if branch.is_empty() {
                return;
            }
            app.active_action = ActiveAction::None;
            let root = app.repo_root.clone();
            if let Ok(lines) = git::add_worktree(&root, &branch) {
                // After creating, open a shell in the new worktree if successful
                let created = lines.iter().any(|l| l.starts_with("✓"));
                if created {
                    refresh_worktrees(app);
                    // Find the newly created worktree path
                    if let Some(wt) = app.worktrees.iter().find(|w| w.branch == branch) {
                        let path = wt.path.clone();
                        app.open_shell(&path);
                    }
                }
            }
            app.clear_input();
        }
        KeyCode::Char(c) => app.input_char(c),
        _ => {}
    }
}

// ───────────────────────────── Sync PR overlay ──────────────────────────────

fn handle_sync_pr_input(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.active_action = ActiveAction::None,
        KeyCode::Up => app.overlay_index = app.overlay_index.saturating_sub(1),
        KeyCode::Down => {
            app.overlay_index = (app.overlay_index + 1).min(app.prs.len().saturating_sub(1));
        }
        KeyCode::Enter => {
            if let Some(pr) = app.prs.get(app.overlay_index).cloned() {
                app.active_action = ActiveAction::None;
                let root = app.repo_root.clone();
                if let Ok(lines) = git::checkout_pr_as_worktree(&root, pr.number, &pr.head_ref_name) {
                    let created = lines.iter().any(|l| l.starts_with("✓"));
                    if created {
                        refresh_worktrees(app);
                        if let Some(wt) = app.worktrees.iter().find(|w| w.branch == pr.head_ref_name) {
                            let path = wt.path.clone();
                            app.open_shell(&path);
                        }
                    }
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
                    // Close PTY session for this worktree if open
                    app.pty_sessions.remove(&path);
                    if app.active_pty_path.as_deref() == Some(&path) {
                        app.active_pty_path = None;
                        app.active_panel = ActivePanel::Sidebar;
                    }
                    let root = app.repo_root.clone();
                    let _ = git::remove_worktree(&root, &path);
                    refresh_worktrees(app);
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
        KeyCode::Esc => app.active_action = ActiveAction::None,
        KeyCode::Up => app.overlay_index = app.overlay_index.saturating_sub(1),
        KeyCode::Down => {
            app.overlay_index = (app.overlay_index + 1).min(deletable_len.saturating_sub(1));
        }
        KeyCode::Enter if deletable_len > 0 => app.delete_confirming = true,
        _ => {}
    }
}

