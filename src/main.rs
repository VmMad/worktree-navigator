mod app;
mod cli;
mod config;
mod git;
mod text_input;
mod types;
mod ui;
mod update;
mod version;

use std::io::{Write, stderr};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    style::Stylize,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::{App, PendingConsoleOperation};
use cli::{BranchBase, ParsedArgs};
use config::{PostCreateRequest, PostCreateScript, RepoConfig};
use text_input::TextInputKeyResult;
use types::{
    ActiveAction, CheckoutRemotePhase, CloneEvent, CopySecretsPhase, OptionsPhase, SyncPrEvent,
    Worktree,
};

struct TuiCleanupGuard {
    active: bool,
}

impl TuiCleanupGuard {
    fn new() -> Self {
        Self { active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for TuiCleanupGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = cleanup_terminal_state();
        }
    }
}

fn main() -> Result<()> {
    let args = cli::parse_args(std::env::args().skip(1))?;
    let cwd = resolve_cwd();

    match args {
        ParsedArgs::Tui { mark_tree } => run_tui(cwd, mark_tree),
        ParsedArgs::Version => {
            if std::env::var_os("WT_CWD").is_some() {
                eprintln!("wt v{}", version::current_version());
            } else {
                println!("wt v{}", version::current_version());
            }
            Ok(())
        }

        ParsedArgs::Update => update::run_manual_update(),
        ParsedArgs::Help => {
            cli::print_help();
            Ok(())
        }
        command => run_cli_command(&cwd, command),
    }
}

fn suspend_terminal(
    mouse_capture_enabled: &mut bool,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
) -> Result<()> {
    terminal.backend_mut().flush()?;
    cleanup_terminal_state()?;
    *mouse_capture_enabled = false;
    Ok(())
}

fn resume_terminal(
    app: &App,
    mouse_capture_enabled: &mut bool,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
) -> Result<()> {
    enable_raw_mode()?;
    if text_input::wants_mouse_capture(app) {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        *mouse_capture_enabled = true;
    } else {
        execute!(
            terminal.backend_mut(),
            EnterAlternateScreen,
            EnableBracketedPaste
        )?;
        *mouse_capture_enabled = false;
    }
    terminal.clear()?;
    Ok(())
}

fn update_repo_config(app: &mut App, update: impl FnOnce(&mut RepoConfig)) -> Result<()> {
    let mut next = app.repo_config.clone();
    update(&mut next);
    config::save_repo_config(&app.repo_root, &next)?;
    app.repo_config = next;
    Ok(())
}

fn select_worktree_by_path(app: &mut App, path: &Path) {
    let path = path.to_string_lossy();
    if let Some(idx) = app.worktrees.iter().position(|wt| wt.path == path) {
        app.selected_index = app::COMMANDS.len() + idx;
    }
}

fn write_exit_post_create_request(
    app: &mut App,
    branch: &str,
    base_branch: Option<String>,
    dest: &Path,
    scripts: Vec<PostCreateScript>,
) -> Result<()> {
    let request_path = config::write_post_create_request(&PostCreateRequest {
        repo_root: app.repo_root.clone(),
        worktree_path: dest.to_path_buf(),
        branch: branch.to_string(),
        base_branch,
        scripts,
    })?;
    app.exit_path = Some(dest.to_string_lossy().into_owned());
    app.exit_post_create_request = Some(request_path.to_string_lossy().into_owned());
    app.should_quit = true;
    Ok(())
}

fn complete_new_worktree_creation(
    app: &mut App,
    mouse_capture_enabled: &mut bool,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    branch: &str,
    base_branch: Option<String>,
    dest: &Path,
) -> Result<()> {
    let scripts = app.repo_config.enabled_post_create_scripts();

    app.new_branch_loading = false;
    app.new_branch_use_existing = false;
    app.new_branch_confirm_existing = None;
    app.active_action = ActiveAction::None;
    app.clear_input();
    refresh_worktrees(app);
    select_worktree_by_path(app, dest);

    if scripts.is_empty() {
        app.exit_path = Some(dest.to_string_lossy().into_owned());
        app.should_quit = true;
        return Ok(());
    }

    if std::env::var_os("WT_SHELL_WRAPPER").is_some() {
        return write_exit_post_create_request(app, branch, base_branch, dest, scripts);
    }

    suspend_terminal(mouse_capture_enabled, terminal)?;
    eprintln!(
        "[wt] Running {} post-create setup step(s) for {}",
        scripts.len(),
        branch
    );

    match config::run_post_create_scripts(
        &app.repo_root,
        dest,
        branch,
        base_branch.as_deref(),
        &scripts,
    ) {
        Ok(()) => {
            eprintln!();
            eprintln!("[wt] Setup complete.");
            app.exit_path = Some(dest.to_string_lossy().into_owned());
            app.should_quit = true;
        }
        Err(err) => {
            resume_terminal(app, mouse_capture_enabled, terminal)?;
            app.overlay_error = Some(format!("Worktree created, but setup failed: {err}"));
        }
    }

    Ok(())
}

fn run_post_create_during_handoff(
    app: &mut App,
    branch: &str,
    base_branch: Option<String>,
    dest: &Path,
) {
    let scripts = app.repo_config.enabled_post_create_scripts();
    if scripts.is_empty() {
        app.exit_path = Some(dest.to_string_lossy().into_owned());
        app.should_quit = true;
        return;
    }

    if std::env::var_os("WT_SHELL_WRAPPER").is_some() {
        match write_exit_post_create_request(app, branch, base_branch, dest, scripts) {
            Ok(()) => {}
            Err(err) => {
                app.console_handoff_needs_resume |= app.console_handoff_active;
                app.overlay_error =
                    Some(format!("Worktree created, but setup request failed: {err}"));
            }
        }
        return;
    }

    eprintln!();
    eprintln!(
        "[wt] Running {} post-create setup step(s) for {branch}",
        scripts.len()
    );
    match config::run_post_create_scripts(
        &app.repo_root,
        dest,
        branch,
        base_branch.as_deref(),
        &scripts,
    ) {
        Ok(()) => {
            eprintln!();
            eprintln!("[wt] Setup complete.");
            app.exit_path = Some(dest.to_string_lossy().into_owned());
            app.should_quit = true;
        }
        Err(err) => {
            app.console_handoff_needs_resume |= app.console_handoff_active;
            app.overlay_error = Some(format!("Worktree created, but setup failed: {err}"));
        }
    }
}

fn start_pending_console_operation(
    app: &mut App,
    mouse_capture_enabled: &mut bool,
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
) -> Result<()> {
    let Some(operation) = app.pending_console_operation.take() else {
        return Ok(());
    };

    suspend_terminal(mouse_capture_enabled, terminal)?;
    app.console_handoff_active = true;
    app.console_handoff_needs_resume = false;

    match operation {
        PendingConsoleOperation::CloneRepo { url, dest } => {
            eprintln!();
            app.clone_receiver = Some(git::start_clone_repo_with_layout(url, dest));
        }
        PendingConsoleOperation::SyncPr { pr_number } => {
            eprintln!();
            eprintln!("[wt] Syncing PR #{pr_number}…");
            app.sync_pr_receiver = Some(git::start_checkout_pr_as_worktree(
                app.repo_root.clone(),
                pr_number,
            ));
        }
        PendingConsoleOperation::SyncWorktree { wt } => {
            eprintln!();
            eprintln!("[wt] Syncing {} with origin…", wt.branch);
            app.sync_receiver = Some(git::start_sync_one_worktree(app.repo_root.clone(), wt));
        }
        PendingConsoleOperation::FetchRemote { remote } => {
            eprintln!();
            eprintln!("[wt] Fetching from {remote}…");
            app.checkout_remote_fetch_receiver =
                Some(git::start_fetch_remote(app.repo_root.clone(), remote));
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct RepoContext {
    cwd: PathBuf,
    repo_root: PathBuf,
    is_workspace: bool,
    no_repo: bool,
}

fn resolve_cwd() -> PathBuf {
    std::env::var("WT_CWD")
        .map(PathBuf::from)
        .or_else(|_| std::env::current_dir())
        .expect("no cwd")
}

fn display_path_with_home(path: &Path) -> String {
    match std::env::var_os("HOME") {
        Some(home) => {
            let home = PathBuf::from(home);
            match path.strip_prefix(&home) {
                Ok(relative) if relative.as_os_str().is_empty() => "~".to_string(),
                Ok(relative) => format!("~/{}", relative.display()),
                Err(_) => path.display().to_string(),
            }
        }
        None => path.display().to_string(),
    }
}

fn resolve_repo_context(cwd: &Path) -> RepoContext {
    let workspace_root_opt = git::find_workspace_root(cwd).or_else(|| {
        if git::detect_worktree_workspace(cwd) {
            let _ = git::create_workspace_marker(cwd);
            Some(cwd.to_path_buf())
        } else {
            None
        }
    });
    let repo_root_opt = if workspace_root_opt.is_some() {
        None
    } else {
        git::find_repo_root(cwd)
    };

    let no_repo = repo_root_opt.is_none() && workspace_root_opt.is_none();
    let repo_root = repo_root_opt
        .or_else(|| workspace_root_opt.clone())
        .unwrap_or_else(|| cwd.to_path_buf());

    RepoContext {
        cwd: cwd.to_path_buf(),
        repo_root,
        is_workspace: workspace_root_opt.is_some(),
        no_repo,
    }
}

fn list_context_worktrees(context: &RepoContext) -> Result<Vec<Worktree>> {
    if context.is_workspace {
        git::list_workspace_worktrees(&context.repo_root)
    } else {
        git::list_worktrees(&context.repo_root)
    }
}

fn run_tui(cwd: PathBuf, mark_tree: bool) -> Result<()> {
    let mut update_notice_rx = (!mark_tree).then(update::start_background_update_check);
    let mut update_notice = None;

    if mark_tree {
        git::create_workspace_marker(&cwd)?;
    }

    let context = resolve_repo_context(&cwd);
    let no_repo = context.no_repo;
    let repo_root = context.repo_root.clone();

    enable_raw_mode()?;
    let mut stderr = stderr();
    execute!(
        stderr,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let mut cleanup_guard = TuiCleanupGuard::new();
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;
    let mut mouse_capture_enabled = true;

    let mut app = App::new(repo_root.clone());

    if no_repo {
        app.no_repo = true;
        app.worktrees_loading = false;
        app.active_action = ActiveAction::CloneRepo;
    } else {
        if let Err(err) = config::load_repo_config(&repo_root).map(|config| {
            app.repo_config = config;
        }) {
            app.overlay_error = Some(format!("Failed to load options: {err}"));
        }
    }

    if !no_repo && context.is_workspace {
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
    } else if !no_repo {
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

    let run_result = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
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
                || app.rename_loading
                || app.delete_loading
                || app.copy_secrets_loading
                || app.checkout_remote_is_loading()
            {
                app.advance_loading_animation();
            }
            // When a console operation finishes and we're exiting anyway, skip the
            // resume: re-entering the alternate screen would clear the operation's
            // console output and flash the TUI for one frame before quitting.
            if app.console_handoff_needs_resume && !app.should_quit {
                resume_terminal(&app, &mut mouse_capture_enabled, &mut terminal)?;
                app.console_handoff_active = false;
                app.console_handoff_needs_resume = false;
            }

            if !app.console_handoff_active {
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
            }

            if app.sync_pending {
                app.sync_pending = false;
                let wt = app.worktrees.get(app.sync_selected_idx).cloned();
                if let Some(wt) = wt {
                    app.pending_console_operation =
                        Some(PendingConsoleOperation::SyncWorktree { wt });
                }
            }

            if let Some(branch) = app.new_branch_pending.take() {
                let root = app.repo_root.clone();
                let base = app.new_branch_base.clone();
                let result = if app.new_branch_use_existing {
                    git::add_worktree_from_existing(&root, &branch)
                } else {
                    git::add_worktree(&root, &branch, base.as_deref())
                };
                match result {
                    Ok((_, dest)) => complete_new_worktree_creation(
                        &mut app,
                        &mut mouse_capture_enabled,
                        &mut terminal,
                        &branch,
                        base,
                        &dest,
                    )?,
                    Err(e) => {
                        app.new_branch_loading = false;
                        app.new_branch_use_existing = false;
                        app.overlay_error = Some(format!("Failed to create branch: {e}"));
                    }
                }
            }

            if let Some((worktree, branch)) = app.rename_pending.take() {
                match git::rename_worktree(&app.repo_root, &worktree, &branch, app.is_workspace) {
                    Ok(new_path) => {
                        let renamed_current = worktree.is_current;
                        let new_path_str = new_path.to_string_lossy().into_owned();
                        app.rename_loading = false;
                        app.rename_target_idx = None;
                        app.active_action = ActiveAction::None;
                        app.clear_input();
                        refresh_worktrees(&mut app);
                        if let Some(idx) = app
                            .worktrees
                            .iter()
                            .position(|wt| wt.path == new_path_str.as_str())
                        {
                            app.selected_index = app::COMMANDS.len() + idx;
                        }
                        if renamed_current {
                            app.exit_path = Some(new_path.to_string_lossy().into_owned());
                            app.should_quit = true;
                        }
                    }
                    Err(e) => {
                        app.rename_loading = false;
                        app.overlay_error = Some(format!("Failed to rename worktree: {e}"));
                    }
                }
            }

            if let Some(paths) = app.delete_pending.take() {
                let root = app.repo_root.clone();
                match git::remove_worktrees(&root, &paths, app.is_workspace) {
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
                        complete_new_worktree_creation(
                            &mut app,
                            &mut mouse_capture_enabled,
                            &mut terminal,
                            &branch,
                            None,
                            &dest,
                        )?;
                    }
                    Err(e) => {
                        app.checkout_remote_phase = CheckoutRemotePhase::EnterBranch;
                        app.overlay_error = Some(e.to_string());
                    }
                }
            }

            start_pending_console_operation(&mut app, &mut mouse_capture_enabled, &mut terminal)?;

            if app.console_handoff_active {
                std::thread::sleep(Duration::from_millis(100));
            } else if event::poll(Duration::from_millis(100))? {
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

        Ok(())
    }));

    drop(terminal);
    let cleanup_result = cleanup_terminal_state();
    cleanup_guard.disarm();

    match (run_result, cleanup_result) {
        (Err(panic_payload), _) => {
            let message = panic_message(&panic_payload);
            return Err(anyhow::anyhow!(
                "wt crashed while rendering the TUI: {message}"
            ));
        }
        (Ok(Err(run_err)), _) => return Err(run_err),
        (Ok(Ok(())), Err(cleanup_err)) => return Err(cleanup_err),
        (Ok(Ok(())), Ok(())) => {}
    }

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
        if std::env::var_os("WT_SHELL_WRAPPER").is_some() {
            println!("WT_PATH={path}");
            if let Some(ref request) = app.exit_post_create_request {
                println!("WT_POST_CREATE={request}");
            }
        } else {
            println!("{path}");
        }
    }

    Ok(())
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn cleanup_terminal_state() -> Result<()> {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }

    let mut stderr = stderr();
    execute!(
        stderr,
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    )?;
    disable_raw_mode()?;

    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }

    Ok(())
}

fn run_cli_command(cwd: &Path, command: ParsedArgs) -> Result<()> {
    match command {
        ParsedArgs::Clone { repo_source, dest } => {
            let dest = resolve_cli_clone_dest(cwd, &repo_source, dest.as_deref());
            let worktree_path = git::clone_repo_with_layout(&repo_source, &dest)?;
            println!("{}", worktree_path.display());
        }
        ParsedArgs::CheckoutPr { pr_number } => {
            let context = require_repo_context(cwd)?;
            let (_, dest) = git::checkout_pr_as_worktree(&context.repo_root, pr_number)?;
            println!("{}", dest.display());
        }
        ParsedArgs::Checkout { branch_name } => {
            let context = require_repo_context(cwd)?;
            let worktree = resolve_checkout_target(&context, branch_name.as_deref())?;
            println!("{}", worktree.path);
        }
        ParsedArgs::Branch { branch_name, base } => {
            let context = require_repo_context(cwd)?;
            let base_branch = resolve_cli_branch_base(&context, &base)?;
            let (_, dest) =
                git::add_worktree(&context.repo_root, &branch_name, Some(&base_branch))?;
            println!("{}", dest.display());
        }
        ParsedArgs::Delete { branch_name, yes } => {
            let context = require_repo_context(cwd)?;
            let worktree = resolve_delete_target(&context, branch_name.as_deref())?;
            confirm_delete(&worktree, yes)?;
            git::remove_worktree(&context.repo_root, &worktree.path, context.is_workspace)?;
            eprintln!(
                "Removed worktree for branch '{}' at {}",
                worktree.branch, worktree.path
            );
            if worktree.is_current {
                println!("{}", context.repo_root.display());
            }
        }
        ParsedArgs::RunPostCreate { request_file } => {
            config::run_post_create_scripts_from_request(Path::new(&request_file))?;
            eprintln!();
            eprintln!("[wt] Setup complete.");
        }
        ParsedArgs::Tui { .. } | ParsedArgs::Version | ParsedArgs::Update | ParsedArgs::Help => {
            unreachable!("handled before CLI execution")
        }
    }

    Ok(())
}

fn require_repo_context(cwd: &Path) -> Result<RepoContext> {
    let context = resolve_repo_context(cwd);
    if context.no_repo {
        anyhow::bail!(
            "No git repository found here. Run `wt clone <repo>` or `wt` with no args to start the clone flow."
        );
    }
    Ok(context)
}

fn resolve_cli_clone_dest(cwd: &Path, repo_source: &str, dest: Option<&str>) -> PathBuf {
    match dest {
        Some(dest) => {
            let path = PathBuf::from(dest);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        }
        None => PathBuf::from(git::dest_from_url(repo_source, cwd)),
    }
}

fn resolve_cli_branch_base(context: &RepoContext, base: &BranchBase) -> Result<String> {
    match base {
        BranchBase::Auto => git::current_branch(&context.cwd)
            .or_else(|_| git::default_branch(&context.repo_root)),
        BranchBase::Current => git::current_branch(&context.cwd).with_context(|| {
            "Could not resolve the current branch from this directory. Run inside a worktree or use `--from-default` / `--base`."
        }),
        BranchBase::Default => git::default_branch(&context.repo_root),
        BranchBase::Explicit(branch) => Ok(branch.clone()),
    }
}

fn resolve_checkout_target(context: &RepoContext, branch_name: Option<&str>) -> Result<Worktree> {
    let worktrees = list_context_worktrees(context)?;
    match branch_name {
        Some(branch_name) => resolve_worktree_by_branch_in(worktrees, branch_name),
        None => resolve_default_worktree_in(worktrees),
    }
}

fn resolve_worktree_by_branch_in(worktrees: Vec<Worktree>, branch_name: &str) -> Result<Worktree> {
    worktrees
        .into_iter()
        .find(|worktree| worktree.branch == branch_name)
        .ok_or_else(|| anyhow::anyhow!("No worktree found for branch '{branch_name}'."))
}

fn resolve_default_worktree_in(worktrees: Vec<Worktree>) -> Result<Worktree> {
    worktrees
        .into_iter()
        .find(|worktree| worktree.is_main)
        .ok_or_else(|| anyhow::anyhow!("No default-branch worktree found."))
}

fn resolve_delete_target(context: &RepoContext, branch_name: Option<&str>) -> Result<Worktree> {
    let worktrees = list_context_worktrees(context)?;
    let target = if let Some(branch_name) = branch_name {
        resolve_worktree_by_branch_in(worktrees, branch_name)?
    } else {
        worktrees.into_iter().find(|worktree| worktree.is_current).ok_or_else(|| {
            anyhow::anyhow!(
                "Could not determine the current worktree from this directory. Pass a branch name explicitly."
            )
        })?
    };

    if target.is_main {
        anyhow::bail!(
            "Refusing to delete the default worktree '{}'.",
            target.branch
        );
    }

    Ok(target)
}

fn confirm_delete(worktree: &Worktree, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }

    let mut stderr = stderr();
    writeln!(
        stderr,
        "About to delete branch '{}' at {}",
        worktree.branch, worktree.path
    )?;
    write!(stderr, "Type the branch name to confirm: ")?;
    stderr.flush()?;

    let mut confirmation = String::new();
    std::io::stdin().read_line(&mut confirmation)?;
    if confirmation.trim() != worktree.branch {
        anyhow::bail!("Delete cancelled.");
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
        ActiveAction::Rename => handle_rename_key(app, code, modifiers),
        ActiveAction::SyncPr => handle_sync_pr_key(app, code, modifiers),
        ActiveAction::SyncTrees => handle_sync_trees_key(app, code),
        ActiveAction::Delete => handle_delete_key(app, code),
        ActiveAction::CopySecrets => handle_copy_secrets_key(app, code),
        ActiveAction::Options => handle_options_key(app, code, modifiers),
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
    app.rename_loading = false;
    app.rename_target_idx = None;
    app.rename_pending = None;
    app.reset_options_editor();

    if action == ActiveAction::NewBranch {
        app.new_branch_base = app
            .selected_worktree_idx()
            .or_else(|| app.default_worktree_idx())
            .and_then(|wt_idx| app.worktrees.get(wt_idx))
            .map(|wt| wt.branch.clone());
    }

    if action == ActiveAction::Rename {
        let target_idx = app
            .selected_worktree_idx()
            .or_else(|| app.current_worktree_idx());
        let Some(target_idx) = target_idx else {
            app.overlay_error = Some("No worktree available to rename.".to_string());
            app.active_action = ActiveAction::None;
            return;
        };
        let Some(target) = app.worktrees.get(target_idx) else {
            app.overlay_error = Some("No worktree available to rename.".to_string());
            app.active_action = ActiveAction::None;
            return;
        };
        if target.is_main {
            app.overlay_error = Some("The default worktree can't be renamed.".to_string());
            app.active_action = ActiveAction::None;
            return;
        }

        app.rename_loading = false;
        app.rename_target_idx = Some(target_idx);
        app.rename_pending = None;
        app.input_buffer = target.branch.clone();
        app.input_cursor = app.input_buffer.chars().count();
        app.reset_loading_animation();
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

    if action == ActiveAction::Options {
        let max_idx = app.repo_config.post_create_scripts.len().saturating_sub(1);
        app.options_selected_idx = app.options_selected_idx.min(max_idx);
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
            app.pending_console_operation = Some(PendingConsoleOperation::FetchRemote {
                remote: app.checkout_remote_name.clone(),
            });
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
        KeyCode::Enter if !app.worktrees.is_empty() => {
            app.sync_loading = true;
            app.reset_loading_animation();
            app.sync_pending = true;
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

fn begin_options_edit(app: &mut App, index: Option<usize>) {
    app.options_phase = OptionsPhase::Editing;
    app.options_edit_idx = index;
    app.overlay_error = None;
    match index {
        Some(idx) => {
            app.input_buffer = app.repo_config.post_create_scripts[idx].command.clone();
            app.input_cursor = app.input_buffer.chars().count();
        }
        None => app.clear_input(),
    }
}

fn handle_options_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.options_phase == OptionsPhase::Editing {
        match text_input::handle_key(app, code, modifiers) {
            TextInputKeyResult::Cancel => {
                app.reset_options_editor();
                app.overlay_error = None;
            }
            TextInputKeyResult::Submit => {
                let command = app.input_buffer.trim().to_string();
                if command.is_empty() {
                    app.overlay_error = Some("Command can't be empty.".to_string());
                    return;
                }

                let edit_idx = app.options_edit_idx;
                let save_result = update_repo_config(app, |config| match edit_idx {
                    Some(idx) => config.post_create_scripts[idx].command = command.clone(),
                    None => config.post_create_scripts.push(PostCreateScript {
                        command: command.clone(),
                        enabled: true,
                    }),
                });

                match save_result {
                    Ok(()) => {
                        if edit_idx.is_none() {
                            app.options_selected_idx =
                                app.repo_config.post_create_scripts.len().saturating_sub(1);
                        }
                        app.reset_options_editor();
                        app.overlay_error = None;
                    }
                    Err(err) => {
                        app.overlay_error =
                            Some(format!("Failed to save post-create scripts: {err}"));
                    }
                }
            }
            TextInputKeyResult::Updated
            | TextInputKeyResult::Ignored
            | TextInputKeyResult::Complete => {}
        }
        return;
    }

    let script_count = app.repo_config.post_create_scripts.len();
    let has_scripts = script_count > 0;

    match code {
        KeyCode::Esc => {
            app.active_action = ActiveAction::None;
            app.reset_options_editor();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.options_selected_idx = app.options_selected_idx.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.options_selected_idx =
                (app.options_selected_idx + 1).min(script_count.saturating_sub(1));
        }
        KeyCode::Char('a') => begin_options_edit(app, None),
        KeyCode::Enter | KeyCode::Char('e') => {
            if has_scripts {
                begin_options_edit(app, Some(app.options_selected_idx));
            } else {
                begin_options_edit(app, None);
            }
        }
        KeyCode::Char(' ') => {
            if !has_scripts {
                return;
            }
            let idx = app.options_selected_idx;
            if let Err(err) = update_repo_config(app, |config| {
                config.post_create_scripts[idx].enabled = !config.post_create_scripts[idx].enabled;
            }) {
                app.overlay_error = Some(format!("Failed to save post-create scripts: {err}"));
            } else {
                app.overlay_error = None;
            }
        }
        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
            if !has_scripts {
                return;
            }
            let idx = app.options_selected_idx;
            match update_repo_config(app, |config| {
                config.post_create_scripts.remove(idx);
            }) {
                Ok(()) => {
                    app.options_selected_idx = app
                        .options_selected_idx
                        .min(app.repo_config.post_create_scripts.len().saturating_sub(1));
                    app.overlay_error = None;
                }
                Err(err) => {
                    app.overlay_error = Some(format!("Failed to save post-create scripts: {err}"));
                }
            }
        }
        _ => {}
    }
}

fn handle_rename_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    if app.rename_loading {
        return;
    }

    match text_input::handle_key(app, code, modifiers) {
        TextInputKeyResult::Cancel => {
            app.active_action = ActiveAction::None;
            app.rename_target_idx = None;
            app.rename_pending = None;
            app.overlay_error = None;
            app.clear_input();
        }
        TextInputKeyResult::Submit => {
            let branch = app.input_buffer.trim().to_string();
            let Some(target_idx) = app.rename_target_idx else {
                app.overlay_error = Some("No worktree selected for rename.".to_string());
                return;
            };
            let Some(worktree) = app.worktrees.get(target_idx).cloned() else {
                app.overlay_error = Some("No worktree selected for rename.".to_string());
                return;
            };
            if branch.is_empty() {
                return;
            }
            if app
                .worktrees
                .iter()
                .enumerate()
                .any(|(idx, wt)| idx != target_idx && wt.branch == branch)
            {
                app.overlay_error = Some(format!("'{branch}' is already checked out."));
                return;
            }

            app.overlay_error = None;
            app.reset_loading_animation();
            app.rename_loading = true;
            app.rename_pending = Some((worktree, branch));
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
            let pr_number: u32 = if let Ok(n) = pr_input.parse() {
                n
            } else {
                app.overlay_error = Some("Invalid PR number. Use #123 or 123.".to_string());
                return;
            };

            app.overlay_error = None;
            app.sync_pr_loading = true;
            app.clear_sync_pr_output();
            app.reset_loading_animation();
            app.pending_console_operation = Some(PendingConsoleOperation::SyncPr { pr_number });
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
                        matches!(event, SyncPrEvent::Finished { .. } | SyncPrEvent::Error(_));
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
            SyncPrEvent::Finished {
                worktree_path,
                branch,
                base_branch,
                created,
            } => {
                app.sync_pr_loading = false;
                app.active_action = ActiveAction::None;
                app.clear_input();
                refresh_worktrees(app);
                select_worktree_by_path(app, &worktree_path);
                if created {
                    run_post_create_during_handoff(app, &branch, base_branch, &worktree_path);
                } else {
                    app.exit_path = Some(worktree_path.to_string_lossy().into_owned());
                    app.should_quit = true;
                }
            }
            SyncPrEvent::Error(err) => {
                app.console_handoff_needs_resume |= app.console_handoff_active;
                app.sync_pr_loading = false;
                app.overlay_error = Some(format!("Failed to sync PR: {err}"));
            }
        }
    }

    if disconnected && app.sync_pr_loading {
        app.console_handoff_needs_resume |= app.console_handoff_active;
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
        app.console_handoff_needs_resume |= app.console_handoff_active;
        app.sync_fetch_ok = fetch_ok;
        app.sync_results = vec![result];
        app.sync_loading = false;
        app.sync_receiver = None;
        refresh_worktrees(app);
    } else if disconnected && app.sync_loading {
        app.console_handoff_needs_resume |= app.console_handoff_active;
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
    app.delete_confirm_targets
        .iter()
        .copied()
        .any(|idx| app.worktrees.get(idx).is_some_and(|wt| wt.is_current))
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
                app.reset_loading_animation();
                app.pending_console_operation = Some(PendingConsoleOperation::CloneRepo {
                    url: app.clone_url.clone(),
                    dest: PathBuf::from(input),
                });
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
            CloneEvent::Finished(worktree_path) => {
                app.clone_loading = false;
                if app.console_handoff_active {
                    let display_path = display_path_with_home(&worktree_path);
                    eprintln!();
                    eprintln!(
                        "{} Cloned into {}",
                        "[wt]".green().bold(),
                        display_path.green()
                    );
                }
                app.exit_path = Some(worktree_path.to_string_lossy().into_owned());
                app.should_quit = true;
            }
            CloneEvent::Error(err) => {
                app.console_handoff_needs_resume |= app.console_handoff_active;
                app.clone_loading = false;
                app.clone_error = Some(err);
            }
        }
    }

    if disconnected && app.clone_loading {
        app.console_handoff_needs_resume |= app.console_handoff_active;
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
                app.pending_console_operation =
                    Some(PendingConsoleOperation::FetchRemote { remote });
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
        app.console_handoff_needs_resume |= app.console_handoff_active;
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

    use super::{handle_paste, open_action, resolve_worktree_by_branch_in};
    use crate::{
        app::App,
        text_input,
        types::{ActiveAction, CheckoutRemotePhase, OptionsPhase, Worktree},
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
        app.active_action = ActiveAction::Rename;
        assert!(text_input::is_active(&app));
        app.rename_loading = true;
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

        app = test_app();
        app.active_action = ActiveAction::Options;
        app.options_phase = OptionsPhase::Editing;
        assert!(text_input::is_active(&app));
        app.options_phase = OptionsPhase::BrowsingScripts;
        assert!(!text_input::is_active(&app));
    }

    #[test]
    fn handle_paste_inserts_into_all_text_input_overlays() {
        let editable_states = [
            (ActiveAction::NewBranch, CheckoutRemotePhase::SelectRemote),
            (ActiveAction::Rename, CheckoutRemotePhase::SelectRemote),
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

        let mut app = test_app();
        app.active_action = ActiveAction::Options;
        app.options_phase = OptionsPhase::Editing;

        handle_paste(&mut app, "pnpm i\n");

        assert_eq!(app.input_buffer, "pnpm i");
        assert_eq!(app.input_cursor, "pnpm i".chars().count());
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

    #[test]
    fn resolve_worktree_by_branch_matches_existing_branch() {
        let worktree = resolve_worktree_by_branch_in(
            vec![
                Worktree {
                    path: "/repo/main".to_string(),
                    branch: "main".to_string(),
                    is_main: true,
                    is_current: false,
                    has_secrets: false,
                },
                Worktree {
                    path: "/repo/feature-test".to_string(),
                    branch: "feature/test".to_string(),
                    is_main: false,
                    is_current: true,
                    has_secrets: false,
                },
            ],
            "feature/test",
        )
        .expect("branch should resolve");

        assert_eq!(worktree.path, "/repo/feature-test");
    }

    #[test]
    fn resolve_worktree_by_branch_errors_for_missing_branch() {
        let err = resolve_worktree_by_branch_in(Vec::new(), "feature/test")
            .expect_err("missing branch should fail");

        assert_eq!(
            err.to_string(),
            "No worktree found for branch 'feature/test'."
        );
    }

    #[test]
    fn new_branch_uses_default_worktree_base_when_command_is_selected() {
        let mut app = test_app();
        app.worktrees = vec![
            Worktree {
                path: "/repo/main".to_string(),
                branch: "main".to_string(),
                is_main: true,
                is_current: false,
                has_secrets: false,
            },
            Worktree {
                path: "/repo/feature-test".to_string(),
                branch: "feature/test".to_string(),
                is_main: false,
                is_current: true,
                has_secrets: false,
            },
        ];
        app.selected_index = 0;

        open_action(&mut app, ActiveAction::NewBranch);

        assert_eq!(app.new_branch_base.as_deref(), Some("main"));
    }

    #[test]
    fn new_branch_prefers_selected_worktree_base() {
        let mut app = test_app();
        app.worktrees = vec![
            Worktree {
                path: "/repo/main".to_string(),
                branch: "main".to_string(),
                is_main: true,
                is_current: false,
                has_secrets: false,
            },
            Worktree {
                path: "/repo/feature-test".to_string(),
                branch: "feature/test".to_string(),
                is_main: false,
                is_current: true,
                has_secrets: false,
            },
        ];
        app.selected_index = crate::app::COMMANDS.len() + 1;

        open_action(&mut app, ActiveAction::NewBranch);

        assert_eq!(app.new_branch_base.as_deref(), Some("feature/test"));
    }
}
