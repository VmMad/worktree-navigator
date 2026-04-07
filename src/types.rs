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

#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    pub text: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageKind {
    Info,
    Command,
    Success,
    Error,
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

impl ConsoleMessage {
    pub fn info(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: MessageKind::Info }
    }
    pub fn command(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: MessageKind::Command }
    }
    pub fn success(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: MessageKind::Success }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: MessageKind::Error }
    }
}
