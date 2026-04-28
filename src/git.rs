use anyhow::{Context, Result};
use std::io::Read;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{CloneEvent, SyncPrEvent, SyncResult, SyncStatus, Worktree};

const MAX_WORKTREE_SCAN_DEPTH: usize = 3;

fn worktree_path_for_name(base_dir: &Path, name: &str) -> PathBuf {
    base_dir.join(Path::new(name))
}

fn ensure_parent_dirs(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;
    }
    Ok(())
}

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

fn parse_worktree_porcelain(
    raw: &str,
    cwd: &Path,
    default_branch: Option<&str>,
) -> Result<Vec<Worktree>> {
    let mut worktrees = Vec::new();

    for block in raw.trim().split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let path_line = lines.iter().find(|l| l.starts_with("worktree "));
        let branch_line = lines.iter().find(|l| l.starts_with("branch "));
        let head_line = lines.iter().find(|l| l.starts_with("HEAD "));
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
            .or_else(|| {
                head_line.map(|l| l.trim_start_matches("HEAD ").to_string())
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

pub fn add_worktree(
    repo_root: &Path,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<(Vec<String>, PathBuf)> {
    let mut messages = Vec::new();

    let dest = worktree_path_for_name(&worktree_base_dir(repo_root), branch_name);
    ensure_parent_dirs(&dest)?;
    let dest_str = dest.to_string_lossy().to_string();
    let git_cwd = resolve_git_cwd(repo_root);

    let mut args = vec!["worktree", "add", &dest_str, "-b", branch_name];
    if let Some(base) = base_branch {
        args.push(base);
        messages.push(format!("$ git worktree add {dest_str} -b {branch_name} {base}"));
    } else {
        messages.push(format!("$ git worktree add {dest_str} -b {branch_name}"));
    }

    let output = Command::new("git")
        .args(&args)
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

#[allow(dead_code)]
pub fn checkout_pr_as_worktree(repo_root: &Path, pr_number: u32) -> Result<(Vec<String>, PathBuf)> {
    checkout_pr_as_worktree_impl(repo_root, pr_number, None)
}

pub fn start_checkout_pr_as_worktree(repo_root: PathBuf, pr_number: u32) -> Receiver<SyncPrEvent> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        match checkout_pr_as_worktree_impl(&repo_root, pr_number, Some(&tx)) {
            Ok((_, worktree_path)) => {
                let _ = tx.send(SyncPrEvent::Finished(worktree_path));
            }
            Err(err) => {
                let _ = tx.send(SyncPrEvent::Error(err.to_string()));
            }
        }
    });

    rx
}

fn checkout_pr_as_worktree_impl(
    repo_root: &Path,
    pr_number: u32,
    tx: Option<&Sender<SyncPrEvent>>,
) -> Result<(Vec<String>, PathBuf)> {
    let mut messages = Vec::new();
    let git_cwd = resolve_git_cwd(repo_root);

    let pr_ref = format!("#{pr_number}");
    push_sync_pr_progress(tx, &mut messages, format!("$ gh pr view {pr_ref}"));
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
    push_sync_pr_progress(
        tx,
        &mut messages,
        format!("✓ Resolved head branch: {branch_name}"),
    );

    push_sync_pr_progress(
        tx,
        &mut messages,
        format!("$ git fetch origin {branch_name}:{branch_name}"),
    );
    let fetch = Command::new("git")
        .args(["fetch", "origin", &format!("{branch_name}:{branch_name}")])
        .current_dir(&git_cwd)
        .output();

    match fetch {
        Ok(out) if !out.status.success() => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            push_sync_pr_progress(
                tx,
                &mut messages,
                format!("  (fetch note: {})", stderr.trim()),
            );
        }
        Err(e) => push_sync_pr_progress(tx, &mut messages, format!("  (fetch warn: {e})")),
        _ => {}
    }

    let dest = worktree_path_for_name(&worktree_base_dir(repo_root), &branch_name);
    let dest_str = dest.to_string_lossy().to_string();
    ensure_parent_dirs(&dest)?;

    push_sync_pr_progress(
        tx,
        &mut messages,
        format!("$ git worktree add {dest_str} {branch_name}"),
    );

    let output = Command::new("git")
        .args(["worktree", "add", &dest_str, &branch_name])
        .current_dir(&git_cwd)
        .output()
        .context("Failed to run git worktree add")?;

    if output.status.success() {
        push_sync_pr_progress(
            tx,
            &mut messages,
            format!("✓ PR #{pr_number} checked out at {dest_str}"),
        );
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        push_sync_pr_progress(tx, &mut messages, format!("✗ {}", stderr.trim()));
    }

    Ok((messages, dest))
}

fn push_sync_pr_progress(
    tx: Option<&Sender<SyncPrEvent>>,
    messages: &mut Vec<String>,
    line: String,
) {
    messages.push(line.clone());
    if let Some(tx) = tx {
        let _ = tx.send(SyncPrEvent::Progress { line });
    }
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

pub fn start_sync_one_worktree(repo_root: PathBuf, wt: Worktree) -> Receiver<(bool, SyncResult)> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = sync_one_worktree(&repo_root, &wt);
        let _ = tx.send(result);
    });

    rx
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

    thread::spawn(move || match clone_repo_with_layout(&url, &dest, &tx) {
        Ok(worktree_path) => {
            let _ = tx.send(CloneEvent::Finished(worktree_path));
        }
        Err(err) => {
            let _ = tx.send(CloneEvent::Error(err.to_string()));
        }
    });

    rx
}

fn clone_repo_with_layout(url: &str, dest: &Path, tx: &Sender<CloneEvent>) -> Result<PathBuf> {
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
            match clone_with_gh(source, &tmp_str, tx) {
                Ok(()) => {}
                Err(_) => {
                    if tmp_dir.exists() {
                        let _ = fs::remove_dir_all(&tmp_dir);
                    }
                    let protocol = preferred_github_protocol();
                    let repo_url = github_url_from_slug(source, &protocol);
                    clone_with_git(&repo_url, &tmp_str, tx)?;
                }
            }
        } else {
            let protocol = preferred_github_protocol();
            let repo_url = github_url_from_slug(source, &protocol);
            clone_with_git(&repo_url, &tmp_str, tx)?;
        }
    } else {
        clone_with_git(source, &tmp_str, tx)?;
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

    let worktree_path = worktree_path_for_name(dest, &default_branch);
    if worktree_path.exists() {
        let _ = fs::remove_dir_all(&tmp_dir);
        anyhow::bail!(
            "Destination already exists: {}",
            worktree_path.to_string_lossy()
        );
    }
    ensure_parent_dirs(&worktree_path)?;
    fs::rename(&tmp_dir, &worktree_path).context("Failed to finalize cloned repository layout")?;
    fs::write(dest.join(".wt-workspace"), "").context("Failed to create .wt-workspace")?;

    Ok(worktree_path)
}

fn clone_with_gh(source: &str, dest: &str, tx: &Sender<CloneEvent>) -> Result<()> {
    clone_with_command(
        Command::new("gh"),
        &["repo", "clone", source, dest, "--", "--progress"],
        "Failed to run gh repo clone",
        "gh repo clone failed",
        tx,
    )
}

fn clone_with_git(source: &str, dest: &str, tx: &Sender<CloneEvent>) -> Result<()> {
    clone_with_command(
        Command::new("git"),
        &["clone", "--progress", source, dest],
        "Failed to run git clone",
        "git clone failed",
        tx,
    )
}

fn clone_with_command(
    mut command: Command,
    args: &[&str],
    spawn_context: &'static str,
    fallback_error: &'static str,
    tx: &Sender<CloneEvent>,
) -> Result<()> {
    let mut child = command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(spawn_context)?;

    let stdout = child
        .stdout
        .take()
        .context("Failed to capture clone stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("Failed to capture clone stderr")?;

    let last_line = Arc::new(Mutex::new(None::<String>));
    let stdout_handle = spawn_output_forwarder(stdout, tx.clone(), Arc::clone(&last_line));
    let stderr_handle = spawn_output_forwarder(stderr, tx.clone(), Arc::clone(&last_line));

    let status = child.wait().context("Failed to wait for clone process")?;

    let _ = stdout_handle.join();
    let _ = stderr_handle.join();

    if status.success() {
        Ok(())
    } else {
        let message = last_line
            .lock()
            .ok()
            .and_then(|line| line.clone())
            .filter(|line| !line.is_empty())
            .unwrap_or_else(|| fallback_error.to_string());
        Err(anyhow::anyhow!("{message}"))
    }
}

fn spawn_output_forwarder(
    mut stream: impl Read + Send + 'static,
    tx: Sender<CloneEvent>,
    last_line: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut current = Vec::new();
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(read) => {
                    for &byte in &buf[..read] {
                        match byte {
                            b'\r' => {
                                emit_clone_output(&tx, &last_line, std::mem::take(&mut current));
                            }
                            b'\n' => {
                                emit_clone_output(&tx, &last_line, std::mem::take(&mut current));
                            }
                            _ => current.push(byte),
                        }
                    }
                }
                Err(_) => break,
            }
        }

        emit_clone_output(&tx, &last_line, current);
    })
}

fn emit_clone_output(
    tx: &Sender<CloneEvent>,
    last_line: &Arc<Mutex<Option<String>>>,
    bytes: Vec<u8>,
) {
    let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
    if line.is_empty() {
        return;
    }

    if let Ok(mut last) = last_line.lock() {
        *last = Some(line.clone());
    }

    let _ = tx.send(CloneEvent::Progress { line });
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
/// the first valid nested git-repo path so git commands have a working context
/// while `worktree_base_dir` still uses `repo_root` to place new trees.
fn resolve_git_cwd(repo_root: &Path) -> PathBuf {
    if is_git_repo(repo_root) {
        return repo_root.to_path_buf();
    }

    if let Ok(repos) = collect_workspace_git_repos(repo_root) {
        if let Some(path) = repos.into_iter().next() {
            return path;
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

fn collect_workspace_git_repos(root: &Path) -> Result<Vec<PathBuf>> {
    let mut repos = Vec::new();
    collect_workspace_git_repos_recursive(root, MAX_WORKTREE_SCAN_DEPTH, &mut repos)?;
    repos.sort();
    Ok(repos)
}

fn collect_workspace_git_repos_recursive(
    dir: &Path,
    remaining_depth: usize,
    repos: &mut Vec<PathBuf>,
) -> Result<()> {
    if is_git_repo(dir) {
        repos.push(dir.to_path_buf());
        return Ok(());
    }

    if remaining_depth == 0 {
        return Ok(());
    }

    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if should_skip_dir(&path) {
            continue;
        }
        collect_workspace_git_repos_recursive(&path, remaining_depth - 1, repos)?;
    }

    Ok(())
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

fn read_git_origin_from_config(git_config: &Path) -> Option<String> {
    let content = fs::read_to_string(git_config).ok()?;
    let mut in_origin = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_origin = trimmed == r#"[remote "origin"]"#;
            continue;
        }
        if in_origin {
            if let Some(rest) = trimmed.strip_prefix("url") {
                if let Some(url) = rest.trim_start().strip_prefix('=') {
                    let url = url.trim().trim_end_matches(".git").to_lowercase();
                    if !url.is_empty() {
                        return Some(url);
                    }
                }
            }
        }
    }
    None
}

pub fn detect_worktree_workspace(dir: &Path) -> bool {
    const MAX_SCAN: usize = 50;

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut origin: Option<String> = None;
    let mut main_count = 0usize;
    let mut linked_count = 0usize;

    for entry in entries.flatten().take(MAX_SCAN) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let git_path = path.join(".git");
        if git_path.is_dir() {
            let Some(url) = read_git_origin_from_config(&git_path.join("config")) else {
                continue;
            };
            match &origin {
                None => origin = Some(url),
                Some(existing) if existing == &url => {}
                Some(_) => return false,
            }
            main_count += 1;
        } else if git_path.is_file() {
            linked_count += 1;
        }
    }

    (linked_count > 0 && main_count > 0) || main_count >= 2
}

/// Recursively scan `workspace_dir` up to 3 directory levels and return nested git repos as worktrees.
pub fn list_workspace_worktrees(workspace_dir: &Path) -> Result<Vec<Worktree>> {
    let cwd = std::env::var("WT_CWD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let cwd = cwd.canonicalize().unwrap_or(cwd);

    let repos = collect_workspace_git_repos(workspace_dir)?;

    // Resolve default branch from the first valid git repo path.
    let default_branch = repos.first().and_then(|path| get_default_branch(path));
    let mut worktrees = Vec::new();

    for path in repos {
        let branch_output = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&path)
            .output();

        let branch = match branch_output {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            _ => {
                let head_output = Command::new("git")
                    .args(["rev-parse", "HEAD"])
                    .current_dir(&path)
                    .output();
                let Ok(head_out) = head_output else {
                    continue;
                };
                if !head_out.status.success() {
                    continue;
                }
                String::from_utf8_lossy(&head_out.stdout).trim().to_string()
            }
        };

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

pub fn list_remotes(repo_root: &Path) -> Result<Vec<String>> {
    let git_cwd = resolve_git_cwd(repo_root);
    let out = Command::new("git")
        .args(["remote"])
        .current_dir(&git_cwd)
        .output()
        .context("Failed to run git remote")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

pub fn start_fetch_remote(repo_root: PathBuf, remote: String) -> Receiver<Result<(), String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let git_cwd = resolve_git_cwd(&repo_root);
        let out = Command::new("git")
            .args(["fetch", &remote])
            .current_dir(&git_cwd)
            .output();
        let result = match out {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                Err(if stderr.is_empty() {
                    format!("git fetch {remote} failed")
                } else {
                    stderr
                })
            }
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    });
    rx
}

pub fn list_remote_branches(repo_root: &Path, remote: &str) -> Vec<String> {
    let git_cwd = resolve_git_cwd(repo_root);
    let prefix = format!("{remote}/");
    let pattern = format!("{remote}/*");
    let out = Command::new("git")
        .args(["branch", "-r", "--list", &pattern])
        .current_dir(&git_cwd)
        .output()
        .unwrap_or_else(|_| std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: vec![],
            stderr: vec![],
        });
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with(&prefix) && !l.contains("HEAD"))
        .map(|l| l[prefix.len()..].to_string())
        .collect()
}

pub fn checkout_remote_branch(repo_root: &Path, remote: &str, branch: &str) -> Result<PathBuf> {
    let git_cwd = resolve_git_cwd(repo_root);

    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let ref_exists = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(&git_cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ref_exists {
        anyhow::bail!("{remote}/{branch} not found after fetch");
    }

    let dest = worktree_path_for_name(&worktree_base_dir(repo_root), branch);
    ensure_parent_dirs(&dest)?;
    let dest_str = dest.to_string_lossy().to_string();

    let local_exists = Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(&git_cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let output = if local_exists {
        let upstream = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", &format!("{branch}@{{upstream}}")])
            .current_dir(&git_cwd)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        let expected = format!("{remote}/{branch}");
        if upstream.as_deref() != Some(expected.as_str()) {
            anyhow::bail!(
                "Local branch '{branch}' tracks '{}', not '{expected}'",
                upstream.as_deref().unwrap_or("(none)")
            );
        }

        Command::new("git")
            .args(["worktree", "add", &dest_str, branch])
            .current_dir(&git_cwd)
            .output()
            .context("Failed to run git worktree add")?
    } else {
        Command::new("git")
            .args([
                "worktree",
                "add",
                "--track",
                "-b",
                branch,
                &dest_str,
                &format!("{remote}/{branch}"),
            ])
            .current_dir(&git_cwd)
            .output()
            .context("Failed to run git worktree add")?
    };

    if output.status.success() {
        Ok(dest)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("{stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        add_worktree, detect_worktree_workspace, list_secret_files, list_workspace_worktrees,
        read_git_origin_from_config, resolve_git_cwd,
    };
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

    fn make_git_repo_with_origin(parent: &Path, name: &str, origin: &str) -> PathBuf {
        let repo = parent.join(name);
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init"]);
        git(&repo, &["remote", "add", "origin", origin]);
        repo
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init"]);
        git(dir, &["checkout", "-b", "main"]);
        git(dir, &["config", "user.email", "wt@example.com"]);
        git(dir, &["config", "user.name", "wt"]);
        fs::write(dir.join("README.md"), "hello\n").expect("repo file should be written");
        git(dir, &["add", "README.md"]);
        git(dir, &["commit", "-m", "init"]);
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

    #[test]
    fn read_git_origin_parses_and_normalizes_url() {
        let dir = make_temp_dir("origin-parse");
        let config = dir.join("config");

        fs::write(
            &config,
            "[core]\n\trepositoryformatversion = 0\n[remote \"origin\"]\n\turl = git@github.com:owner/repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        ).unwrap();
        assert_eq!(
            read_git_origin_from_config(&config),
            Some("git@github.com:owner/repo".to_string())
        );

        fs::write(
            &config,
            "[remote \"origin\"]\n\turl = https://github.com/owner/repo\n",
        ).unwrap();
        assert_eq!(
            read_git_origin_from_config(&config),
            Some("https://github.com/owner/repo".to_string())
        );

        fs::write(&config, "[core]\n\tbare = false\n").unwrap();
        assert_eq!(read_git_origin_from_config(&config), None);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_worktree_preserves_nested_branch_paths() {
        let workspace = make_temp_dir("nested-worktree-add");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);

        let branch = "feat/team/branch";
        let expected = workspace.join(branch);
        let (_messages, dest) =
            add_worktree(&repo, branch, None).expect("nested worktree should be created");

        assert_eq!(dest, expected);
        assert!(dest.exists());

        let head = Command::new("git")
            .args(["symbolic-ref", "--short", "HEAD"])
            .current_dir(&dest)
            .output()
            .expect("git should inspect nested worktree");
        assert!(head.status.success());
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), branch);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn lists_nested_workspace_repos_recursively() {
        let workspace = make_temp_dir("nested-workspace");
        fs::write(workspace.join(".wt-workspace"), "").expect("workspace marker should be written");

        let first_repo = workspace.join("feat/team/branch-a");
        let second_repo = workspace.join("fix/branch-b");
        fs::create_dir_all(&first_repo).expect("first repo dir should be created");
        fs::create_dir_all(&second_repo).expect("second repo dir should be created");
        init_repo(&first_repo);
        init_repo(&second_repo);

        let ignored_repo = workspace.join("skip/too/deep/branch-c/more");
        fs::create_dir_all(&ignored_repo).expect("deep repo dir should be created");
        init_repo(&ignored_repo);

        let worktrees = list_workspace_worktrees(&workspace).expect("workspace scan should succeed");
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, first_repo.to_string_lossy().to_string());
        assert_eq!(worktrees[0].branch, "main");
        assert_eq!(worktrees[1].path, second_repo.to_string_lossy().to_string());
        assert_eq!(worktrees[1].branch, "main");
        assert_eq!(resolve_git_cwd(&workspace), first_repo);
        assert!(!worktrees
            .iter()
            .any(|wt| wt.path == ignored_repo.to_string_lossy().to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn detect_returns_false_for_empty_dir() {
        let dir = make_temp_dir("detect-empty");
        assert!(!detect_worktree_workspace(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detect_returns_false_for_single_main_worktree() {
        let dir = make_temp_dir("detect-single");
        make_git_repo_with_origin(&dir, "main", "git@github.com:owner/repo.git");
        assert!(!detect_worktree_workspace(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detect_returns_true_for_two_clones_same_origin() {
        let dir = make_temp_dir("detect-two-clones");
        make_git_repo_with_origin(&dir, "main", "git@github.com:owner/repo.git");
        make_git_repo_with_origin(&dir, "feature", "git@github.com:owner/repo.git");
        assert!(detect_worktree_workspace(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detect_returns_false_for_two_clones_different_origins() {
        let dir = make_temp_dir("detect-diff-origins");
        make_git_repo_with_origin(&dir, "repo1", "git@github.com:owner/repo-a.git");
        make_git_repo_with_origin(&dir, "repo2", "git@github.com:owner/repo-b.git");
        assert!(!detect_worktree_workspace(&dir));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detect_returns_true_for_linked_worktree_alongside_main() {
        let dir = make_temp_dir("detect-linked");
        make_git_repo_with_origin(&dir, "main", "git@github.com:owner/repo.git");
        let linked = dir.join("feature");
        fs::create_dir_all(&linked).unwrap();
        fs::write(
            linked.join(".git"),
            "gitdir: ../main/.git/worktrees/feature\n",
        )
        .unwrap();
        assert!(detect_worktree_workspace(&dir));
        let _ = fs::remove_dir_all(dir);
    }
}
