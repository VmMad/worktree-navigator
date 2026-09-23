use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::projects::Project;
use crate::types::Worktree;

const STATE_FILE: &str = "worktree-navigator/state.json";
/// Earlier builds kept only the projects, in this file.
const LEGACY_PROJECTS_FILE: &str = "projects.json";

/// Kept between runs so the TUI draws before it touches the filesystem or git.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub worktrees: BTreeMap<PathBuf, Vec<Worktree>>,
}

fn state_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WT_STATE_FILE") {
        return Ok(PathBuf::from(path));
    }

    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .context("Could not resolve a config directory; set HOME or XDG_CONFIG_HOME.")?;

    Ok(config_home.join(STATE_FILE))
}

pub fn load() -> Result<State> {
    let path = state_path()?;
    let legacy = path.with_file_name(LEGACY_PROJECTS_FILE);
    let raw = match fs::read_to_string(&path).or_else(|_| fs::read_to_string(&legacy)) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(err) => return Err(err).with_context(|| format!("Failed to read {}", path.display())),
    };

    serde_json::from_str(&raw).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Writes through a temp file and a rename, so a concurrent reader never sees a partial file.
// ponytail: the last concurrent writer wins. Add a file lock if that ever loses data.
pub fn save(state: &State) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let data = serde_json::to_string_pretty(state).context("Failed to serialize state")?;
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    fs::write(&tmp, data).with_context(|| format!("Failed to write {}", tmp.display()))?;
    fs::rename(&tmp, &path).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

pub fn cached_worktrees(repo_root: &Path, cwd: &Path) -> Option<Vec<Worktree>> {
    let mut worktrees = load().ok()?.worktrees.remove(repo_root)?;
    for worktree in &mut worktrees {
        worktree.is_current = Path::new(&worktree.path)
            .canonicalize()
            .is_ok_and(|path| path == cwd);
    }
    Some(worktrees)
}

pub fn save_worktrees(repo_root: &Path, worktrees: &[Worktree]) {
    let mut state = load().unwrap_or_default();
    state
        .worktrees
        .insert(repo_root.to_path_buf(), worktrees.to_vec());
    let _ = save(&state);
}
