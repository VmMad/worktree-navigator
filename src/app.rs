use std::collections::HashMap;
use std::path::PathBuf;

use crate::pty::PtySession;
use crate::types::{ActiveAction, ActivePanel, PullRequest, Worktree};

pub const COMMANDS: &[(&str, &str)] = &[
    ("New Branch", "n"),
    ("Sync GH PR", "p"),
    ("Delete Worktree", "d"),
    ("Refresh", "r"),
];

pub struct App {
    pub repo_root: PathBuf,
    pub worktrees: Vec<Worktree>,
    pub worktrees_loading: bool,
    pub worktrees_error: Option<String>,

    pub prs: Vec<PullRequest>,
    pub prs_loading: bool,
    pub prs_error: Option<String>,

    /// Per-worktree PTY sessions (keyed by worktree path)
    pub pty_sessions: HashMap<String, PtySession>,
    /// Path of the worktree whose shell is displayed in the right panel
    pub active_pty_path: Option<String>,
    /// Current console panel size (cols, rows) for PTY sizing
    pub console_size: (u16, u16),

    pub sidebar_index: usize,
    pub active_panel: ActivePanel,
    pub active_action: ActiveAction,

    pub input_buffer: String,
    pub input_cursor: usize,
    pub overlay_index: usize,
    pub delete_confirming: bool,

    pub exit_path: Option<String>,
    pub should_quit: bool,
}

impl App {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            worktrees: vec![],
            worktrees_loading: true,
            worktrees_error: None,
            prs: vec![],
            prs_loading: false,
            prs_error: None,
            pty_sessions: HashMap::new(),
            active_pty_path: None,
            console_size: (120, 40),
            sidebar_index: 0,
            active_panel: ActivePanel::Sidebar,
            active_action: ActiveAction::None,
            input_buffer: String::new(),
            input_cursor: 0,
            overlay_index: 0,
            delete_confirming: false,
            exit_path: None,
            should_quit: false,
        }
    }

    pub fn total_sidebar_items(&self) -> usize {
        COMMANDS.len() + self.worktrees.len()
    }

    pub fn deletable_worktrees(&self) -> Vec<&Worktree> {
        self.worktrees
            .iter()
            .filter(|wt| !wt.is_main && !wt.is_current)
            .collect()
    }

    /// Open (or reuse) a PTY session for the given worktree and focus it.
    /// Returns an error message if the PTY could not be spawned.
    pub fn open_shell(&mut self, worktree_path: &str) -> Result<(), String> {
        let (cols, rows) = self.console_size;
        if !self.pty_sessions.contains_key(worktree_path) {
            match PtySession::spawn(worktree_path, cols, rows) {
                Ok(session) => {
                    self.pty_sessions.insert(worktree_path.to_string(), session);
                }
                Err(err) => {
                    return Err(format!(
                        "failed to open shell for '{}': {}",
                        worktree_path, err
                    ));
                }
            }
        }
        self.active_pty_path = Some(worktree_path.to_string());
        self.active_panel = ActivePanel::Console;
        Ok(())
    }

    /// Send bytes to the active PTY.
    pub fn send_to_pty(&mut self, data: &[u8]) {
        if let Some(path) = self.active_pty_path.as_deref()
            && let Some(session) = self.pty_sessions.get_mut(path)
        {
            session.write_input(data);
        }
    }

    /// Resize all open PTY sessions to new console dimensions.
    pub fn resize_ptys(&mut self, cols: u16, rows: u16) {
        self.console_size = (cols, rows);
        for session in self.pty_sessions.values_mut() {
            session.resize(cols, rows);
        }
    }

    pub fn input_char(&mut self, c: char) {
        let byte_pos = self
            .input_buffer
            .char_indices()
            .nth(self.input_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input_buffer.len());
        self.input_buffer.insert(byte_pos, c);
        self.input_cursor += 1;
    }

    pub fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            let byte_pos = self
                .input_buffer
                .char_indices()
                .nth(self.input_cursor)
                .map(|(i, _)| i)
                .unwrap_or(self.input_buffer.len());
            self.input_buffer.remove(byte_pos);
        }
    }

    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }
}

