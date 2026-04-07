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
    Delete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActivePanel {
    Sidebar,
    Console,
}

