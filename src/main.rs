mod app;
mod git;
mod text_input;
mod types;
mod ui;
mod update;
mod version;

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
use text_input::TextInputKeyResult;
use types::{ActiveAction, CheckoutRemotePhase, CloneEvent, CopySecretsPhase, SyncPrEvent};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("wt v{}", version::current_version());
        return Ok(());
    }

    if args.iter().any(|a| a == "--update") {
        update::run_manual_update()?;
        return Ok(());
    }

    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("no cwd"));

    let mark_tree = args.iter().any(|a| a == "--mark-tree");
    let mut update_notice_rx = (!mark_tree).then(update::start_background_update_check);
    let mut update_notice = None;

    if mark_tree {
        git::create_workspace_marker(&cwd)?;
    }

    let workspace_root_opt = git::find_workspace_root(&cwd).or_else(|| {
        if git::detect_worktree_workspace(&cwd) {
            let _ = git::create_workspace_marker(&cwd);
            Some(cwd.clone())
        } else {
            None
        }
    });
    let repo_root_opt = if workspace_root_opt.is_some() {
        None
    } else {
        git::find_repo_root(&cwd)
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
        if let Some(rx) = update_notice_rx.as_ref() {
            match rx.try_recv() {
                Ok(notice) => {
                    update_notice = notice;
                    update_notice_rx = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => update_notice_rx = None,
            }
        }

        poll_clone_updates(&mut app);
        poll_sync_pr_updates(&mut app);
        poll_sync_updates(&mut app);
        poll_checkout_remote_fetch(&mut app);
        if app.sync_loading
            || app.sync_pr_loading
            || app.clone_loading
            || app.new_branch_loading
            || app.delete_loading
            || app.copy_secrets_loading
            || app.checkout_remote_is_loading()
        {
            app.advance_loading_animation();
        }
        let wants_mouse_capture = text_input::wants_mouse_capture(&app);
        if wants_mouse_capture != mouse_capture_enabled {
            if wants_mouse_capture {
                execute!(terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                execute!(terminal.backend_mut(), DisableMouseCapture)?;
            }
            mouse_capture_enabled = wants_mouse_capture;
        }
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Start sync after the loading frame has been rendered.
        if app.sync_pending {
            app.sync_pending = false;
            let wt = app.worktrees.get(app.sync_selected_idx).cloned();
            if let Some(wt) = wt {
                let root = app.repo_root.clone();
                app.sync_receiver = Some(git::start_sync_one_worktree(root, wt));
            }
        }

        // Execute pending new branch after the loading frame has been rendered.
        if let Some(branch) = app.new_branch_pending.take() {
            let root = app.repo_root.clone();
            let result = if app.new_branch_use_existing {
                git::add_worktree_from_existing(&root, &branch)
            } else {
                let base = app.new_branch_base.as_deref();
                git::add_worktree(&root, &branch, base)
            };
            match result {
                Ok((_, dest)) => {
                    app.new_branch_loading = false;
                    app.new_branch_use_existing = false;
                    app.new_branch_confirm_existing = None;
                    app.active_action = ActiveAction::None;
                    app.clear_input();
                    refresh_worktrees(&mut app);
                    app.exit_path = Some(dest.to_string_lossy().into_owned());
                    app.should_quit = true;
                }
                Err(e) => {
                    app.new_branch_loading = false;
                    app.new_branch_use_existing = false;
                    app.overlay_error = Some(format!("Failed to create branch: {e}"));
                }
            }
        }

        // Execute pending delete after the loading frame has been rendered.
        if let Some(paths) = app.delete_pending.take() {
            let root = app.repo_root.clone();
            match git::remove_worktrees(&root, &paths) {
                Ok(_) => {
                    app.delete_loading = false;
                    app.active_action = ActiveAction::None;
                    app.delete_warn_current = false;
                    app.delete_confirm_targets.clear();
                    app.overlay_error = None;
                    app.delete_checked.clear();
                    if let Some(path) = app.delete_redirect_path.take() {
                        app.exit_path = Some(path);
                        app.should_quit = true;
                    } else {
                        refresh_worktrees(&mut app);
                    }
                }
                Err(e) => {
                    app.delete_loading = false;
                    app.active_action = ActiveAction::None;
                    app.delete_warn_current = false;
                    app.delete_confirm_targets.clear();
                    app.delete_redirect_path = None;
                    app.delete_checked.clear();
                    app.overlay_error = Some(format!("Failed to delete worktree: {e}"));
                }
            }
        }

        // Execute pending copy secrets after the loading frame has been rendered.
        if app.copy_secrets_pending {
            app.copy_secrets_pending = false;
            let source = app
                .copy_secrets_source_idx
                .and_then(|i| app.worktrees.get(i))
                .cloned();
            let target = app.worktrees.get(app.copy_secrets_target_idx).cloned();
            if let (Some(source), Some(target)) = (source, target) {
                match git::copy_secret_files(&source, &target, true) {
                    Ok(_) => {
                        app.copy_secrets_loading = false;
                        app.active_action = ActiveAction::None;
                        app.overlay_error = None;
                        refresh_worktrees(&mut app);
                    }
                    Err(e) => {
                        app.copy_secrets_loading = false;
                        app.copy_secrets_phase = CopySecretsPhase::SelectTarget;
                        app.overlay_error = Some(format!("Failed to copy secrets: {e}"));
                    }
                }
            } else {
                app.copy_secrets_loading = false;
            }
        }

        if let Some((remote, branch)) = app.checkout_remote_pending.take() {
            match git::checkout_remote_branch(&app.repo_root, &remote, &branch) {
                Ok(dest) => {
                    app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
                    app.active_action = ActiveAction::None;
                    app.clear_input();
                    refresh_worktrees(&mut app);
                    app.exit_path = Some(dest.to_string_lossy().into_owned());
                    app.should_quit = true;
                }
                Err(e) => {
                    app.checkout_remote_phase = CheckoutRemotePhase::EnterBranch;
                    app.overlay_error = Some(e.to_string());
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

    // Drain any mouse/key events buffered while the event loop was blocked
    // (e.g. during synchronous git operations) so they don't leak into the shell.
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }

    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
    )?;
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }

    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    if update_notice.is_none()
        && let Some(rx) = update_notice_rx.take()
        && let Ok(notice) = rx.try_recv()
    {
        update_notice = notice;
    }

    if let Some(notice) = update_notice {
        eprintln!(
            "Update available for wt: v{} (current: v{}). Run `wt --update` to install.",
            notice.latest_version, notice.current_version
        );
    }

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
        ActiveAction::NewBranch => handle_new_branch_key(app, code, modifiers),
        ActiveAction::SyncPr => handle_sync_pr_key(app, code, modifiers),
        ActiveAction::SyncTrees => handle_sync_trees_key(app, code),
        ActiveAction::Delete => handle_delete_key(app, code),
        ActiveAction::CopySecrets => handle_copy_secrets_key(app, code),
        ActiveAction::CloneRepo => handle_clone_key(app, code, modifiers),
        ActiveAction::CheckoutRemote => handle_checkout_remote_key(app, code, modifiers),
        ActiveAction::None => handle_nav_key(app, code, modifiers),
    }
}

fn handle_mouse(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    if matches!(kind, MouseEventKind::Down(MouseButton::Left))
        && app.active_action == ActiveAction::NewBranch
        && app.new_branch_confirm_existing.is_some()
    {
        handle_new_branch_confirm_click(app, column, row);
        return;
    }

    if matches!(kind, MouseEventKind::Down(MouseButton::Left))
        && app.active_action == ActiveAction::Delete
        && (app.delete_confirming || app.delete_warn_current)
    {
        handle_delete_confirm_click(app, column, row);
        return;
    }

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
    let delete_select = app.active_action == ActiveAction::Delete
        && !app.delete_confirming
        && !app.delete_warn_current
        && !app.delete_loading;
    let copy_select = app.active_action == ActiveAction::CopySecrets
        && app.copy_secrets_phase != CopySecretsPhase::ConfirmOverwrite
        && !app.copy_secrets_loading;

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
                        app.reset_loading_animation();
                        app.sync_pending = true;
                    }
                } else if delete_select {
                    if idx >= app::COMMANDS.len() {
                        let wt_idx = idx - app::COMMANDS.len();
                        if app.is_deletable_worktree_idx(wt_idx) {
                            app.overlay_index = wt_idx;
                            toggle_delete_selection(app, wt_idx);
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
                let current = delete_cursor_idx(app);
                app.overlay_index = app
                    .previous_deletable_worktree_idx(current)
                    .unwrap_or(current);
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
                let current = delete_cursor_idx(app);
                app.overlay_index = app.next_deletable_worktree_idx(current).unwrap_or(current);
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
        KeyCode::Char(c) => {
            if let Some(action) = App::command_action_for_shortcut(c) {
                open_action(app, action);
            }
        }
        KeyCode::Enter => activate(app),
        _ => {}
    }
}

fn activate(app: &mut App) {
    let idx = app.selected_index;
    if idx < app::COMMANDS.len() {
        if let Some(action) = App::command_action_for_index(idx) {
            open_action(app, action);
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
    app.new_branch_use_existing = false;
    app.new_branch_confirm_existing = None;
    app.new_branch_confirm_yes = false;
    app.delete_confirming = false;
    app.delete_warn_current = false;
    app.delete_confirm_targets.clear();
    app.delete_confirm_yes = false;
    app.delete_redirect_path = None;
    app.overlay_error = None;
    app.new_branch_base = None;

    if action == ActiveAction::NewBranch {
        let idx = app.selected_index;
        if idx >= app::COMMANDS.len() {
            let wt_idx = idx - app::COMMANDS.len();
            if let Some(wt) = app.worktrees.get(wt_idx) {
                app.new_branch_base = Some(wt.branch.clone());
            }
        }
    }

    if action == ActiveAction::SyncTrees {
        app.sync_results.clear();
        app.sync_loading = false;
        app.sync_pending = false;
        app.sync_receiver = None;
        app.sync_fetch_ok = true;
        app.reset_loading_animation();
        // Pre-select the main (first) worktree
        app.sync_selected_idx = app.worktrees.iter().position(|w| w.is_main).unwrap_or(0);
    }

    if action == ActiveAction::SyncPr {
        app.sync_pr_loading = false;
        app.sync_pr_receiver = None;
        app.clear_sync_pr_output();
    }

    if action == ActiveAction::Delete {
        app.delete_checked.clear();
        app.overlay_index = initial_delete_cursor_idx(app).unwrap_or(0);
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

    if action == ActiveAction::CheckoutRemote {
        app.checkout_remote_name.clear();
        app.checkout_remote_fetch_receiver = None;
        app.checkout_remote_pending = None;

        let remotes = git::list_remotes(&app.repo_root).unwrap_or_default();
        if remotes.is_empty() {
            app.checkout_remote_remotes = vec![];
            app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
            app.overlay_error = Some("No remotes configured for this repository.".to_string());
        } else if remotes.len() == 1 {
            app.checkout_remote_name = remotes[0].clone();
            app.checkout_remote_remotes = remotes;
            app.checkout_remote_phase = CheckoutRemotePhase::FetchingRemote;
            app.checkout_remote_fetch_receiver = Some(git::start_fetch_remote(
                app.repo_root.clone(),
                app.checkout_remote_name.clone(),
            ));
            app.reset_loading_animation();
        } else {
            app.checkout_remote_remotes = remotes;
            app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
        }
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
                app.reset_loading_animation();
                app.sync_pending = true;
            }
        }
        _ => {}
    }
}

fn handle_new_branch_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.new_branch_loading {
        return;
    }

    if app.new_branch_confirm_existing.is_some() {
        match code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                app.new_branch_confirm_yes = true;
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => {
                app.new_branch_confirm_yes = false;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.new_branch_confirm_yes = true;
                finish_new_branch_existing_confirmation(app, true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.new_branch_confirm_yes = false;
                finish_new_branch_existing_confirmation(app, false);
            }
            KeyCode::Enter => {
                finish_new_branch_existing_confirmation(app, app.new_branch_confirm_yes)
            }
            _ => {}
        }
        return;
    }

    match text_input::handle_key(app, code, modifiers) {
        TextInputKeyResult::Cancel => {
            app.active_action = ActiveAction::None;
            app.clear_input();
            app.overlay_error = None;
        }
        TextInputKeyResult::Submit => {
            let branch = app.input_buffer.trim().to_string();
            if branch.is_empty() {
                return;
            }
            match git::branch_exists(&app.repo_root, &branch) {
                Ok(true) => {
                    app.overlay_error = None;
                    app.new_branch_confirm_existing = Some(branch);
                    app.new_branch_confirm_yes = false;
                    return;
                }
                Ok(false) => {}
                Err(e) => {
                    app.overlay_error = Some(format!("Failed to inspect branch: {e}"));
                    return;
                }
            }
            app.overlay_error = None;
            app.new_branch_loading = true;
            app.new_branch_pending = Some(branch);
        }
        TextInputKeyResult::Updated
        | TextInputKeyResult::Ignored
        | TextInputKeyResult::Complete => {}
    }
}

fn handle_sync_pr_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.sync_pr_loading {
        return;
    }

    match text_input::handle_key(app, code, modifiers) {
        TextInputKeyResult::Cancel => {
            app.active_action = ActiveAction::None;
            app.overlay_error = None;
            app.clear_input();
        }
        TextInputKeyResult::Submit => {
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
            app.clear_sync_pr_output();
            app.reset_loading_animation();
            app.sync_pr_receiver = Some(git::start_checkout_pr_as_worktree(
                app.repo_root.clone(),
                pr_number,
            ));
        }
        TextInputKeyResult::Updated
        | TextInputKeyResult::Ignored
        | TextInputKeyResult::Complete => {}
    }
}

fn poll_sync_pr_updates(app: &mut App) {
    let mut events = Vec::new();
    let mut disconnected = false;
    let mut clear_receiver = false;

    if let Some(receiver) = app.sync_pr_receiver.as_ref() {
        loop {
            match receiver.try_recv() {
                Ok(event) => {
                    let is_terminal =
                        matches!(event, SyncPrEvent::Finished(_) | SyncPrEvent::Error(_));
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
            SyncPrEvent::Progress { line } => {
                app.push_sync_pr_output(line);
            }
            SyncPrEvent::Finished(worktree_path) => {
                app.sync_pr_loading = false;
                app.active_action = ActiveAction::None;
                app.clear_input();
                refresh_worktrees(app);
                app.exit_path = Some(worktree_path.to_string_lossy().into_owned());
                app.should_quit = true;
            }
            SyncPrEvent::Error(err) => {
                app.sync_pr_loading = false;
                app.overlay_error = Some(format!("Failed to sync PR: {err}"));
            }
        }
    }

    if disconnected && app.sync_pr_loading {
        app.sync_pr_loading = false;
        app.overlay_error = Some("PR sync ended unexpectedly.".to_string());
    }

    if clear_receiver {
        app.sync_pr_receiver = None;
    }
}

fn poll_sync_updates(app: &mut App) {
    let mut completed = None;
    let mut disconnected = false;

    if let Some(receiver) = app.sync_receiver.as_ref() {
        match receiver.try_recv() {
            Ok(result) => {
                completed = Some(result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                disconnected = true;
            }
        }
    }

    if let Some((fetch_ok, result)) = completed {
        app.sync_fetch_ok = fetch_ok;
        app.sync_results = vec![result];
        app.sync_loading = false;
        app.sync_receiver = None;
        refresh_worktrees(app);
    } else if disconnected && app.sync_loading {
        app.sync_loading = false;
        app.sync_receiver = None;
        app.overlay_error = Some("Sync ended unexpectedly.".to_string());
    }
}

fn handle_delete_key(app: &mut App, code: KeyCode) {
    if app.delete_loading {
        return;
    }

    if app.delete_warn_current {
        match code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                app.delete_confirm_yes = true;
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => {
                app.delete_confirm_yes = false;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.delete_confirm_yes = true;
                finish_delete_current_warning(app, true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.delete_confirm_yes = false;
                finish_delete_current_warning(app, false);
            }
            KeyCode::Enter => finish_delete_current_warning(app, app.delete_confirm_yes),
            _ => {}
        }
        return;
    }

    if app.delete_confirming {
        match code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h') | KeyCode::Char('k') => {
                app.delete_confirm_yes = true;
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('l') => {
                app.delete_confirm_yes = false;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.delete_confirm_yes = true;
                finish_delete_confirmation(app, true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.delete_confirm_yes = false;
                finish_delete_confirmation(app, false);
            }
            KeyCode::Enter => finish_delete_confirmation(app, app.delete_confirm_yes),
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.delete_checked.clear();
            app.overlay_error = None;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let current = delete_cursor_idx(app);
            app.overlay_index = app
                .previous_deletable_worktree_idx(current)
                .unwrap_or(current);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let current = delete_cursor_idx(app);
            app.overlay_index = app.next_deletable_worktree_idx(current).unwrap_or(current);
        }
        KeyCode::Char(' ') => toggle_delete_selection(app, delete_cursor_idx(app)),
        KeyCode::Enter => submit_delete_selection(app),
        _ => {}
    }
}

fn handle_copy_secrets_key(app: &mut App, code: KeyCode) {
    if app.copy_secrets_loading {
        return;
    }
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
    if app.copy_secrets_source_idx.is_none() {
        return;
    }
    app.copy_secrets_loading = true;
    app.copy_secrets_pending = true;
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

fn initial_delete_cursor_idx(app: &App) -> Option<usize> {
    if app.selected_index >= app::COMMANDS.len() {
        let wt_idx = app.selected_index - app::COMMANDS.len();
        if app.is_deletable_worktree_idx(wt_idx) {
            return Some(wt_idx);
        }
    }

    app.first_deletable_worktree_idx()
}

fn delete_cursor_idx(app: &App) -> usize {
    let fallback = app.first_deletable_worktree_idx().unwrap_or(0);
    if app.is_deletable_worktree_idx(app.overlay_index) {
        app.overlay_index
    } else {
        fallback
    }
}

fn toggle_delete_selection(app: &mut App, wt_idx: usize) {
    if !app.is_deletable_worktree_idx(wt_idx) {
        return;
    }

    app.overlay_index = wt_idx;
    if !app.delete_checked.insert(wt_idx) {
        app.delete_checked.remove(&wt_idx);
    }
}

fn submit_delete_selection(app: &mut App) {
    let mut selected_indices: Vec<usize> = app.delete_checked.iter().copied().collect();
    selected_indices.retain(|idx| app.is_deletable_worktree_idx(*idx));

    let targets = if selected_indices.is_empty() {
        if app.first_deletable_worktree_idx().is_some() {
            vec![delete_cursor_idx(app)]
        } else {
            vec![]
        }
    } else {
        selected_indices
    };

    if !targets.is_empty() {
        app.overlay_index = targets[0];
        app.delete_confirm_targets = targets;
        app.delete_confirm_yes = false;
        app.delete_confirming = true;
    }
}

fn finish_delete_confirmation(app: &mut App, confirmed: bool) {
    if !confirmed {
        app.delete_confirming = false;
        app.delete_confirm_targets.clear();
        return;
    }

    if delete_targets_include_current(app) {
        app.delete_confirming = false;
        app.delete_warn_current = true;
        app.delete_confirm_yes = false;
        return;
    }

    start_delete_pending(app);
}

fn finish_delete_current_warning(app: &mut App, confirmed: bool) {
    if !confirmed {
        app.delete_warn_current = false;
        app.delete_confirm_targets.clear();
        return;
    }

    start_delete_pending(app);
}

fn start_delete_pending(app: &mut App) {
    let includes_current = delete_targets_include_current(app);
    let paths: Vec<String> = app
        .delete_confirm_targets
        .iter()
        .copied()
        .filter(|idx| app.is_deletable_worktree_idx(*idx))
        .filter_map(|idx| app.worktrees.get(idx).map(|wt| wt.path.clone()))
        .collect();

    app.delete_confirming = false;
    app.delete_warn_current = false;
    app.delete_confirm_targets.clear();

    if includes_current {
        app.delete_redirect_path = app
            .default_worktree_idx()
            .and_then(|idx| app.worktrees.get(idx).map(|wt| wt.path.clone()));

        if app.delete_redirect_path.is_none() {
            app.overlay_error = Some("No default worktree available to fall back to".to_string());
            return;
        }
    } else {
        app.delete_redirect_path = None;
    }

    if !paths.is_empty() {
        app.delete_loading = true;
        app.delete_pending = Some(paths);
    }
}

fn delete_targets_include_current(app: &App) -> bool {
    app.delete_confirm_targets.iter().copied().any(|idx| {
        app.worktrees
            .get(idx)
            .map(|wt| wt.is_current)
            .unwrap_or(false)
    })
}

fn handle_delete_confirm_click(app: &mut App, column: u16, row: u16) {
    let popup_width = 60_u16;
    let popup_height = if app.delete_warn_current {
        11_u16
    } else {
        10_u16
    };
    let popup_x = app.frame_width.saturating_sub(popup_width) / 2;
    let popup_y = app.frame_height.saturating_sub(popup_height) / 2;
    let relative_x = column.saturating_sub(popup_x);
    let relative_y = row.saturating_sub(popup_y);

    if (5..=6).contains(&relative_y) {
        if relative_x <= popup_width / 2 {
            app.delete_confirm_yes = true;
            if app.delete_warn_current {
                finish_delete_current_warning(app, true);
            } else {
                finish_delete_confirmation(app, true);
            }
        } else {
            app.delete_confirm_yes = false;
            if app.delete_warn_current {
                finish_delete_current_warning(app, false);
            } else {
                finish_delete_confirmation(app, false);
            }
        }
    }
}

fn finish_new_branch_existing_confirmation(app: &mut App, confirmed: bool) {
    let branch = app.new_branch_confirm_existing.take();

    if !confirmed {
        app.new_branch_use_existing = false;
        return;
    }

    if let Some(branch) = branch {
        app.new_branch_use_existing = true;
        app.new_branch_loading = true;
        app.new_branch_pending = Some(branch);
        app.overlay_error = None;
    }
}

fn handle_new_branch_confirm_click(app: &mut App, column: u16, row: u16) {
    let popup_width = 48_u16;
    let popup_height = 10_u16;
    let popup_x = app.frame_width.saturating_sub(popup_width) / 2;
    let popup_y = app.frame_height.saturating_sub(popup_height) / 2;
    let relative_x = column.saturating_sub(popup_x);
    let relative_y = row.saturating_sub(popup_y);

    if (5..=6).contains(&relative_y) {
        if relative_x <= popup_width / 2 {
            app.new_branch_confirm_yes = true;
            finish_new_branch_existing_confirmation(app, true);
        } else {
            app.new_branch_confirm_yes = false;
            finish_new_branch_existing_confirmation(app, false);
        }
    }
}

fn handle_clone_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.clone_loading {
        return;
    }

    match text_input::handle_key(app, code, modifiers) {
        TextInputKeyResult::Cancel => {
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
        TextInputKeyResult::Submit => {
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
                app.reset_loading_animation();
                app.clone_receiver = Some(git::start_clone_repo_with_layout(
                    app.clone_url.clone(),
                    PathBuf::from(input),
                ));
            }
        }
        TextInputKeyResult::Updated
        | TextInputKeyResult::Ignored
        | TextInputKeyResult::Complete => {}
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

fn handle_checkout_remote_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.checkout_remote_is_loading() {
        return;
    }

    match app.checkout_remote_phase {
        CheckoutRemotePhase::SelectRemote => match text_input::handle_key(app, code, modifiers) {
            TextInputKeyResult::Cancel => {
                app.active_action = ActiveAction::None;
                app.overlay_error = None;
                app.clear_input();
            }
            TextInputKeyResult::Submit => {
                let remote = app.input_buffer.trim().to_string();
                if remote.is_empty() {
                    return;
                }
                app.checkout_remote_name = remote.clone();
                app.overlay_error = None;
                app.checkout_remote_phase = CheckoutRemotePhase::FetchingRemote;
                app.checkout_remote_fetch_receiver =
                    Some(git::start_fetch_remote(app.repo_root.clone(), remote));
                app.reset_loading_animation();
            }
            TextInputKeyResult::Updated
            | TextInputKeyResult::Ignored
            | TextInputKeyResult::Complete => {}
        },
        CheckoutRemotePhase::EnterBranch => match text_input::handle_key(app, code, modifiers) {
            TextInputKeyResult::Cancel => {
                app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
                app.input_buffer = app.checkout_remote_name.clone();
                app.input_cursor = app.checkout_remote_name.chars().count();
                app.overlay_error = None;
            }
            TextInputKeyResult::Complete => {
                if let Some(ghost) = app.checkout_remote_ghost() {
                    app.input_str(&ghost);
                }
            }
            TextInputKeyResult::Submit => {
                let branch_input = app.input_buffer.trim();
                if branch_input.is_empty() {
                    return;
                }
                let branch = match git::normalize_checkout_remote_branch_input(
                    branch_input,
                    &app.checkout_remote_name,
                    &app.checkout_remote_remotes,
                    &app.checkout_remote_branches,
                ) {
                    Ok(branch) => branch,
                    Err(err) => {
                        app.overlay_error = Some(err.to_string());
                        return;
                    }
                };
                if app.worktrees.iter().any(|wt| wt.branch == branch) {
                    app.overlay_error = Some(format!("'{branch}' is already checked out."));
                    return;
                }
                app.overlay_error = None;
                app.checkout_remote_phase = CheckoutRemotePhase::CreatingWorktree;
                app.checkout_remote_pending = Some((app.checkout_remote_name.clone(), branch));
                app.reset_loading_animation();
            }
            TextInputKeyResult::Updated | TextInputKeyResult::Ignored => {}
        },
        CheckoutRemotePhase::FetchingRemote | CheckoutRemotePhase::CreatingWorktree => {}
    }
}

fn poll_checkout_remote_fetch(app: &mut App) {
    if app.checkout_remote_phase != CheckoutRemotePhase::FetchingRemote {
        return;
    }

    let result = app
        .checkout_remote_fetch_receiver
        .as_ref()
        .and_then(|rx| match rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Disconnected) => Some(Err("Fetch ended unexpectedly.".to_string())),
            Err(TryRecvError::Empty) => None,
        });

    if let Some(result) = result {
        app.checkout_remote_fetch_receiver = None;
        match result {
            Ok(()) => {
                app.checkout_remote_branches =
                    git::list_remote_branches(&app.repo_root, &app.checkout_remote_name);
                app.checkout_remote_phase = CheckoutRemotePhase::EnterBranch;
                app.clear_input();
                app.overlay_error = None;
            }
            Err(e) => {
                app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
                app.input_buffer = app.checkout_remote_name.clone();
                app.input_cursor = app.checkout_remote_name.chars().count();
                app.overlay_error = Some(format!("Fetch failed: {e}"));
            }
        }
    }
}

fn handle_paste(app: &mut App, text: &str) {
    let _ = text_input::handle_paste(app, text);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::handle_paste;
    use crate::{
        app::App,
        text_input,
        types::{ActiveAction, CheckoutRemotePhase},
    };

    fn test_app() -> App {
        App::new(PathBuf::from("."))
    }

    #[test]
    fn text_input_overlay_active_only_for_editable_overlays() {
        let mut app = test_app();

        assert!(!text_input::is_active(&app));

        app.active_action = ActiveAction::NewBranch;
        assert!(text_input::is_active(&app));
        app.new_branch_loading = true;
        assert!(!text_input::is_active(&app));

        app = test_app();
        app.active_action = ActiveAction::SyncPr;
        assert!(text_input::is_active(&app));
        app.sync_pr_loading = true;
        assert!(!text_input::is_active(&app));

        app = test_app();
        app.active_action = ActiveAction::CloneRepo;
        assert!(text_input::is_active(&app));
        app.clone_loading = true;
        assert!(!text_input::is_active(&app));

        app = test_app();
        app.active_action = ActiveAction::CheckoutRemote;
        app.checkout_remote_phase = CheckoutRemotePhase::SelectRemote;
        assert!(text_input::is_active(&app));
        app.checkout_remote_phase = CheckoutRemotePhase::EnterBranch;
        assert!(text_input::is_active(&app));
        app.checkout_remote_phase = CheckoutRemotePhase::FetchingRemote;
        assert!(!text_input::is_active(&app));
    }

    #[test]
    fn handle_paste_inserts_into_all_text_input_overlays() {
        let editable_states = [
            (ActiveAction::NewBranch, CheckoutRemotePhase::SelectRemote),
            (ActiveAction::SyncPr, CheckoutRemotePhase::SelectRemote),
            (ActiveAction::CloneRepo, CheckoutRemotePhase::SelectRemote),
            (
                ActiveAction::CheckoutRemote,
                CheckoutRemotePhase::SelectRemote,
            ),
            (
                ActiveAction::CheckoutRemote,
                CheckoutRemotePhase::EnterBranch,
            ),
        ];

        for (action, phase) in editable_states {
            let mut app = test_app();
            app.active_action = action;
            app.checkout_remote_phase = phase;

            handle_paste(&mut app, "feature/test\n");

            assert_eq!(app.input_buffer, "feature/test");
            assert_eq!(app.input_cursor, "feature/test".chars().count());
        }
    }

    #[test]
    fn handle_paste_ignores_non_editable_states() {
        let mut app = test_app();
        app.active_action = ActiveAction::SyncTrees;

        handle_paste(&mut app, "feature/test");
        assert!(app.input_buffer.is_empty());

        app.active_action = ActiveAction::CheckoutRemote;
        app.checkout_remote_phase = CheckoutRemotePhase::CreatingWorktree;

        handle_paste(&mut app, "feature/test");
        assert!(app.input_buffer.is_empty());
    }
}
