use std::fs;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::types::Worktree;

const REPO_CONFIG_FILE: &str = "worktree-navigator.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RepoConfig {
    #[serde(default)]
    pub post_create_scripts: Vec<PostCreateScript>,
    #[serde(default)]
    pub copy_secrets_from_default_branch: bool,
    #[serde(default)]
    pub post_delete_scripts: Vec<PostCreateScript>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostCreateScript {
    pub command: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostCreateRequest {
    pub repo_root: PathBuf,
    pub worktree_path: PathBuf,
    pub branch: String,
    pub base_branch: Option<String>,
    pub scripts: Vec<PostCreateScript>,
}

const fn default_true() -> bool {
    true
}

impl RepoConfig {
    pub fn enabled_post_create_scripts(&self) -> Vec<PostCreateScript> {
        self.post_create_scripts
            .iter()
            .filter(|script| script.enabled && !script.command.trim().is_empty())
            .cloned()
            .collect()
    }

    pub fn enabled_post_delete_scripts(&self) -> Vec<PostCreateScript> {
        self.post_delete_scripts
            .iter()
            .filter(|script| script.enabled && !script.command.trim().is_empty())
            .cloned()
            .collect()
    }
}

pub fn load_repo_config(repo_root: &Path) -> Result<RepoConfig> {
    let path = repo_config_path(repo_root)?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(RepoConfig::default()),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to read {}", path.display()));
        }
    };

    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse repository config at {}", path.display()))
}

pub fn save_repo_config(repo_root: &Path, config: &RepoConfig) -> Result<()> {
    let path = repo_config_path(repo_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let data = serde_json::to_string_pretty(config).context("Failed to serialize repo config")?;
    fs::write(&path, data).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Setup steps write to stderr instead of stdout: the shell wrapper reads the worktree path to
/// `cd` into from `wt`'s stdout, so anything else on that stream breaks navigation.
fn stderr_as_stdout() -> Result<Stdio> {
    let stderr = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .context("Failed to redirect setup output to stderr")?;
    Ok(Stdio::from(stderr))
}

pub fn run_post_create_scripts(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: Option<&str>,
    scripts: &[PostCreateScript],
) -> Result<()> {
    run_scripts(
        repo_root,
        worktree_path,
        worktree_path,
        branch,
        base_branch,
        "setup",
        scripts,
    )
}

pub fn copy_default_secrets(repo_root: &Path, worktree_path: &Path) -> Result<usize> {
    let Some(default_worktree) = list_repo_worktrees(repo_root)?
        .into_iter()
        .find(|worktree| worktree.is_main)
    else {
        anyhow::bail!("Could not find the default worktree to copy secrets from.");
    };

    let destination = Worktree {
        path: worktree_path.to_string_lossy().into_owned(),
        branch: String::new(),
        is_main: false,
        is_current: false,
        has_secrets: false,
    };
    if !git::worktree_has_secrets(Path::new(&default_worktree.path))
        || git::worktree_has_secrets(worktree_path)
    {
        return Ok(0);
    }

    git::copy_secret_files(&default_worktree, &destination, false)
}

fn list_repo_worktrees(repo_root: &Path) -> Result<Vec<Worktree>> {
    git::list_any_worktrees(repo_root, git::is_managed_workspace(repo_root))
}

pub fn run_post_delete_scripts(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: &str,
    scripts: &[PostCreateScript],
) -> Result<()> {
    if scripts
        .iter()
        .all(|script| !script.enabled || script.command.trim().is_empty())
    {
        return Ok(());
    }

    let default_worktree_path = list_repo_worktrees(repo_root)?
        .into_iter()
        .find(|worktree| worktree.is_main)
        .map_or_else(
            || repo_root.to_path_buf(),
            |worktree| PathBuf::from(worktree.path),
        );

    run_scripts(
        repo_root,
        &default_worktree_path,
        worktree_path,
        branch,
        Some(base_branch),
        "post-delete",
        scripts,
    )
}

fn run_scripts(
    repo_root: &Path,
    cwd: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: Option<&str>,
    action: &str,
    scripts: &[PostCreateScript],
) -> Result<()> {
    let enabled_scripts: Vec<&PostCreateScript> = scripts
        .iter()
        .filter(|script| script.enabled && !script.command.trim().is_empty())
        .collect();
    if enabled_scripts.is_empty() {
        return Ok(());
    }

    let default_worktree_path = list_repo_worktrees(repo_root)?
        .into_iter()
        .find(|wt| wt.is_main)
        .map(|wt| wt.path)
        .unwrap_or_default();

    for (index, script) in enabled_scripts.iter().enumerate() {
        eprintln!();
        eprintln!(
            "[wt] Running {action} step {}/{}",
            index + 1,
            enabled_scripts.len()
        );
        eprintln!("[wt] $ {}", script.command);

        let status = Command::new("sh")
            .args(["-lc", &script.command])
            .current_dir(cwd)
            .stdout(stderr_as_stdout()?)
            .env("WT_REPO_ROOT", repo_root)
            .env("WT_WORKTREE_PATH", worktree_path)
            .env("WT_WORKTREE_BRANCH", branch)
            .env("WT_WORKTREE_BASE_BRANCH", base_branch.unwrap_or(""))
            .env("WT_DEFAULT_WORKTREE_PATH", &default_worktree_path)
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("Failed to run {action} command: {}", script.command))?;

        if !status.success() {
            anyhow::bail!("{action} command failed: {}", script.command);
        }
    }

    Ok(())
}

pub fn write_post_create_request(request: &PostCreateRequest) -> Result<PathBuf> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before UNIX_EPOCH")?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "wt-post-create-{}-{unique}.json",
        std::process::id()
    ));
    let data =
        serde_json::to_string(request).context("Failed to serialize post-create setup request")?;
    fs::write(&path, data).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

pub fn run_post_create_scripts_from_request(request_path: &Path) -> Result<()> {
    let raw = fs::read_to_string(request_path)
        .with_context(|| format!("Failed to read {}", request_path.display()))?;
    let request: PostCreateRequest = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse {}", request_path.display()))?;
    let _ = fs::remove_file(request_path);

    let script_count = request
        .scripts
        .iter()
        .filter(|script| script.enabled && !script.command.trim().is_empty())
        .count();

    eprintln!(
        "[wt] Running {} post-create setup step(s) for {}",
        script_count, request.branch
    );

    run_post_create_scripts(
        &request.repo_root,
        &request.worktree_path,
        &request.branch,
        request.base_branch.as_deref(),
        &request.scripts,
    )
}

pub fn repo_config_path(repo_root: &Path) -> Result<PathBuf> {
    Ok(git::git_common_dir(repo_root)?.join(REPO_CONFIG_FILE))
}

#[cfg(test)]
mod tests {
    use super::{
        PostCreateRequest, PostCreateScript, RepoConfig, copy_default_secrets, load_repo_config,
        repo_config_path, run_post_create_scripts, run_post_create_scripts_from_request,
        run_post_delete_scripts, save_repo_config, write_post_create_request,
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
        let dir = std::env::temp_dir().join(format!("wt-config-{name}-{unique}"));
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
    fn missing_repo_config_defaults_to_empty() {
        let workspace = make_temp_dir("defaults");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);

        let config = load_repo_config(&repo).expect("missing config should load");

        assert!(config.post_create_scripts.is_empty());
        assert!(!config.copy_secrets_from_default_branch);
        assert!(config.post_delete_scripts.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn repo_config_round_trips() {
        let workspace = make_temp_dir("roundtrip");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);

        let config = RepoConfig {
            post_create_scripts: vec![
                PostCreateScript {
                    command: "pnpm i".to_string(),
                    enabled: true,
                },
                PostCreateScript {
                    command: "git submodule update --init --recursive".to_string(),
                    enabled: false,
                },
            ],
            copy_secrets_from_default_branch: true,
            post_delete_scripts: vec![PostCreateScript {
                command: "cleanup".to_string(),
                enabled: false,
            }],
        };

        save_repo_config(&repo, &config).expect("config should save");
        let loaded = load_repo_config(&repo).expect("config should load");

        assert_eq!(loaded, config);
        assert!(repo_config_path(&repo).expect("config path").exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn post_create_scripts_receive_worktree_context() {
        let workspace = make_temp_dir("scripts");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);
        let worktree = workspace.join("feature-test");
        fs::create_dir_all(&worktree).expect("worktree dir should be created");

        run_post_create_scripts(
            &repo,
            &worktree,
            "feature/test",
            Some("main"),
            &[PostCreateScript {
                command: "printf '%s|%s|%s' \"$WT_WORKTREE_BRANCH\" \"$WT_WORKTREE_BASE_BRANCH\" \"$WT_REPO_ROOT\" > hook.out".to_string(),
                enabled: true,
            }],
        )
        .expect("post-create script should run");

        assert_eq!(
            fs::read_to_string(worktree.join("hook.out")).expect("hook output should exist"),
            format!("feature/test|main|{}", repo.display())
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn default_secrets_copy_is_optional_and_skips_existing_secrets() {
        let workspace = make_temp_dir("copy-secrets");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);
        let worktree = workspace.join("feature");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                worktree.to_str().unwrap(),
            ],
        );

        assert_eq!(
            copy_default_secrets(&repo, &worktree).expect("empty default secrets should be fine"),
            0
        );
        fs::write(repo.join(".env.local"), "TOKEN=source\n").expect("secret should be written");
        assert_eq!(
            copy_default_secrets(&repo, &worktree).expect("secrets should copy"),
            1
        );
        assert_eq!(
            fs::read_to_string(worktree.join(".env.local")).expect("secret should be copied"),
            "TOKEN=source\n"
        );

        fs::write(repo.join(".env.private"), "OTHER=source\n").expect("secret should be written");
        assert_eq!(
            copy_default_secrets(&repo, &worktree)
                .expect("existing destination secrets should be left alone"),
            0
        );
        assert!(!worktree.join(".env.private").exists());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn workspace_root_copies_secrets_and_sets_default_worktree_environment() {
        let workspace = make_temp_dir("workspace-copy-secrets");
        fs::write(workspace.join(".wt-workspace"), "").expect("workspace marker should exist");
        let default_worktree = workspace.join("a-main");
        fs::create_dir_all(&default_worktree).expect("default worktree dir should exist");
        init_repo(&default_worktree);
        fs::write(default_worktree.join(".env.local"), "TOKEN=source\n")
            .expect("secret should be written");

        let feature_worktree = workspace.join("z-feature");
        git(
            &default_worktree,
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                feature_worktree.to_str().unwrap(),
            ],
        );

        assert_eq!(
            copy_default_secrets(&workspace, &feature_worktree)
                .expect("workspace secrets should copy"),
            1
        );
        assert_eq!(
            fs::read_to_string(feature_worktree.join(".env.local"))
                .expect("secret should be copied"),
            "TOKEN=source\n"
        );

        run_post_create_scripts(
            &workspace,
            &feature_worktree,
            "feature",
            Some("main"),
            &[PostCreateScript {
                command: "printf '%s' \"$WT_DEFAULT_WORKTREE_PATH\" > default-path.txt".into(),
                enabled: true,
            }],
        )
        .expect("workspace post-create script should run");
        assert_eq!(
            fs::read_to_string(feature_worktree.join("default-path.txt"))
                .expect("default path should be exposed"),
            default_worktree.to_string_lossy()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn post_delete_scripts_use_surviving_default_worktree_and_removed_context() {
        let workspace = make_temp_dir("post-delete");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);
        let removed_worktree = workspace.join("feature");
        let output = workspace.join("post-delete.txt");
        let command = format!(
            "printf '%s|%s|%s|%s' \"$WT_WORKTREE_BRANCH\" \"$WT_WORKTREE_PATH\" \"$WT_WORKTREE_BASE_BRANCH\" \"$PWD\" > '{}'",
            output.display()
        );

        run_post_delete_scripts(
            &repo,
            &removed_worktree,
            "feature/gone",
            "main",
            &[PostCreateScript {
                command,
                enabled: true,
            }],
        )
        .expect("post-delete script should run");

        assert_eq!(
            fs::read_to_string(output).expect("script output should exist"),
            format!(
                "feature/gone|{}|main|{}",
                removed_worktree.display(),
                repo.display()
            )
        );

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn post_create_request_round_trips_and_runs() {
        let workspace = make_temp_dir("request");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);
        let worktree = workspace.join("feature-test");
        fs::create_dir_all(&worktree).expect("worktree dir should be created");

        let request_path = write_post_create_request(&PostCreateRequest {
            repo_root: repo,
            worktree_path: worktree.clone(),
            branch: "feature/test".to_string(),
            base_branch: Some("main".to_string()),
            scripts: vec![PostCreateScript {
                command:
                    "printf '%s|%s' \"$WT_WORKTREE_PATH\" \"$WT_DEFAULT_WORKTREE_PATH\" > hook.out"
                        .to_string(),
                enabled: true,
            }],
        })
        .expect("request file should be written");

        run_post_create_scripts_from_request(&request_path).expect("request should run");

        assert!(
            !request_path.exists(),
            "request file should be removed after running"
        );
        assert!(
            fs::read_to_string(worktree.join("hook.out"))
                .expect("hook output should exist")
                .starts_with(&format!("{}|", worktree.display()))
        );

        let _ = fs::remove_dir_all(workspace);
    }
}
