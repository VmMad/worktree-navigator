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
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
        MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use types::ActiveAction;

fn main() -> Result<()> {
    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("no cwd"));

    let repo_root = git::find_repo_root(&cwd).unwrap_or(cwd);

    enable_raw_mode()?;
    let mut stderr = stderr();
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

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

    loop {
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Execute pending sync after the loading frame has been rendered.
        if app.sync_pending {
            app.sync_pending = false;
            let wt = app.worktrees.get(app.sync_selected_idx).cloned();
            if let Some(wt) = wt {
                let root = app.repo_root.clone();
                let (fetch_ok, result) = git::sync_one_worktree(&root, &wt);
                app.sync_fetch_ok = fetch_ok;
                app.sync_results = vec![result];
                app.sync_loading = false;
                refresh_worktrees(&mut app);
            }
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => handle_key(&mut app, key.code, key.modifiers),
                Event::Mouse(m) => handle_mouse(&mut app, m.kind, m.row),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    if let Some(ref path) = app.exit_path {
        println!("{path}");
    }

    Ok(())
}

// ─────────────────────────────── Event handlers ─────────────────────────────

fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    match app.active_action {
        ActiveAction::NewBranch => handle_new_branch_key(app, code),
        ActiveAction::SyncPr => handle_sync_pr_key(app, code),
        ActiveAction::SyncTrees => handle_sync_trees_key(app, code),
        ActiveAction::Delete => handle_delete_key(app, code),
        ActiveAction::None => handle_nav_key(app, code),
    }
}

fn handle_mouse(app: &mut App, kind: MouseEventKind, row: u16) {
    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();

    // While a blocking overlay is open (not sync-select), ignore mouse.
    if app.active_action != ActiveAction::None && !sync_select {
        return;
    }

    match kind {
        MouseEventKind::Moved => {
            let target = app.row_to_item(row).and_then(|idx| {
                // In sync-select mode only highlight worktree rows, not commands.
                if sync_select && idx < app::COMMANDS.len() { None } else { Some(row) }
            });
            app.hovered_row = target;
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(idx) = app.row_to_item(row) {
                if sync_select {
                    // Only worktree rows trigger sync.
                    if idx >= app::COMMANDS.len() {
                        app.sync_selected_idx = idx - app::COMMANDS.len();
                        app.sync_loading = true;
                        app.sync_pending = true;
                    }
                } else {
                    app.selected_index = idx;
                    activate(app);
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if sync_select {
                app.sync_selected_idx = app.sync_selected_idx.saturating_sub(1);
            } else {
                app.selected_index = app.selected_index.saturating_sub(1);
            }
        }
        MouseEventKind::ScrollDown => {
            if sync_select {
                let max = app.worktrees.len().saturating_sub(1);
                app.sync_selected_idx = (app.sync_selected_idx + 1).min(max);
            } else {
                let max = app.total_items().saturating_sub(1);
                app.selected_index = (app.selected_index + 1).min(max);
            }
        }
        _ => {}
    }
}

fn handle_nav_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected_index = app.selected_index.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.total_items().saturating_sub(1);
            app.selected_index = (app.selected_index + 1).min(max);
        }
        KeyCode::Char('n') => open_action(app, ActiveAction::NewBranch),
        KeyCode::Char('p') => open_action(app, ActiveAction::SyncPr),
        KeyCode::Char('d') => open_action(app, ActiveAction::Delete),
        KeyCode::Char('s') => open_action(app, ActiveAction::SyncTrees),
        KeyCode::Char('r') => refresh_worktrees(app),
        KeyCode::Enter => activate(app),
        _ => {}
    }
}

fn activate(app: &mut App) {
    let idx = app.selected_index;
    if idx < app::COMMANDS.len() {
        match app::COMMANDS[idx].0 {
            "New Branch" => open_action(app, ActiveAction::NewBranch),
            "Sync GH PR" => open_action(app, ActiveAction::SyncPr),
            "Delete Worktree" => open_action(app, ActiveAction::Delete),
            "Sync Trees" => open_action(app, ActiveAction::SyncTrees),
            "Refresh List" => refresh_worktrees(app),
            _ => {}
        }
    } else {
        let wt_idx = idx - app::COMMANDS.len();
        if let Some(wt) = app.worktrees.get(wt_idx) {
            app.exit_path = Some(wt.path.clone());
            app.should_quit = true;
        }
    }
}

fn open_action(app: &mut App, action: ActiveAction) {
    app.overlay_index = 0;
    app.clear_input();
    app.delete_confirming = false;
    app.overlay_error = None;

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

    if action == ActiveAction::SyncTrees {
        app.sync_results.clear();
        app.sync_loading = false;
        app.sync_pending = false;
        // Pre-select the main (first) worktree
        app.sync_selected_idx = app.worktrees.iter().position(|w| w.is_main).unwrap_or(0);
    }

    app.active_action = action;
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

// ────────────────────────── Overlay key handlers ────────────────────────────

fn handle_sync_trees_key(app: &mut App, code: KeyCode) {
    // While loading, ignore all keys.
    if app.sync_loading {
        return;
    }

    // Results phase: any key closes.
    if !app.sync_results.is_empty() {
        match code {
            KeyCode::Esc | KeyCode::Enter => {
                app.active_action = ActiveAction::None;
                app.sync_results.clear();
            }
            _ => {}
        }
        return;
    }

    // Select phase.
    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.sync_selected_idx = app.sync_selected_idx.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.worktrees.len().saturating_sub(1);
            app.sync_selected_idx = (app.sync_selected_idx + 1).min(max);
        }
        KeyCode::Enter => {
            if !app.worktrees.is_empty() {
                app.sync_loading = true;
                app.sync_pending = true;
            }
        }
        _ => {}
    }
}

fn handle_new_branch_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.clear_input();
            app.overlay_error = None;
        }
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Enter => {
            let branch = app.input_buffer.trim().to_string();
            if branch.is_empty() {
                return;
            }
            let root = app.repo_root.clone();
            match git::add_worktree(&root, &branch) {
                Ok(_) => {
                    app.active_action = ActiveAction::None;
                    app.clear_input();
                    refresh_worktrees(app);
                    if let Some(wt) = app.worktrees.iter().find(|w| w.branch == branch) {
                        app.exit_path = Some(wt.path.clone());
                        app.should_quit = true;
                    }
                }
                Err(e) => {
                    app.overlay_error = Some(format!("Failed to create branch: {e}"));
                }
            }
        }
        KeyCode::Char(c) => app.input_char(c),
        _ => {}
    }
}

fn handle_sync_pr_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.overlay_error = None;
        }
        KeyCode::Up => app.overlay_index = app.overlay_index.saturating_sub(1),
        KeyCode::Down => {
            app.overlay_index = (app.overlay_index + 1).min(app.prs.len().saturating_sub(1));
        }
        KeyCode::Enter => {
            if let Some(pr) = app.prs.get(app.overlay_index).cloned() {
                let root = app.repo_root.clone();
                match git::checkout_pr_as_worktree(&root, pr.number, &pr.head_ref_name) {
                    Ok(_) => {
                        app.active_action = ActiveAction::None;
                        refresh_worktrees(app);
                        if let Some(wt) =
                            app.worktrees.iter().find(|w| w.branch == pr.head_ref_name)
                        {
                            app.exit_path = Some(wt.path.clone());
                            app.should_quit = true;
                        }
                    }
                    Err(e) => {
                        app.overlay_error =
                            Some(format!("Failed to sync PR #{}: {e}", pr.number));
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_delete_key(app: &mut App, code: KeyCode) {
    let deletable_len = app.deletable_worktrees().len();

    if app.delete_confirming {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let path = app
                    .deletable_worktrees()
                    .get(app.overlay_index)
                    .map(|wt| wt.path.clone());
                app.delete_confirming = false;
                if let Some(path) = path {
                    let root = app.repo_root.clone();
                    match git::remove_worktree(&root, &path) {
                        Ok(_) => {
                            app.active_action = ActiveAction::None;
                            app.overlay_error = None;
                            refresh_worktrees(app);
                        }
                        Err(e) => {
                            app.active_action = ActiveAction::None;
                            app.overlay_error =
                                Some(format!("Failed to delete worktree: {e}"));
                        }
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
            app.overlay_error = None;
        }
        KeyCode::Up => app.overlay_index = app.overlay_index.saturating_sub(1),
        KeyCode::Down => {
            app.overlay_index = (app.overlay_index + 1).min(deletable_len.saturating_sub(1));
        }
        KeyCode::Enter if deletable_len > 0 => app.delete_confirming = true,
        _ => {}
    }
}
