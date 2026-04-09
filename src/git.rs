use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

    let sanitized = branch_name.replace('/', "-");
    let dest = worktree_base_dir(repo_root).join(&sanitized);
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

pub fn checkout_pr_as_worktree(repo_root: &Path, pr_number: u32) -> Result<Vec<String>> {
    let mut messages = Vec::new();

    let pr_ref = format!("#{pr_number}");
    let pr_info = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_ref,
            "--json",
            "headRefName",
            "-q",
            ".headRefName",
        ])
        .current_dir(repo_root)
        .output()
        .context("Failed to run gh pr view")?;

    if !pr_info.status.success() {
        let stderr = String::from_utf8_lossy(&pr_info.stderr);
        anyhow::bail!("{}", stderr.trim());
    }

    let branch_name = String::from_utf8_lossy(&pr_info.stdout).trim().to_string();
    if branch_name.is_empty() {
        anyhow::bail!("Could not resolve head branch for PR #{pr_number}");
    }

    // Fetch the remote branch first
    messages.push(format!("$ git fetch origin {branch_name}:{branch_name}"));
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

    let dest = worktree_base_dir(repo_root).join(format!("pr-{pr_number}"));
    let dest_str = dest.to_string_lossy().to_string();

    messages.push(format!("$ git worktree add {dest_str} {branch_name}"));

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, &branch_name])
        .current_dir(repo_root)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        messages.push(format!("✓ PR #{pr_number} checked out at {dest_str}"));
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
            } else {
                let stderr_lower = stderr.to_lowercase();
                if stderr_lower.contains("uncommitted changes")
                    || stderr_lower.contains("local changes")
                    || stderr_lower.contains("not possible to fast-forward")
                    || stderr_lower.contains("you have unstaged changes")
                {
                    SyncStatus::Skipped("dirty working tree".to_string())
                } else {
                    SyncStatus::Error(stderr)
                }
            }
        }
    };

    (
        fetch_ok,
        SyncResult {
            branch: wt.branch.clone(),
            status,
        },
    )
}

/// Derive a default destination path from clone input.
/// e.g. `git@github.com:org/repo.git` or `org/repo` → `<cwd>/repo`
pub fn dest_from_url(source: &str, cwd: &Path) -> String {
    let name = source
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git")
        .to_string();
    cwd.join(name).to_string_lossy().into_owned()
}

/// Clone a repo and place the default branch checkout under `<dest>/<branch>`.
/// Returns the path to the checked-out default branch directory.
pub fn clone_repo_with_layout(url: &str, dest: &Path) -> Result<PathBuf> {
    let source = url.trim();
    fs::create_dir_all(dest).context("Failed to create destination directory")?;
    let tmp_dir = dest.join(format!(
        ".wt-clone-tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    let tmp_str = tmp_dir.to_string_lossy().to_string();

    if is_github_owner_repo(source) {
        let mut cloned = false;

        if gh_available() {
            let gh_clone = Command::new("gh")
                .args(["repo", "clone", source, &tmp_str])
                .output()
                .context("Failed to run gh repo clone")?;

            if gh_clone.status.success() {
                cloned = true;
            }
        }

        if !cloned {
            let protocol = preferred_github_protocol();
            let repo_url = github_url_from_slug(source, &protocol);
            clone_with_git(&repo_url, &tmp_str)?;
        }
    } else {
        clone_with_git(source, &tmp_str)?;
    }

    // Detect the default branch from the cloned checkout.
    let head = Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(&tmp_dir)
        .output()
        .ok();

    let default_branch = head
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_start_matches("refs/heads/")
                .to_string()
        })
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "main".to_string());

    let worktree_path = dest.join(default_branch.replace('/', "-"));
    if worktree_path.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
        anyhow::bail!(
            "Destination already exists: {}",
            worktree_path.to_string_lossy()
        );
    }
    fs::rename(&tmp_dir, &worktree_path).context("Failed to finalize cloned repository layout")?;

    Ok(worktree_path)
}

fn clone_with_git(source: &str, dest: &str) -> Result<()> {
    let clone = Command::new("git")
        .args(["clone", source, dest])
        .output()
        .context("Failed to run git clone")?;

    if clone.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&clone.stderr);
        Err(anyhow::anyhow!("{}", stderr.trim()))
    }
}

fn worktree_base_dir(repo_root: &Path) -> PathBuf {
    // Non-bare repositories use <repo_root>/.git as common dir.
    // In that layout we want branch folders beside repo_root (e.g. repo/main, repo/feature).
    if repo_root.join(".git").exists() {
        repo_root
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| repo_root.to_path_buf())
    } else {
        // Bare repositories already use repo_root as the common directory.
        repo_root.to_path_buf()
    }
}

fn is_github_owner_repo(input: &str) -> bool {
    let value = input.trim();
    if value.is_empty()
        || value.contains("://")
        || value.contains('@')
        || value.contains(' ')
        || value.starts_with('/')
    {
        return false;
    }

    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");

    !owner.is_empty() && !repo.is_empty() && parts.next().is_none()
}

fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn preferred_github_protocol() -> String {
    let out = Command::new("gh")
        .args(["config", "get", "git_protocol"])
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let protocol = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
            if protocol == "https" {
                "https".to_string()
            } else {
                "ssh".to_string()
            }
        }
        _ => "ssh".to_string(),
    }
}

fn github_url_from_slug(slug: &str, protocol: &str) -> String {
    let normalized = slug.trim().trim_end_matches(".git");
    if protocol == "https" {
        format!("https://github.com/{normalized}.git")
    } else {
        format!("git@github.com:{normalized}.git")
    }
}

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

    let git_common_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();

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
