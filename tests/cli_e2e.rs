use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const fn binary_path() -> &'static str {
    env!("CARGO_BIN_EXE_worktree-navigator")
}

struct TestEnv {
    root: PathBuf,
    origin: PathBuf,
    repo: PathBuf,
    home: PathBuf,
    bin_dir: PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let root = make_temp_dir(name);
        let home = root.join("home");
        let bin_dir = root.join("bin");
        let origin = root.join("origin.git");
        let repo = root.join("main");

        fs::create_dir_all(&home).expect("home dir should be created");
        fs::create_dir_all(&bin_dir).expect("bin dir should be created");

        git(
            &root,
            &["init", "--bare", origin.to_string_lossy().as_ref()],
        );
        git(
            &root,
            &[
                "clone",
                origin.to_string_lossy().as_ref(),
                repo.to_string_lossy().as_ref(),
            ],
        );

        git(&repo, &["config", "user.email", "wt@example.com"]);
        git(&repo, &["config", "user.name", "wt"]);
        git(&repo, &["checkout", "-b", "main"]);
        fs::write(repo.join("README.md"), "hello\n").expect("repo file should be written");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "init"]);
        git(&repo, &["push", "-u", "origin", "main"]);
        git(&origin, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(&repo, &["fetch", "origin"]);
        git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        Self {
            root,
            origin,
            repo,
            home,
            bin_dir,
        }
    }

    fn path_for_branch(&self, branch: &str) -> PathBuf {
        self.root.join(branch.replace('/', "-"))
    }

    fn create_branch(&self, branch: &str) {
        git(&self.repo, &["checkout", "-b", branch]);
        let file_name = format!("{}.txt", branch.replace('/', "-"));
        fs::write(self.repo.join(file_name), format!("{branch}\n"))
            .expect("branch file should be written");
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-m", branch]);
        git(&self.repo, &["push", "-u", "origin", branch]);
        git(&self.repo, &["checkout", "main"]);
    }

    fn create_existing_worktree(&self, branch: &str) -> PathBuf {
        let path = self.path_for_branch(branch);
        git(
            &self.repo,
            &["worktree", "add", path.to_string_lossy().as_ref(), branch],
        );
        path
    }

    fn create_worktree_from_base(&self, branch: &str, base: &str) -> PathBuf {
        let path = self.path_for_branch(branch);
        git(
            &self.repo,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                path.to_string_lossy().as_ref(),
                base,
            ],
        );
        path
    }

    fn commit_in_worktree(worktree: &Path, message: &str) {
        let file_name = format!("{}.txt", message.replace('/', "-"));
        fs::write(worktree.join(file_name), format!("{message}\n"))
            .expect("worktree file should be written");
        git(worktree, &["add", "."]);
        git(worktree, &["commit", "-m", message]);
    }

    fn set_fake_gh(&self, script: &str) {
        let gh_path = self.bin_dir.join("gh");
        fs::write(&gh_path, script).expect("fake gh should be written");
        fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755))
            .expect("fake gh should be executable");
    }

    fn path_env(&self) -> OsString {
        let mut value = OsString::new();
        value.push(&self.bin_dir);
        value.push(":");
        value.push(std::env::var_os("PATH").unwrap_or_default());
        value
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn make_temp_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("wt-cli-e2e-{name}-{unique}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "safe.bareRepository=all"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "safe.bareRepository=all"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {:?} failed in {}\nstdout:\n{}\nstderr:\n{}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_wt(env: &TestEnv, cwd: &Path, args: &[&str]) -> Output {
    Command::new(binary_path())
        .args(args)
        .current_dir(cwd)
        .env("HOME", &env.home)
        .output()
        .expect("wt should run")
}

fn run_wt_wrapped(env: &TestEnv, cwd: &Path, args: &[&str]) -> Output {
    Command::new(binary_path())
        .args(args)
        .current_dir(cwd)
        .env("HOME", &env.home)
        .env("WT_CWD", cwd)
        .output()
        .expect("wt should run")
}

fn run_wt_with_path(env: &TestEnv, cwd: &Path, args: &[&str]) -> Output {
    Command::new(binary_path())
        .args(args)
        .current_dir(cwd)
        .env("HOME", &env.home)
        .env("WT_CWD", cwd)
        .env("PATH", env.path_env())
        .output()
        .expect("wt should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[test]
fn help_and_version_work_end_to_end() {
    let env = TestEnv::new("help-version");

    let help = run_wt(&env, &env.repo, &["--help"]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("Available commands:"));
    assert!(stdout(&help).contains("clone <repo> [dest]"));
    assert!(stdout(&help).contains("gco [branch]"));

    let help_alias = run_wt_wrapped(&env, &env.repo, &["help"]);
    assert!(help_alias.status.success());
    assert!(stdout(&help_alias).is_empty());
    assert!(stderr(&help_alias).contains("Available commands:"));

    let version = run_wt_wrapped(&env, &env.repo, &["--version"]);
    assert!(version.status.success());
    assert!(stdout(&version).is_empty());
    assert!(stderr(&version).starts_with("wt v"));
}

#[test]
fn clone_command_clones_repo_into_default_destination() {
    let env = TestEnv::new("clone-default-dest");
    let source = env.origin.to_string_lossy().into_owned();

    let output = run_wt_wrapped(&env, &env.root, &["clone", source.as_str()]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));

    let workspace = env.root.join("origin");
    let worktree = workspace.join("main");
    assert_eq!(stdout(&output), worktree.to_string_lossy());
    assert!(worktree.exists());
    assert!(workspace.join(".wt-workspace").exists());
}

#[test]
fn clone_command_honors_explicit_destination() {
    let env = TestEnv::new("clone-explicit-dest");
    let source = env.origin.to_string_lossy().into_owned();

    let output = run_wt_wrapped(&env, &env.root, &["clone", source.as_str(), "custom-clone"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));

    let workspace = env.root.join("custom-clone");
    let worktree = workspace.join("main");
    assert_eq!(stdout(&output), worktree.to_string_lossy());
    assert!(worktree.exists());
    assert!(workspace.join(".wt-workspace").exists());
}

#[test]
fn clone_command_accepts_github_slug_sources() {
    let env = TestEnv::new("clone-gh-slug");
    let source = env.origin.to_string_lossy().into_owned();
    env.set_fake_gh(&format!(
        "#!/usr/bin/env bash\nset -e\nif [[ \"$1\" == \"--version\" ]]; then\n  printf 'gh version 99.0.0\\n'\n  exit 0\nfi\nif [[ \"$1\" == \"repo\" && \"$2\" == \"clone\" && \"$3\" == \"owner/repo\" ]]; then\n  git clone --progress \"{source}\" \"$4\"\n  exit 0\nfi\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n"
    ));

    let output = run_wt_with_path(&env, &env.root, &["clone", "owner/repo"]);
    assert!(output.status.success(), "stderr:\n{}", stderr(&output));

    let workspace = env.root.join("repo");
    let worktree = workspace.join("main");
    assert_eq!(stdout(&output), worktree.to_string_lossy());
    assert!(worktree.exists());
    assert!(workspace.join(".wt-workspace").exists());
}

#[test]
fn checkout_commands_jump_to_existing_worktrees() {
    let env = TestEnv::new("checkout-existing");
    env.create_branch("feature/existing");
    let existing = env.create_existing_worktree("feature/existing");

    for args in [
        &["gco", "feature/existing"][..],
        &["checkout", "feature/existing"][..],
    ] {
        let output = run_wt_wrapped(&env, &env.repo, args);
        assert!(output.status.success(), "stderr:\n{}", stderr(&output));
        assert_eq!(stdout(&output), existing.to_string_lossy());
    }
}

#[test]
fn checkout_commands_default_to_the_default_branch_worktree() {
    let env = TestEnv::new("checkout-default");
    let default_path = env.repo.clone();

    for args in [&["gco"][..], &["checkout"][..]] {
        let output = run_wt_wrapped(&env, &env.repo, args);
        assert!(output.status.success(), "stderr:\n{}", stderr(&output));
        assert_eq!(stdout(&output), default_path.to_string_lossy());
    }
}

#[test]
fn branch_commands_create_worktrees_from_expected_bases() {
    let env = TestEnv::new("branch-create");
    let current_worktree = env.create_worktree_from_base("feature/current", "main");
    TestEnv::commit_in_worktree(&current_worktree, "feature/current");

    let main_head = git_stdout(&env.repo, &["rev-parse", "main"]);
    let current_head = git_stdout(&current_worktree, &["rev-parse", "HEAD"]);

    let from_default = run_wt_wrapped(
        &env,
        &current_worktree,
        &["b", "feature/from-default", "-d"],
    );
    assert!(
        from_default.status.success(),
        "stderr:\n{}",
        stderr(&from_default)
    );
    let from_default_path = PathBuf::from(stdout(&from_default));
    assert_eq!(
        git_stdout(&from_default_path, &["rev-parse", "HEAD"]),
        main_head
    );

    let from_current = run_wt_wrapped(&env, &current_worktree, &["branch", "feature/from-current"]);
    assert!(
        from_current.status.success(),
        "stderr:\n{}",
        stderr(&from_current)
    );
    let from_current_path = PathBuf::from(stdout(&from_current));
    assert_eq!(
        git_stdout(&from_current_path, &["rev-parse", "HEAD"]),
        current_head
    );
}

#[test]
fn delete_commands_remove_worktrees_and_redirect_when_deleting_current() {
    let env = TestEnv::new("delete-worktree");

    let named_branch = "feature/delete-named";
    env.create_branch(named_branch);
    let named_path = env.create_existing_worktree(named_branch);
    let named_delete = run_wt_wrapped(&env, &env.repo, &["delete", named_branch, "--yes"]);
    assert!(
        named_delete.status.success(),
        "stderr:\n{}",
        stderr(&named_delete)
    );
    assert!(!named_path.exists());
    assert!(stdout(&named_delete).is_empty());

    let current_branch = "feature/delete-current";
    env.create_branch(current_branch);
    let current_path = env.create_existing_worktree(current_branch);
    let current_delete = run_wt_wrapped(&env, &current_path, &["d", "--yes"]);
    assert!(
        current_delete.status.success(),
        "stderr:\n{}",
        stderr(&current_delete)
    );
    assert!(!current_path.exists());
    assert_eq!(stdout(&current_delete), env.repo.to_string_lossy());
}

#[test]
fn pr_commands_create_or_reuse_pr_worktrees() {
    let env = TestEnv::new("pr-command");
    let pr_branch = "pr/123";
    env.create_branch(pr_branch);
    env.set_fake_gh(
        "#!/usr/bin/env bash\nset -e\nif [[ \"$1\" == \"pr\" && \"$2\" == \"view\" ]]; then\n  printf 'pr/123\\n'\n  exit 0\nfi\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n",
    );

    let first = run_wt_with_path(&env, &env.repo, &["pr", "123"]);
    assert!(first.status.success(), "stderr:\n{}", stderr(&first));
    let pr_path = PathBuf::from(stdout(&first));
    assert!(pr_path.exists());
    assert_eq!(
        git_stdout(&pr_path, &["symbolic-ref", "--short", "HEAD"]),
        pr_branch
    );

    let second = run_wt_with_path(&env, &env.repo, &["checkout-pr", "123"]);
    assert!(second.status.success(), "stderr:\n{}", stderr(&second));
    assert_eq!(stdout(&second), pr_path.to_string_lossy());
}

#[test]
fn update_command_reports_missing_compatible_asset() {
    let env = TestEnv::new("update-command");
    env.set_fake_gh(
        "#!/usr/bin/env bash\nset -e\nif [[ \"$1\" == \"api\" ]]; then\n  printf '{\"tag_name\":\"v9.9.9\",\"assets\":[]}'\n  exit 0\nfi\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n",
    );

    let output = Command::new(binary_path())
        .arg("--update")
        .current_dir(&env.repo)
        .env("HOME", &env.home)
        .env("PATH", env.path_env())
        .output()
        .expect("wt update should run");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("No compatible release asset found for this wt binary."),
        "stderr:\n{}",
        stderr(&output)
    );
}

#[test]
fn update_command_refreshes_zsh_wrapper_and_prints_restart_message() {
    let env = TestEnv::new("update-zsh-wrapper");
    let update_target = env.root.join("wt-updated");
    env.set_fake_gh(
        "#!/usr/bin/env bash\nset -e\nif [[ \"$1\" == \"api\" ]]; then\n  printf '{\"tag_name\":\"v9.9.9\",\"assets\":[{\"name\":\"worktree-navigator-x86_64-linux-gnu\"}]}'\n  exit 0\nfi\nif [[ \"$1\" == \"release\" && \"$2\" == \"download\" ]]; then\n  out=\"\"\n  while [[ $# -gt 0 ]]; do\n    if [[ \"$1\" == \"--output\" ]]; then\n      out=\"$2\"\n      shift 2\n      continue\n    fi\n    shift\n  done\n  printf '#!/usr/bin/env bash\\nexit 0\\n' > \"$out\"\n  chmod +x \"$out\"\n  exit 0\nfi\necho \"unexpected gh invocation: $*\" >&2\nexit 1\n",
    );

    let output = Command::new(binary_path())
        .arg("--update")
        .current_dir(&env.repo)
        .env("HOME", &env.home)
        .env("PATH", env.path_env())
        .env("SHELL", "/bin/zsh")
        .env("WT_UPDATE_TARGET", &update_target)
        .output()
        .expect("wt update should run");

    assert!(output.status.success(), "stderr:\n{}", stderr(&output));
    assert!(update_target.exists());
    assert!(stdout(&output).is_empty());

    let zshrc = fs::read_to_string(env.home.join(".zshrc")).expect("zshrc should exist");
    assert!(zshrc.contains("# worktree-navigator wt()"));
    assert!(zshrc.contains("WT_SHELL_WRAPPER=1 command wt \"$@\""));
    assert!(zshrc.contains("WT_POST_CREATE="));

    let stderr = stderr(&output);
    assert!(stderr.contains("Updated wt to v9.9.9"), "stderr:\n{stderr}");
    assert!(
        stderr.contains("Restart your console to reload the wt shell wrapper."),
        "stderr:\n{stderr}"
    );
}

#[test]
fn mark_tree_creates_workspace_marker() {
    if Command::new("script").arg("-V").output().is_err() {
        return;
    }

    let env = TestEnv::new("mark-tree");
    let workspace = env.root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir should be created");

    let mut child = Command::new("script")
        .args([
            "-q",
            "-c",
            &format!(
                "env WT_CWD={} {} --mark-tree",
                workspace.display(),
                binary_path()
            ),
            "/dev/null",
        ])
        .current_dir(&workspace)
        .env("HOME", &env.home)
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("script should run wt in a pseudo tty");

    let mut stdin = child.stdin.take().expect("script stdin should be piped");
    stdin.write_all(b"q").expect("quit key should be sent");
    drop(stdin);

    let status = child.wait().expect("mark-tree command should exit");
    assert!(status.success());
    assert!(workspace.join(".wt-workspace").exists());
}
