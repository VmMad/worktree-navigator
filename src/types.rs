use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_current: bool,
    pub has_secrets: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActiveAction {
    None,
    NewBranch,
    SyncPr,
    SyncTrees,
    Delete,
    CopySecrets,
    CloneRepo,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CopySecretsPhase {
    SelectSource,
    SelectTarget,
    ConfirmOverwrite,
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

#[derive(Debug)]
pub enum CloneEvent {
    Progress { line: String },
    Finished(PathBuf),
    Error(String),
}

#[derive(Debug)]
pub enum SyncPrEvent {
    Progress { line: String },
    Finished(PathBuf),
    Error(String),
}
