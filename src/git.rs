use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::{SyncResult, SyncStatus, Worktree};

pub fn list_worktrees(repo_root: &Path) -> Result<Vec<Worktree>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
        .context("Failed to run git worktree list")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let cwd = cwd.canonicalize().unwrap_or(cwd);

    parse_worktree_porcelain(&stdout, &cwd)
}

fn parse_worktree_porcelain(raw: &str, cwd: &Path) -> Result<Vec<Worktree>> {
    let mut worktrees = Vec::new();

    for block in raw.trim().split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let path_line = lines.iter().find(|l| l.starts_with("worktree "));
        let head_line = lines.iter().find(|l| l.starts_with("HEAD "));
        let branch_line = lines.iter().find(|l| l.starts_with("branch "));
        let is_bare = lines.iter().any(|l| *l == "bare");

        let Some(path_str) = path_line.map(|l| l.trim_start_matches("worktree ")) else {
            continue;
        };
        if is_bare {
            continue;
        }

        let path = PathBuf::from(path_str);
        let sha = head_line
            .map(|l| &l["HEAD ".len()..][..7.min(l.len() - "HEAD ".len())])
            .unwrap_or("unknown")
            .to_string();

        let branch = branch_line
            .map(|l| {
                l.trim_start_matches("branch ")
                    .trim_start_matches("refs/heads/")
                    .to_string()
            })
            .unwrap_or_else(|| "HEAD".to_string());

        let is_main = worktrees.is_empty();
        let is_current = path.canonicalize().unwrap_or(path.clone()) == cwd;

        worktrees.push(Worktree {
            path: path_str.to_string(),
            branch,
            sha,
            is_main,
            is_current,
        });
    }

    Ok(worktrees)
}

pub fn add_worktree(repo_root: &Path, branch_name: &str) -> Result<Vec<String>> {
    let mut messages = Vec::new();

    let parent = repo_root
        .parent()
        .context("Repo root has no parent directory")?;
    let repo_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");

    let sanitized = branch_name.replace('/', "-");
    let dest = parent.join(format!("{repo_name}-{sanitized}"));
    let dest_str = dest.to_string_lossy().to_string();

    messages.push(format!("$ git worktree add {dest_str} -b {branch_name}"));

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, "-b", branch_name])
        .current_dir(repo_root)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        messages.push(format!("✓ Created worktree at {dest_str}"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        messages.push(format!("✗ {}", stderr.trim()));
    }

    Ok(messages)
}

pub fn remove_worktree(repo_root: &Path, worktree_path: &str) -> Result<Vec<String>> {
    let mut messages = Vec::new();
    messages.push(format!("$ git worktree remove --force {worktree_path}"));

    let output = Command::new("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .current_dir(repo_root)
        .output()
        .context("Failed to run git worktree remove")?;

    if output.status.success() {
        messages.push(format!("✓ Removed worktree at {worktree_path}"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        messages.push(format!("✗ {}", stderr.trim()));
    }

    Ok(messages)
}

pub fn checkout_pr_as_worktree(
    repo_root: &Path,
    pr_number: u32,
    branch_name: &str,
) -> Result<Vec<String>> {
    let mut messages = Vec::new();

    // Fetch the remote branch first
    messages.push(format!(
        "$ git fetch origin {branch_name}:{branch_name}"
    ));
    let fetch = Command::new("git")
        .args(["fetch", "origin", &format!("{branch_name}:{branch_name}")])
        .current_dir(repo_root)
        .output();

    match fetch {
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Non-fatal: branch may already exist locally
            messages.push(format!("  (fetch note: {})", stderr.trim()));
        }
        Err(e) => messages.push(format!("  (fetch warn: {e})")),
        _ => {}
    }

    let parent = repo_root
        .parent()
        .context("Repo root has no parent directory")?;
    let repo_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo");

    let dest = parent.join(format!("{repo_name}-pr-{pr_number}"));
    let dest_str = dest.to_string_lossy().to_string();

    messages.push(format!("$ git worktree add {dest_str} {branch_name}"));

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, branch_name])
        .current_dir(repo_root)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        messages.push(format!(
            "✓ PR #{pr_number} checked out at {dest_str}"
        ));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        messages.push(format!("✗ {}", stderr.trim()));
    }

    Ok(messages)
}

/// Fetch from all remotes then fast-forward a single worktree to origin/<branch>.
/// Returns (fetch_succeeded, SyncResult).
pub fn sync_one_worktree(repo_root: &Path, wt: &Worktree) -> (bool, SyncResult) {
    let fetch_ok = Command::new("git")
        .args(["fetch", "--all", "--quiet"])
        .current_dir(repo_root)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let remote_ref = format!("origin/{}", wt.branch);

    // Check that origin/<branch> exists before attempting the merge.
    let ref_exists = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(&wt.path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !ref_exists {
        return (
            fetch_ok,
            SyncResult {
                branch: wt.branch.clone(),
                status: SyncStatus::Skipped(format!("{remote_ref} not found on remote")),
            },
        );
    }

    let out = Command::new("git")
        .args(["merge", "--ff-only", &remote_ref])
        .current_dir(&wt.path)
        .output();

    let status = match out {
        Err(e) => SyncStatus::Error(e.to_string()),
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();

            if o.status.success() {
                if stdout.contains("Already up to date") {
                    SyncStatus::UpToDate
                } else {
                    let range = stdout
                        .lines()
                        .find(|l| l.starts_with("Updating "))
                        .map(|l| l.trim_start_matches("Updating ").trim().to_string())
                        .unwrap_or_default();
                    SyncStatus::Updated(range)
                }
            } else if stderr.contains("uncommitted changes")
                || stderr.contains("local changes")
                || stderr.contains("not possible to fast-forward")
                || stderr.contains("You have unstaged changes")
            {
                SyncStatus::Skipped("dirty working tree".to_string())
            } else {
                SyncStatus::Error(stderr)
            }
        }
    };

    (fetch_ok, SyncResult { branch: wt.branch.clone(), status })
}

/// Walk up from cwd to find the git repo root.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    // Ask git directly — works for normal, bare, and worktree checkouts.
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(start)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let git_common_dir = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    // --git-common-dir returns "." when cwd is a bare repo root.
    let git_dir = if git_common_dir == "." {
        start.to_path_buf()
    } else {
        PathBuf::from(&git_common_dir)
    };

    let git_dir = git_dir.canonicalize().unwrap_or(git_dir);

    // For a normal repo the git dir is <root>/.git — return the parent.
    // For a bare repo the git dir IS the root.
    if git_dir.file_name().map(|n| n == ".git").unwrap_or(false) {
        git_dir.parent().map(PathBuf::from)
    } else {
        Some(git_dir)
    }
}
