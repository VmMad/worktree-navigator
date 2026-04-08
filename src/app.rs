use std::path::PathBuf;

use crate::types::{ActiveAction, PullRequest, SyncResult, Worktree};

pub const COMMANDS: &[(&str, &str)] = &[
    ("New Branch", "n"),
    ("Sync GH PR", "p"),
    ("Delete Worktree", "d"),
    ("Sync Trees", "s"),
    ("Refresh List", "r"),
];

pub struct App {
    pub repo_root: PathBuf,
    pub no_repo: bool,
    pub worktrees: Vec<Worktree>,
    pub worktrees_loading: bool,
    pub worktrees_error: Option<String>,

    pub prs: Vec<PullRequest>,
    pub prs_loading: bool,
    pub prs_error: Option<String>,

    pub sync_selected_idx: usize,
    pub sync_loading: bool,
    pub sync_pending: bool,
    pub sync_fetch_ok: bool,
    pub sync_results: Vec<SyncResult>,

    pub clone_step: u8,
    pub clone_url: String,
    pub clone_loading: bool,
    pub clone_pending: bool,
    pub clone_error: Option<String>,

    pub selected_index: usize,
    pub active_action: ActiveAction,

    pub input_buffer: String,
    pub input_cursor: usize,
    pub overlay_index: usize,
    pub delete_confirming: bool,
    pub overlay_error: Option<String>,

    pub exit_path: Option<String>,
    pub should_quit: bool,

    /// Maps screen row → item index, populated each render frame for mouse hit detection.
    pub item_rows: Vec<(u16, usize)>,
    /// Screen row currently under the mouse cursor (for hover highlight).
    pub hovered_row: Option<u16>,
}

impl App {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            no_repo: false,
            worktrees: vec![],
            worktrees_loading: true,
            worktrees_error: None,
            prs: vec![],
            prs_loading: false,
            prs_error: None,
            sync_selected_idx: 0,
            sync_loading: false,
            sync_pending: false,
            sync_fetch_ok: true,
            sync_results: vec![],
            clone_step: 0,
            clone_url: String::new(),
            clone_loading: false,
            clone_pending: false,
            clone_error: None,
            selected_index: 0,
            active_action: ActiveAction::None,
            input_buffer: String::new(),
            input_cursor: 0,
            overlay_index: 0,
            delete_confirming: false,
            overlay_error: None,
            exit_path: None,
            should_quit: false,
            item_rows: vec![],
            hovered_row: None,
        }
    }

    pub fn total_items(&self) -> usize {
        COMMANDS.len() + self.worktrees.len()
    }

    pub fn deletable_worktrees(&self) -> Vec<&Worktree> {
        self.worktrees
            .iter()
            .filter(|wt| !wt.is_main && !wt.is_current)
            .collect()
    }

    /// Returns the item index for the given screen row, if any.
    pub fn row_to_item(&self, row: u16) -> Option<usize> {
        self.item_rows.iter().find(|(r, _)| *r == row).map(|(_, idx)| *idx)
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
