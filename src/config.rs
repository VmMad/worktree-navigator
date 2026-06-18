use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::git;

const REPO_CONFIG_FILE: &str = "worktree-navigator.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RepoConfig {
    #[serde(default)]
    pub post_create_scripts: Vec<PostCreateScript>,
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

fn default_true() -> bool {
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

pub fn run_post_create_scripts(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: Option<&str>,
    scripts: &[PostCreateScript],
) -> Result<()> {
    let enabled_scripts: Vec<&PostCreateScript> = scripts
        .iter()
        .filter(|script| script.enabled && !script.command.trim().is_empty())
        .collect();
    if enabled_scripts.is_empty() {
        return Ok(());
    }

    let default_worktree_path = git::list_worktrees(repo_root)?
        .into_iter()
        .find(|wt| wt.is_main)
        .map(|wt| wt.path)
        .unwrap_or_default();

    for (index, script) in enabled_scripts.iter().enumerate() {
        eprintln!();
        eprintln!(
            "[wt] Running setup step {}/{}",
            index + 1,
            enabled_scripts.len()
        );
        eprintln!("[wt] $ {}", script.command);

        let status = Command::new("sh")
            .args(["-lc", &script.command])
            .current_dir(worktree_path)
            .env("WT_REPO_ROOT", repo_root)
            .env("WT_WORKTREE_PATH", worktree_path)
            .env("WT_WORKTREE_BRANCH", branch)
            .env("WT_WORKTREE_BASE_BRANCH", base_branch.unwrap_or(""))
            .env("WT_DEFAULT_WORKTREE_PATH", &default_worktree_path)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("Failed to run setup command: {}", script.command))?;

        if !status.success() {
            anyhow::bail!("Setup command failed: {}", script.command);
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
        PostCreateRequest, PostCreateScript, RepoConfig, load_repo_config, repo_config_path,
        run_post_create_scripts, run_post_create_scripts_from_request, save_repo_config,
        write_post_create_request,
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
    fn post_create_request_round_trips_and_runs() {
        let workspace = make_temp_dir("request");
        let repo = workspace.join("repo");
        fs::create_dir_all(&repo).expect("repo dir should be created");
        init_repo(&repo);
        let worktree = workspace.join("feature-test");
        fs::create_dir_all(&worktree).expect("worktree dir should be created");

        let request_path = write_post_create_request(&PostCreateRequest {
            repo_root: repo.clone(),
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
