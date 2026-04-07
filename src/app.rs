use std::path::PathBuf;

use crate::types::{ActiveAction, ActivePanel, ConsoleMessage, PullRequest, Worktree};

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

    pub messages: Vec<ConsoleMessage>,

    pub sidebar_index: usize,
    pub active_panel: ActivePanel,
    pub active_action: ActiveAction,

    /// Text buffer for the "new branch" input
    pub input_buffer: String,
    /// Cursor position inside input_buffer (char index)
    pub input_cursor: usize,

    /// Selection index inside action overlays (PR list, delete list)
    pub overlay_index: usize,
    /// Waiting for y/n confirmation before delete
    pub delete_confirming: bool,

    /// When set, the app should exit and the shell should cd here
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
            messages: vec![ConsoleMessage::info(
                "Worktree Navigator ready. Use ↑↓ to navigate, Enter to select.",
            )],
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

    pub fn log(&mut self, msg: ConsoleMessage) {
        self.messages.push(msg);
    }

    pub fn log_lines(&mut self, lines: Vec<String>) {
        for line in lines {
            let kind = if line.starts_with("✓") {
                crate::types::MessageKind::Success
            } else if line.starts_with("✗") {
                crate::types::MessageKind::Error
            } else {
                crate::types::MessageKind::Command
            };
            self.messages.push(ConsoleMessage { text: line, kind });
        }
    }

    // ──────────────────────────────── input handling ─────────────────────────

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
