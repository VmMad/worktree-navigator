use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub sha: String,
    pub is_main: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveAction {
    None,
    NewBranch,
    SyncPr,
    SyncTrees,
    Delete,
    CloneRepo,
}

#[derive(Debug, Clone)]
pub enum SyncStatus {
    UpToDate,
    Updated(String), // commit range, e.g. "a1b2c3..d4e5f6"
    Skipped(String), // reason (dirty, no upstream, etc.)
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub branch: String,
    pub status: SyncStatus,
}

#[derive(Debug, Clone)]
pub struct CloneProgress {
    pub phase: String,
    pub detail: Option<String>,
    pub ratio: f64,
}

impl CloneProgress {
    pub fn new(phase: impl Into<String>, detail: Option<String>, ratio: f64) -> Self {
        Self {
            phase: phase.into(),
            detail,
            ratio: ratio.clamp(0.0, 1.0),
        }
    }
}

impl Default for CloneProgress {
    fn default() -> Self {
        Self::new("Preparing clone…", None, 0.0)
    }
}

#[derive(Debug)]
pub enum CloneEvent {
    Progress(CloneProgress),
    Finished(PathBuf),
    Error(String),
}
