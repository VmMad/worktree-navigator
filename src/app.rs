use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use crate::types::{
    ActiveAction, CloneEvent, CopySecretsPhase, SyncResult, Worktree,
};

pub const COMMANDS: &[(&str, &str)] = &[
    ("New Branch", "n"),
    ("Sync GH PR", "p"),
    ("Delete Worktree", "d"),
    ("Sync Trees", "s"),
    ("Copy Secrets", "c"),
];

pub struct App {
    pub repo_root: PathBuf,
    pub no_repo: bool,
    pub is_workspace: bool,
    pub worktrees: Vec<Worktree>,
    pub worktrees_loading: bool,
    pub worktrees_error: Option<String>,

    pub sync_selected_idx: usize,
    pub sync_pr_loading: bool,
    pub sync_pr_pending: Option<u32>,
    pub sync_loading: bool,
    pub sync_pending: bool,
    pub sync_fetch_ok: bool,
    pub sync_results: Vec<SyncResult>,

    pub new_branch_loading: bool,
    pub new_branch_pending: Option<String>,

    pub delete_loading: bool,
    pub delete_pending: Option<String>,

    pub copy_secrets_loading: bool,
    pub copy_secrets_pending: bool,

    pub clone_step: u8,
    pub clone_url: String,
    pub clone_loading: bool,
    pub clone_receiver: Option<Receiver<CloneEvent>>,
    pub clone_animation_frame: usize,
    pub clone_error: Option<String>,

    pub selected_index: usize,
    pub active_action: ActiveAction,

    pub input_buffer: String,
    pub input_cursor: usize,
    pub overlay_index: usize,
    pub delete_confirming: bool,
    pub overlay_error: Option<String>,
    pub copy_secrets_phase: CopySecretsPhase,
    pub copy_secrets_source_idx: Option<usize>,
    pub copy_secrets_target_idx: usize,
    pub copy_secrets_confirm_yes: bool,

    pub exit_path: Option<String>,
    pub should_quit: bool,

    /// Maps screen row → item index, populated each render frame for mouse hit detection.
    pub item_rows: Vec<(u16, usize)>,
    /// Screen row currently under the mouse cursor (for hover highlight).
    pub hovered_row: Option<u16>,
    pub frame_width: u16,
    pub frame_height: u16,
}

impl App {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            no_repo: false,
            is_workspace: false,
            worktrees: vec![],
            worktrees_loading: true,
            worktrees_error: None,
            sync_selected_idx: 0,
            sync_pr_loading: false,
            sync_pr_pending: None,
            sync_loading: false,
            sync_pending: false,
            sync_fetch_ok: true,
            sync_results: vec![],
            new_branch_loading: false,
            new_branch_pending: None,
            delete_loading: false,
            delete_pending: None,
            copy_secrets_loading: false,
            copy_secrets_pending: false,
            clone_step: 0,
            clone_url: String::new(),
            clone_loading: false,
            clone_receiver: None,
            clone_animation_frame: 0,
            clone_error: None,
            selected_index: 0,
            active_action: ActiveAction::None,
            input_buffer: String::new(),
            input_cursor: 0,
            overlay_index: 0,
            delete_confirming: false,
            overlay_error: None,
            copy_secrets_phase: CopySecretsPhase::SelectSource,
            copy_secrets_source_idx: None,
            copy_secrets_target_idx: 0,
            copy_secrets_confirm_yes: true,
            exit_path: None,
            should_quit: false,
            item_rows: vec![],
            hovered_row: None,
            frame_width: 0,
            frame_height: 0,
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

    pub fn next_copy_target_idx(&self, from: usize) -> Option<usize> {
        self.worktrees
            .iter()
            .enumerate()
            .find_map(|(idx, _)| (idx != from).then_some(idx))
    }

    /// Returns the item index for the given screen row, if any.
    pub fn row_to_item(&self, row: u16) -> Option<usize> {
        self.item_rows
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, idx)| *idx)
    }

    fn input_byte_index(&self) -> usize {
        self.input_buffer
            .char_indices()
            .nth(self.input_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input_buffer.len())
    }

    fn input_len(&self) -> usize {
        self.input_buffer.chars().count()
    }

    pub fn input_char(&mut self, c: char) {
        let byte_pos = self.input_byte_index();
        self.input_buffer.insert(byte_pos, c);
        self.input_cursor += 1;
    }

    pub fn input_str(&mut self, s: &str) {
        let byte_pos = self.input_byte_index();
        self.input_buffer.insert_str(byte_pos, s);
        self.input_cursor += s.chars().count();
    }

    pub fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
            let byte_pos = self.input_byte_index();
            self.input_buffer.remove(byte_pos);
        }
    }

    pub fn input_delete(&mut self) {
        if self.input_cursor < self.input_len() {
            let byte_pos = self.input_byte_index();
            self.input_buffer.remove(byte_pos);
        }
    }

    pub fn input_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    pub fn input_right(&mut self) {
        self.input_cursor = (self.input_cursor + 1).min(self.input_len());
    }

    pub fn input_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn input_end(&mut self) {
        self.input_cursor = self.input_len();
    }

    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    pub fn reset_clone_animation(&mut self) {
        self.clone_animation_frame = 0;
    }

    pub fn advance_loading_animation(&mut self) {
        self.clone_animation_frame = (self.clone_animation_frame + 1) % 3;
    }

    pub fn clone_animation_dots(&self) -> &'static str {
        match self.clone_animation_frame {
            0 => ".  ",
            1 => ".. ",
            _ => "...",
        }
    }
}
