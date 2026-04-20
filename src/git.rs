use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{CloneEvent, SyncResult, SyncStatus, Worktree};

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
    let default_branch = get_default_branch(repo_root);

    parse_worktree_porcelain(&stdout, &cwd, default_branch.as_deref())
}

fn get_default_branch(git_repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(git_repo)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let branch = raw.trim().strip_prefix("refs/remotes/origin/")?;
    if branch.is_empty() {
        return None;
    }
    Some(branch.to_string())
}

fn parse_worktree_porcelain(raw: &str, cwd: &Path, default_branch: Option<&str>) -> Result<Vec<Worktree>> {
    let mut worktrees = Vec::new();

    for block in raw.trim().split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let path_line = lines.iter().find(|l| l.starts_with("worktree "));
        let branch_line = lines.iter().find(|l| l.starts_with("branch "));
        let is_bare = lines.iter().any(|l| *l == "bare");

        let Some(path_str) = path_line.map(|l| l.trim_start_matches("worktree ")) else {
            continue;
        };
        if is_bare {
            continue;
        }

        let path = PathBuf::from(path_str);
        let branch = branch_line
            .map(|l| {
                l.trim_start_matches("branch ")
                    .trim_start_matches("refs/heads/")
                    .to_string()
            })
            .unwrap_or_else(|| "HEAD".to_string());

        let is_main = match default_branch {
            Some(db) => branch == db,
            None => worktrees.is_empty(),
        };
        let is_current = path.canonicalize().unwrap_or(path.clone()) == cwd;

        worktrees.push(Worktree {
            path: path_str.to_string(),
            branch,
            is_main,
            is_current,
            has_secrets: worktree_has_secrets(&path),
        });
    }

    worktrees.sort_by_key(|w| !w.is_main);
    Ok(worktrees)
}

pub fn add_worktree(repo_root: &Path, branch_name: &str) -> Result<(Vec<String>, PathBuf)> {
    let mut messages = Vec::new();

    let sanitized = branch_name.replace('/', "-");
    let dest = worktree_base_dir(repo_root).join(&sanitized);
    let dest_str = dest.to_string_lossy().to_string();
    let git_cwd = resolve_git_cwd(repo_root);

    messages.push(format!("$ git worktree add {dest_str} -b {branch_name}"));

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, "-b", branch_name])
        .current_dir(&git_cwd)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        messages.push(format!("✓ Created worktree at {dest_str}"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        messages.push(format!("✗ {}", stderr.trim()));
    }

    Ok((messages, dest))
}

pub fn remove_worktree(repo_root: &Path, worktree_path: &str) -> Result<Vec<String>> {
    let mut messages = Vec::new();
    let git_cwd = resolve_git_cwd(repo_root);
    messages.push(format!("$ git worktree remove --force {worktree_path}"));

    let output = Command::new("git")
        .args(["worktree", "remove", "--force", worktree_path])
        .current_dir(&git_cwd)
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

pub fn checkout_pr_as_worktree(repo_root: &Path, pr_number: u32) -> Result<(Vec<String>, PathBuf)> {
    let mut messages = Vec::new();
    let git_cwd = resolve_git_cwd(repo_root);

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
        .current_dir(&git_cwd)
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
        .current_dir(&git_cwd)
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

    let dest = worktree_base_dir(repo_root).join(branch_name.replace('/', "-"));
    let dest_str = dest.to_string_lossy().to_string();

    messages.push(format!("$ git worktree add {dest_str} {branch_name}"));

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, &branch_name])
        .current_dir(&git_cwd)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        messages.push(format!("✓ PR #{pr_number} checked out at {dest_str}"));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        messages.push(format!("✗ {}", stderr.trim()));
    }

    Ok((messages, dest))
}

/// Fetch from all remotes then fast-forward a single worktree to origin/<branch>.
/// Returns (fetch_succeeded, SyncResult).
pub fn sync_one_worktree(repo_root: &Path, wt: &Worktree) -> (bool, SyncResult) {
    let git_cwd = resolve_git_cwd(repo_root);
    let fetch_ok = Command::new("git")
        .args(["fetch", "--all", "--quiet"])
        .current_dir(&git_cwd)
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

pub fn copy_secret_files(from: &Worktree, to: &Worktree, overwrite: bool) -> Result<usize> {
    let from_path = Path::new(&from.path);
    let to_path = Path::new(&to.path);

    let source_files = list_secret_files(from_path)?;
    if source_files.is_empty() {
        anyhow::bail!("This worktree doesn't contain secrets");
    }

    if !overwrite && worktree_has_secrets(to_path) {
        anyhow::bail!("Destination worktree already contains secrets");
    }

    let mut copied = 0;
    for rel in source_files {
        let src = from_path.join(&rel);
        let dest = to_path.join(&rel);

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create destination directory {}",
                    parent.display()
                )
            })?;
        }

        fs::copy(&src, &dest).with_context(|| {
            format!(
                "Failed to copy secret {} to {}",
                src.display(),
                dest.display()
            )
        })?;
        copied += 1;
    }

    Ok(copied)
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

pub fn start_clone_repo_with_layout(url: String, dest: PathBuf) -> Receiver<CloneEvent> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || match clone_repo_with_layout(&url, &dest) {
        Ok(worktree_path) => {
            let _ = tx.send(CloneEvent::Finished(worktree_path));
        }
        Err(err) => {
            let _ = tx.send(CloneEvent::Error(err.to_string()));
        }
    });

    rx
}

fn clone_repo_with_layout(url: &str, dest: &Path) -> Result<PathBuf> {
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
        if gh_available() {
            match clone_with_gh(source, &tmp_str) {
                Ok(()) => {}
                Err(_) => {
                    if tmp_dir.exists() {
                        let _ = fs::remove_dir_all(&tmp_dir);
                    }
                    let protocol = preferred_github_protocol();
                    let repo_url = github_url_from_slug(source, &protocol);
                    clone_with_git(&repo_url, &tmp_str)?;
                }
            }
        } else {
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
    fs::write(dest.join(".wt-workspace"), "").context("Failed to create .wt-workspace")?;

    Ok(worktree_path)
}

fn clone_with_gh(source: &str, dest: &str) -> Result<()> {
    clone_with_command(
        Command::new("gh"),
        &["repo", "clone", source, dest],
        "Failed to run gh repo clone",
        "gh repo clone failed",
    )
}

fn clone_with_git(source: &str, dest: &str) -> Result<()> {
    clone_with_command(
        Command::new("git"),
        &["clone", "--progress", source, dest],
        "Failed to run git clone",
        "git clone failed",
    )
}

fn clone_with_command(
    mut command: Command,
    args: &[&str],
    spawn_context: &'static str,
    fallback_error: &'static str,
) -> Result<()> {
    let output = command.args(args).output().context(spawn_context)?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let message = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            fallback_error.to_string()
        };
        Err(anyhow::anyhow!("{message}"))
    }
}

fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns a path suitable as `current_dir` for git commands.
///
/// In workspace mode `repo_root` is the parent directory holding individual
/// worktrees as subdirectories — it is not itself a git repo. We fall back to
/// the first valid git-repo subdirectory so git commands have a working context
/// while `worktree_base_dir` still uses `repo_root` to place new trees.
fn resolve_git_cwd(repo_root: &Path) -> PathBuf {
    if is_git_repo(repo_root) {
        return repo_root.to_path_buf();
    }

    if let Ok(entries) = fs::read_dir(repo_root) {
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let path = entry.path();
            if is_git_repo(&path) {
                return path;
            }
        }
    }

    repo_root.to_path_buf()
}

fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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

pub fn create_workspace_marker(dir: &Path) -> Result<()> {
    let marker = dir.join(".wt-workspace");
    fs::write(&marker, "").context("Failed to create .wt-workspace")?;
    Ok(())
}

/// Walk up from `start` looking for a `.wt-workspace` marker file.
pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".wt-workspace").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Scan immediate subdirectories of `workspace_dir` for git repos and return
/// them as `Worktree` entries. Each valid git subdir is treated as one worktree.
pub fn list_workspace_worktrees(workspace_dir: &Path) -> Result<Vec<Worktree>> {
    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let cwd = cwd.canonicalize().unwrap_or(cwd);

    let mut worktrees = Vec::new();

    let mut entries: Vec<_> = fs::read_dir(workspace_dir)
        .context("Failed to read workspace directory")?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();

    entries.sort_by_key(|e| e.file_name());

    // Resolve default branch from the first valid git repo subdir
    let first_git_dir = entries.iter().find(|e| e.path().join(".git").exists()).map(|e| e.path());
    let default_branch = first_git_dir.as_deref().and_then(get_default_branch);

    for entry in entries {
        let path = entry.path();

        let branch_output = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&path)
            .output();

        let Ok(branch_out) = branch_output else {
            continue;
        };
        if !branch_out.status.success() {
            continue;
        }

        let branch = String::from_utf8_lossy(&branch_out.stdout)
            .trim()
            .to_string();

        let path_str = path.to_string_lossy().to_string();
        let is_main = match default_branch.as_deref() {
            Some(db) => branch == db,
            None => worktrees.is_empty(),
        };
        let is_current = path.canonicalize().unwrap_or(path.clone()) == cwd;

        worktrees.push(Worktree {
            path: path_str,
            branch,
            is_main,
            is_current,
            has_secrets: worktree_has_secrets(&path),
        });
    }

    worktrees.sort_by_key(|w| !w.is_main);
    Ok(worktrees)
}

pub fn worktree_has_secrets(path: &Path) -> bool {
    list_secret_files(path)
        .map(|files| !files.is_empty())
        .unwrap_or(false)
}

fn list_secret_files(root: &Path) -> Result<Vec<PathBuf>> {
    let tracked = tracked_files(root)?;
    let mut found = Vec::new();
    collect_secret_files(root, root, &tracked, &mut found)?;
    found.sort();
    Ok(found)
}

fn tracked_files(root: &Path) -> Result<HashSet<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .context("Failed to list tracked files")?;

    if !output.status.success() {
        return Ok(HashSet::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn collect_secret_files(
    root: &Path,
    dir: &Path,
    tracked: &HashSet<PathBuf>,
    found: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = entry
            .file_type()
            .with_context(|| format!("Failed to inspect {}", path.display()))?;

        if metadata.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_secret_files(root, &path, tracked, found)?;
            continue;
        }

        if !metadata.is_file() && !metadata.is_symlink() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(".env") {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .with_context(|| format!("Failed to relativize {}", path.display()))?
            .to_path_buf();
        if tracked.contains(&rel) {
            continue;
        }

        found.push(rel);
    }

    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            ".git"
                | "node_modules"
                | ".next"
                | ".nuxt"
                | ".turbo"
                | ".cache"
                | "dist"
                | "build"
                | "target"
                | "coverage"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::{list_secret_files, looks_like_clone_progress, parse_clone_progress};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("wt-{name}-{unique}"));
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git command should run");
        assert!(
            status.success(),
            "git {:?} failed in {}",
            args,
            dir.display()
        );
    }

    #[test]
    fn parses_receiving_objects_progress() {
        let progress =
            parse_clone_progress("Receiving objects:  42% (42/100), 1.23 MiB | 1.23 MiB/s")
                .expect("expected progress line to parse");

        assert_eq!(progress.phase, "Receiving objects");
        assert_eq!(
            progress.detail.as_deref(),
            Some("Receiving objects:  42% (42/100), 1.23 MiB | 1.23 MiB/s")
        );
        assert!((progress.ratio - 0.473).abs() < 0.001);
    }

    #[test]
    fn parses_remote_counting_progress() {
        let progress = parse_clone_progress("remote: Counting objects: 100% (24/24), done.")
            .expect("expected progress line to parse");

        assert_eq!(progress.phase, "Counting objects");
        assert_eq!(
            progress.detail.as_deref(),
            Some("Counting objects: 100% (24/24), done.")
        );
        assert!((progress.ratio - 0.10).abs() < 0.001);
    }

    #[test]
    fn leaves_error_output_unclassified() {
        let line = "fatal: repository 'git@github.com:owner/missing.git' not found";
        assert!(parse_clone_progress(line).is_none());
        assert!(!looks_like_clone_progress(line));
    }

    #[test]
    fn parses_gh_clone_prelude() {
        let progress = parse_clone_progress("Cloning into 'tea-website'...")
            .expect("expected progress line to parse");

        assert_eq!(progress.phase, "Starting clone");
        assert_eq!(
            progress.detail.as_deref(),
            Some("Cloning into 'tea-website'...")
        );
        assert!((progress.ratio - 0.02).abs() < 0.001);
    }

    #[test]
    fn lists_only_untracked_env_files_recursively() {
        let dir = make_temp_dir("secret-scan");
        git(&dir, &["init"]);
        fs::write(dir.join(".env"), "SECRET=1\n").expect("secret file should be written");
        fs::write(dir.join(".env.default"), "tracked=true\n")
            .expect("tracked env template should be written");
        fs::create_dir_all(dir.join("apps/web")).expect("nested dir should exist");
        fs::write(dir.join("apps/web/.env.local"), "WEB_SECRET=1\n")
            .expect("nested secret should be written");
        fs::create_dir_all(dir.join("node_modules/pkg")).expect("ignored dir should exist");
        fs::write(dir.join("node_modules/pkg/.env"), "IGNORE=1\n")
            .expect("ignored secret should be written");
        git(&dir, &["add", ".env.default"]);

        let files = list_secret_files(&dir).expect("secret scan should succeed");

        assert_eq!(
            files,
            vec![PathBuf::from(".env"), PathBuf::from("apps/web/.env.local")]
        );

        let _ = fs::remove_dir_all(dir);
    }
}
