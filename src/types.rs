use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub is_main: bool,
    pub is_current: bool,
    pub has_secrets: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveAction {
    None,
    NewBranch,
    Rename,
    SyncPr,
    SyncTrees,
    Delete,
    CopySecrets,
    Options,
    CloneRepo,
    CheckoutRemote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionsPhase {
    BrowsingScripts,
    Editing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutRemotePhase {
    SelectRemote,
    FetchingRemote,
    EnterBranch,
    CreatingWorktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Finished(PathBuf),
    Error(String),
}

#[derive(Debug)]
pub enum SyncPrEvent {
    Progress {
        line: String,
    },
    Finished {
        worktree_path: PathBuf,
        branch: String,
        base_branch: Option<String>,
        created: bool,
    },
    Error(String),
}
