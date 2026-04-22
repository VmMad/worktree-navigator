mod app;
mod git;
mod types;
mod ui;
mod update;

use std::io::stderr;
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::App;
use types::{ActiveAction, CloneEvent, CopySecretsPhase};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--update") {
        update::run_manual_update()?;
        return Ok(());
    }

    if !args.iter().any(|a| a == "--mark-tree") {
        match update::maybe_prompt_for_update()? {
            update::StartupUpdateAction::Continue => {}
            update::StartupUpdateAction::ExitAfterUpdateFlow => return Ok(()),
        }
    }

    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("no cwd"));

    let mark_tree = args.iter().any(|a| a == "--mark-tree");

    if mark_tree {
        git::create_workspace_marker(&cwd)?;
    }

    let repo_root_opt = git::find_repo_root(&cwd);
    let workspace_root_opt = if repo_root_opt.is_none() {
        git::find_workspace_root(&cwd)
    } else {
        None
    };

    let no_repo = repo_root_opt.is_none() && workspace_root_opt.is_none();
    let repo_root = repo_root_opt
        .or_else(|| workspace_root_opt.clone())
        .unwrap_or_else(|| cwd.clone());

    enable_raw_mode()?;
    let mut stderr = stderr();
    execute!(
        stderr,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    let mut mouse_capture_enabled = true;

    let mut app = App::new(repo_root.clone());

    if no_repo {
        app.no_repo = true;
        app.worktrees_loading = false;
        app.active_action = ActiveAction::CloneRepo;
    } else if workspace_root_opt.is_some() {
        app.is_workspace = true;
        match git::list_workspace_worktrees(&repo_root) {
            Ok(wts) => {
                let current_idx = wts.iter().position(|w| w.is_current).unwrap_or(0);
                app.worktrees = wts;
                app.worktrees_loading = false;
                app.selected_index = app::COMMANDS.len() + current_idx;
            }
            Err(e) => {
                app.worktrees_loading = false;
                app.worktrees_error = Some(e.to_string());
            }
        }
    } else {
        match git::list_worktrees(&repo_root) {
            Ok(wts) => {
                let current_idx = wts.iter().position(|w| w.is_current).unwrap_or(0);
                app.worktrees = wts;
                app.worktrees_loading = false;
                if !app.worktrees.is_empty() {
                    app.selected_index = app::COMMANDS.len() + current_idx;
                }
            }
            Err(e) => {
                app.worktrees_loading = false;
                app.worktrees_error = Some(e.to_string());
            }
        }
    }

    loop {
        poll_clone_updates(&mut app);
        if app.clone_loading {
            app.advance_clone_animation();
        }
        let wants_mouse_capture = app.active_action != ActiveAction::CloneRepo;
        if wants_mouse_capture != mouse_capture_enabled {
            if wants_mouse_capture {
                execute!(terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                execute!(terminal.backend_mut(), DisableMouseCapture)?;
            }
            mouse_capture_enabled = wants_mouse_capture;
        }
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

        // Execute pending PR sync after the loading frame has been rendered.
        if let Some(pr_number) = app.sync_pr_pending.take() {
            let root = app.repo_root.clone();
            match git::checkout_pr_as_worktree(&root, pr_number) {
                Ok((_, dest)) => {
                    app.sync_pr_loading = false;
                    app.active_action = ActiveAction::None;
                    app.clear_input();
                    refresh_worktrees(&mut app);
                    app.exit_path = Some(dest.to_string_lossy().into_owned());
                    app.should_quit = true;
                }
                Err(e) => {
                    app.sync_pr_loading = false;
                    app.overlay_error = Some(format!("Failed to sync PR #{pr_number}: {e}"));
                }
            }
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => handle_key(&mut app, key.code, key.modifiers),
                Event::Paste(text) => handle_paste(&mut app, &text),
                Event::Mouse(m) => handle_mouse(&mut app, m.kind, m.column, m.row),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    disable_raw_mode()?;
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
        ActiveAction::CopySecrets => handle_copy_secrets_key(app, code),
        ActiveAction::CloneRepo => handle_clone_key(app, code),
        ActiveAction::None => handle_nav_key(app, code, modifiers),
    }
}

fn handle_mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    if matches!(kind, MouseEventKind::Down(MouseButton::Left))
        && app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase == CopySecretsPhase::ConfirmOverwrite
    {
        handle_copy_secrets_confirm_click(app, column, row);
        return;
    }

    let sync_select = app.active_action == ActiveAction::SyncTrees
        && !app.sync_loading
        && app.sync_results.is_empty();
    let delete_select = app.active_action == ActiveAction::Delete && !app.delete_confirming;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite;

    // While a blocking overlay is open (not inline-select), ignore mouse.
    if app.active_action != ActiveAction::None && !sync_select && !delete_select && !copy_select {
        return;
    }

    match kind {
        MouseEventKind::Moved => {
            let target = app.row_to_item(row).and_then(|idx| {
                // In inline-select mode only highlight worktree rows, not commands.
                if (sync_select || delete_select || copy_select) && idx < app::COMMANDS.len() {
                    None
                } else {
                    Some(row)
                }
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
                } else if delete_select {
                    // Only deletable worktree rows trigger delete confirmation.
                    if idx >= app::COMMANDS.len() {
                        let wt_idx = idx - app::COMMANDS.len();
                        if app
                            .worktrees
                            .get(wt_idx)
                            .map(|wt| !wt.is_main && !wt.is_current)
                            .unwrap_or(false)
                        {
                            app.overlay_index = delete_overlay_index_for_worktree(app, wt_idx);
                            app.delete_confirming = true;
                        }
                    }
                } else if copy_select {
                    if idx >= app::COMMANDS.len() {
                        handle_copy_secrets_select(app, idx - app::COMMANDS.len());
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
            } else if copy_select {
                let next = copy_selection_idx(app).saturating_sub(1);
                set_copy_selection_idx(app, next);
            } else if delete_select {
                app.overlay_index = app.overlay_index.saturating_sub(1);
            } else if app.selected_index == 0 {
                app.selected_index = app.total_items().saturating_sub(1);
            } else {
                app.selected_index -= 1;
            }
        }
        MouseEventKind::ScrollDown => {
            if sync_select {
                let max = app.worktrees.len().saturating_sub(1);
                app.sync_selected_idx = (app.sync_selected_idx + 1).min(max);
            } else if copy_select {
                let max = app.worktrees.len().saturating_sub(1);
                let next = (copy_selection_idx(app) + 1).min(max);
                set_copy_selection_idx(app, next);
            } else if delete_select {
                let max = app.deletable_worktrees().len().saturating_sub(1);
                app.overlay_index = (app.overlay_index + 1).min(max);
            } else {
                let max = app.total_items().saturating_sub(1);
                if app.selected_index >= max {
                    app.selected_index = 0;
                } else {
                    app.selected_index += 1;
                }
            }
        }
        _ => {}
    }
}

fn handle_nav_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Up | KeyCode::Char('k') if ctrl => {
            app.selected_index = app::COMMANDS.len().min(app.total_items().saturating_sub(1));
        }
        KeyCode::Down | KeyCode::Char('j') if ctrl => {
            app.selected_index = app.total_items().saturating_sub(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.selected_index == 0 {
                app.selected_index = app.total_items().saturating_sub(1);
            } else {
                app.selected_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.total_items().saturating_sub(1);
            if app.selected_index >= max {
                app.selected_index = 0;
            } else {
                app.selected_index += 1;
            }
        }
        KeyCode::Char('n') => open_action(app, ActiveAction::NewBranch),
        KeyCode::Char('p') => open_action(app, ActiveAction::SyncPr),
        KeyCode::Char('d') => open_action(app, ActiveAction::Delete),
        KeyCode::Char('s') => open_action(app, ActiveAction::SyncTrees),
        KeyCode::Char('c') => open_action(app, ActiveAction::CopySecrets),
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
            "Copy Secrets" => open_action(app, ActiveAction::CopySecrets),
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

    if action == ActiveAction::SyncTrees {
        app.sync_results.clear();
        app.sync_loading = false;
        app.sync_pending = false;
        // Pre-select the main (first) worktree
        app.sync_selected_idx = app.worktrees.iter().position(|w| w.is_main).unwrap_or(0);
    }

    if action == ActiveAction::SyncPr {
        app.sync_pr_loading = false;
        app.sync_pr_pending = None;
    }

    if action == ActiveAction::Delete {
        app.overlay_index = 0;
    }

    if action == ActiveAction::CopySecrets {
        app.copy_secrets_phase = CopySecretsPhase::SelectSource;
        app.copy_secrets_source_idx = None;
        app.copy_secrets_target_idx = app
            .worktrees
            .iter()
            .position(|wt| wt.is_current)
            .unwrap_or(0);
        app.copy_secrets_confirm_yes = true;
    }

    app.active_action = action;
}

fn refresh_worktrees(app: &mut App) {
    let result = if app.is_workspace {
        git::list_workspace_worktrees(&app.repo_root)
    } else {
        git::list_worktrees(&app.repo_root)
    };
    match result {
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

    // Results phase: Esc/Enter (or any other key) closes.
    if !app.sync_results.is_empty() {
        match code {
            KeyCode::Esc | KeyCode::Enter => {}
            _ => {}
        }
        app.active_action = ActiveAction::None;
        app.sync_results.clear();
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
                Ok((_, dest)) => {
                    app.active_action = ActiveAction::None;
                    app.clear_input();
                    refresh_worktrees(app);
                    app.exit_path = Some(dest.to_string_lossy().into_owned());
                    app.should_quit = true;
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
    if app.sync_pr_loading {
        return;
    }

    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.overlay_error = None;
            app.clear_input();
        }
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Enter => {
            let raw = app.input_buffer.trim();
            let pr_input = raw.trim_start_matches('#');
            if pr_input.is_empty() || !pr_input.chars().all(|c| c.is_ascii_digit()) {
                app.overlay_error = Some("Invalid PR number. Use #123 or 123.".to_string());
                return;
            }
            let pr_number: u32 = match pr_input.parse() {
                Ok(n) => n,
                Err(_) => {
                    app.overlay_error = Some("Invalid PR number. Use #123 or 123.".to_string());
                    return;
                }
            };

            app.overlay_error = None;
            app.sync_pr_loading = true;
            app.sync_pr_pending = Some(pr_number);
        }
        KeyCode::Char(c) => app.input_char(c),
        _ => {}
    }
}

fn handle_delete_key(app: &mut App, code: KeyCode) {
    let deletable_len = app.deletable_worktrees().len();

    if app.delete_confirming {
        match code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
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
                            app.overlay_error = Some(format!("Failed to delete worktree: {e}"));
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
        KeyCode::Up | KeyCode::Char('k') => app.overlay_index = app.overlay_index.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            app.overlay_index = (app.overlay_index + 1).min(deletable_len.saturating_sub(1));
        }
        KeyCode::Enter if deletable_len > 0 => app.delete_confirming = true,
        _ => {}
    }
}

fn handle_copy_secrets_key(app: &mut App, code: KeyCode) {
    match app.copy_secrets_phase {
        CopySecretsPhase::SelectSource => match code {
            KeyCode::Esc => {
                app.active_action = ActiveAction::None;
                app.overlay_error = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let next = copy_selection_idx(app).saturating_sub(1);
                set_copy_selection_idx(app, next);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = app.worktrees.len().saturating_sub(1);
                let next = (copy_selection_idx(app) + 1).min(max);
                set_copy_selection_idx(app, next);
            }
            KeyCode::Enter => handle_copy_secrets_select(app, copy_selection_idx(app)),
            _ => {}
        },
        CopySecretsPhase::SelectTarget => match code {
            KeyCode::Esc => {
                app.copy_secrets_phase = CopySecretsPhase::SelectSource;
                app.copy_secrets_source_idx = None;
                app.overlay_error = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let next = copy_selection_idx(app).saturating_sub(1);
                set_copy_selection_idx(app, next);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = app.worktrees.len().saturating_sub(1);
                let next = (copy_selection_idx(app) + 1).min(max);
                set_copy_selection_idx(app, next);
            }
            KeyCode::Enter => handle_copy_secrets_select(app, copy_selection_idx(app)),
            _ => {}
        },
        CopySecretsPhase::ConfirmOverwrite => match code {
            KeyCode::Left | KeyCode::Char('h') => app.copy_secrets_confirm_yes = true,
            KeyCode::Right | KeyCode::Char('l') => app.copy_secrets_confirm_yes = false,
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.copy_secrets_confirm_yes = true;
                finish_copy_secrets(app, true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.copy_secrets_confirm_yes = false;
                finish_copy_secrets(app, false);
            }
            KeyCode::Enter => finish_copy_secrets(app, app.copy_secrets_confirm_yes),
            _ => {}
        },
    }
}

fn copy_selection_idx(app: &App) -> usize {
    match app.copy_secrets_phase {
        CopySecretsPhase::SelectSource => app.copy_secrets_source_idx.unwrap_or_else(|| {
            app.worktrees
                .iter()
                .position(|wt| wt.is_current)
                .unwrap_or(0)
        }),
        CopySecretsPhase::SelectTarget | CopySecretsPhase::ConfirmOverwrite => {
            app.copy_secrets_target_idx
        }
    }
}

fn set_copy_selection_idx(app: &mut App, idx: usize) {
    if app.copy_secrets_phase == CopySecretsPhase::SelectSource {
        app.copy_secrets_source_idx = Some(idx);
    } else {
        app.copy_secrets_target_idx = idx;
    }
}

fn handle_copy_secrets_select(app: &mut App, wt_idx: usize) {
    match app.copy_secrets_phase {
        CopySecretsPhase::SelectSource => {
            let Some(wt) = app.worktrees.get(wt_idx) else {
                return;
            };
            if !wt.has_secrets {
                app.overlay_error = Some("this worktree doesn't contain secrets".to_string());
                app.copy_secrets_source_idx = Some(wt_idx);
                return;
            }

            let Some(target_idx) = app.next_copy_target_idx(wt_idx) else {
                app.overlay_error = Some("No destination worktree available".to_string());
                return;
            };

            app.overlay_error = None;
            app.copy_secrets_source_idx = Some(wt_idx);
            app.copy_secrets_target_idx = target_idx;
            app.copy_secrets_confirm_yes = true;
            app.copy_secrets_phase = CopySecretsPhase::SelectTarget;
        }
        CopySecretsPhase::SelectTarget => {
            if Some(wt_idx) == app.copy_secrets_source_idx {
                return;
            }

            let Some(target) = app.worktrees.get(wt_idx) else {
                return;
            };
            app.copy_secrets_target_idx = wt_idx;
            app.overlay_error = None;

            if target.has_secrets {
                app.copy_secrets_phase = CopySecretsPhase::ConfirmOverwrite;
            } else {
                finish_copy_secrets(app, true);
            }
        }
        CopySecretsPhase::ConfirmOverwrite => {}
    }
}

fn finish_copy_secrets(app: &mut App, confirmed: bool) {
    if !confirmed {
        app.copy_secrets_phase = CopySecretsPhase::SelectTarget;
        return;
    }

    let Some(source_idx) = app.copy_secrets_source_idx else {
        return;
    };
    let target_idx = app.copy_secrets_target_idx;
    let Some(source) = app.worktrees.get(source_idx).cloned() else {
        return;
    };
    let Some(target) = app.worktrees.get(target_idx).cloned() else {
        return;
    };

    match git::copy_secret_files(&source, &target, true) {
        Ok(_) => {
            app.active_action = ActiveAction::None;
            app.overlay_error = None;
            refresh_worktrees(app);
        }
        Err(err) => {
            app.copy_secrets_phase = CopySecretsPhase::SelectTarget;
            app.overlay_error = Some(format!("Failed to copy secrets: {err}"));
        }
    }
}

fn handle_copy_secrets_confirm_click(app: &mut App, column: u16, row: u16) {
    let popup_width = 60_u16;
    let popup_height = 8_u16;
    let popup_x = app.frame_width.saturating_sub(popup_width) / 2;
    let popup_y = app.frame_height.saturating_sub(popup_height) / 2;
    let relative_x = column.saturating_sub(popup_x);
    let relative_y = row.saturating_sub(popup_y);

    if (4..=5).contains(&relative_y) {
        if relative_x <= popup_width / 2 {
            app.copy_secrets_confirm_yes = true;
            finish_copy_secrets(app, true);
        } else {
            app.copy_secrets_confirm_yes = false;
            finish_copy_secrets(app, false);
        }
    }
}

fn delete_overlay_index_for_worktree(app: &App, worktree_idx: usize) -> usize {
    let mut deletable_idx = 0;
    for (i, wt) in app.worktrees.iter().enumerate() {
        if !wt.is_main && !wt.is_current {
            if i == worktree_idx {
                return deletable_idx;
            }
            deletable_idx += 1;
        }
    }
    0
}

fn handle_clone_key(app: &mut App, code: KeyCode) {
    if app.clone_loading {
        return;
    }

    match code {
        KeyCode::Esc => {
            if app.clone_step == 1 {
                app.clone_step = 0;
                app.input_buffer = app.clone_url.clone();
                app.input_cursor = app.clone_url.chars().count();
                app.clone_error = None;
            } else if app.no_repo {
                app.should_quit = true;
            } else {
                app.active_action = ActiveAction::None;
                app.clone_error = None;
                app.clear_input();
            }
        }
        KeyCode::Backspace => app.input_backspace(),
        KeyCode::Delete => app.input_delete(),
        KeyCode::Left => app.input_left(),
        KeyCode::Right => app.input_right(),
        KeyCode::Home => app.input_home(),
        KeyCode::End => app.input_end(),
        KeyCode::Enter => {
            let input = app.input_buffer.trim().to_string();
            if input.is_empty() {
                return;
            }
            if app.clone_step == 0 {
                app.clone_url = input.clone();
                let dest = git::dest_from_url(&input, &app.repo_root);
                app.clear_input();
                app.input_buffer = dest.clone();
                app.input_cursor = dest.chars().count();
                app.clone_step = 1;
                app.clone_error = None;
            } else {
                app.clone_loading = true;
                app.clone_error = None;
                app.clear_clone_output();
                app.reset_clone_animation();
                app.clone_receiver = Some(git::start_clone_repo_with_layout(
                    app.clone_url.clone(),
                    PathBuf::from(input),
                ));
            }
        }
        KeyCode::Char(c) => app.input_char(c),
        _ => {}
    }
}

fn poll_clone_updates(app: &mut App) {
    let mut events = Vec::new();
    let mut disconnected = false;
    let mut clear_receiver = false;

    if let Some(receiver) = app.clone_receiver.as_ref() {
        loop {
            match receiver.try_recv() {
                Ok(event) => {
                    let is_terminal =
                        matches!(event, CloneEvent::Finished(_) | CloneEvent::Error(_));
                    events.push(event);
                    if is_terminal {
                        clear_receiver = true;
                        break;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    clear_receiver = true;
                    break;
                }
            }
        }
    }

    for event in events {
        match event {
            CloneEvent::Progress { line } => {
                app.push_clone_output(line);
            }
            CloneEvent::Finished(worktree_path) => {
                app.clone_loading = false;
                app.exit_path = Some(worktree_path.to_string_lossy().into_owned());
                app.should_quit = true;
            }
            CloneEvent::Error(err) => {
                app.clone_loading = false;
                app.clone_error = Some(err);
            }
        }
    }

    if disconnected && app.clone_loading {
        app.clone_loading = false;
        app.clone_error = Some("Clone process ended unexpectedly.".to_string());
    }

    if clear_receiver {
        app.clone_receiver = None;
    }
}

fn handle_paste(app: &mut App, text: &str) {
    if app.active_action != ActiveAction::CloneRepo || app.clone_loading {
        return;
    }

    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() {
        return;
    }

    app.input_str(text);
}
