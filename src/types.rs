#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub sha: String,
    pub is_main: bool,
    pub is_current: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    pub head_ref_name: String,
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
    Updated(String),  // commit range, e.g. "a1b2c3..d4e5f6"
    Skipped(String),  // reason (dirty, no upstream, etc.)
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub branch: String,
    pub status: SyncStatus,
}
